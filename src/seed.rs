use serde_json::json;
use sqlx::SqlitePool;

use crate::config::Config;
use crate::ids;
use crate::mock::STATIC_CHECKOUT_ID;
use crate::time;

/// Amount (minor units) and currency of the static demo checkout line item.
const STATIC_CHECKOUT_AMOUNT: i64 = 2000;
const STATIC_CHECKOUT_CURRENCY: &str = "usd";

/// Seed the main account and the configured event destinations. Idempotent: it
/// only creates rows that do not exist, re-seeds config-declared destinations by
/// name, and normalizes webhook secrets so each endpoint URL has exactly one.
pub async fn seed(pool: &SqlitePool, config: &Config) -> Result<(), sqlx::Error> {
    let main = seed_main_account(pool, config).await?;
    seed_static_checkout_session(pool, &main, &config.base_url()).await?;
    seed_event_destinations(pool, config, &main).await?;
    normalize_destination_secrets(pool).await?;
    Ok(())
}

/// Upsert the well-known static demo checkout session (`cs_test_static`) into a
/// fresh `open` state owned by `account_id`. Idempotent: re-running leaves
/// exactly one row, so it serves both boot-time seeding and re-opening the
/// session after it was completed or expired.
pub async fn seed_static_checkout_session(
    pool: &SqlitePool,
    account_id: &str,
    base_url: &str,
) -> Result<(), sqlx::Error> {
    let now = time::now_epoch();
    let expires_at = now + 365 * 86_400;
    let line_items = json!([{
        "id": "li_static_demo",
        "object": "item",
        "amount_total": STATIC_CHECKOUT_AMOUNT,
        "currency": STATIC_CHECKOUT_CURRENCY,
        "description": "Static demo item",
        "price": {
            "id": "price_static_demo",
            "object": "price",
            "active": true,
            "billing_scheme": "per_unit",
            "created": now,
            "currency": STATIC_CHECKOUT_CURRENCY,
            "livemode": false,
            "metadata": {},
            "nickname": null,
            "product": "prod_static_demo",
            "recurring": null,
            "type": "one_time",
            "unit_amount": STATIC_CHECKOUT_AMOUNT,
            "unit_amount_decimal": STATIC_CHECKOUT_AMOUNT.to_string(),
        },
        "quantity": 1,
    }]);
    let url = format!(
        "{}/mock/checkout/{}",
        base_url.trim_end_matches('/'),
        STATIC_CHECKOUT_ID
    );
    sqlx::query(
        "INSERT INTO checkout_sessions (id, account_id, mode, status, created, expires_at,
            success_url, cancel_url, customer, customer_account, line_items, payment_intent,
            subscription, amount_total, currency, url, metadata)
         VALUES (?1, ?2, 'payment', 'open', ?3, ?4, NULL, NULL, NULL, NULL, ?5, NULL, NULL, ?6, ?7, ?8, '{}')
         ON CONFLICT(id) DO UPDATE SET
            account_id = excluded.account_id,
            mode = 'payment',
            status = 'open',
            created = excluded.created,
            expires_at = excluded.expires_at,
            success_url = NULL,
            cancel_url = NULL,
            customer = NULL,
            customer_account = NULL,
            line_items = excluded.line_items,
            payment_intent = NULL,
            subscription = NULL,
            amount_total = excluded.amount_total,
            currency = excluded.currency,
            url = excluded.url,
            metadata = '{}'",
    )
    .bind(STATIC_CHECKOUT_ID)
    .bind(account_id)
    .bind(now)
    .bind(expires_at)
    .bind(line_items.to_string())
    .bind(STATIC_CHECKOUT_AMOUNT)
    .bind(STATIC_CHECKOUT_CURRENCY)
    .bind(&url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Enforce one signing secret per endpoint URL across every destination, including
/// rows created before this rule or directly through the v2 API. Destinations that
/// share a `url` therefore verify webhooks with the same `whsec_` secret.
pub async fn normalize_destination_secrets(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT id, url FROM event_destinations")
        .fetch_all(pool)
        .await?;
    for (id, url) in rows {
        let secret = ids::webhook_secret_for_url(&url);
        sqlx::query("UPDATE event_destinations SET secret = ?1 WHERE id = ?2 AND secret <> ?1")
            .bind(&secret)
            .bind(&id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn seed_main_account(pool: &SqlitePool, config: &Config) -> Result<String, sqlx::Error> {
    if let Some(id) =
        sqlx::query_scalar::<_, String>("SELECT id FROM accounts WHERE is_main = 1 LIMIT 1")
            .fetch_optional(pool)
            .await?
    {
        ensure_keys(pool, &id).await?;
        return Ok(id);
    }

    let id = ids::account_id();
    let now = time::now_epoch();
    let configuration = json!({
        "merchant": {
            "capabilities": {
                "card_payments": { "requested": true },
                "transfers": { "requested": true }
            }
        }
    });
    let capabilities = json!({ "card_payments": "active", "transfers": "active" });
    let requirements = json!({
        "alternatives": [],
        "current_deadline": null,
        "currently_due": [],
        "disabled_reason": null,
        "eventually_due": [],
        "past_due": [],
        "pending_verification": [],
    });
    let identity = json!({
        "business_details": { "name": config.main_account.name },
    });
    let defaults = json!({ "currency": "usd" });

    sqlx::query(
        "INSERT INTO accounts (id, created, updated, livemode, is_main, business_type, email, country, default_currency,
            identity, configuration, capabilities, requirements, defaults, details_submitted, charges_enabled, payouts_enabled, metadata)
         VALUES (?1, ?2, ?2, 0, 1, ?3, ?4, ?5, 'usd', ?6, ?7, ?8, ?9, ?10, 1, 1, 1, '{}')",
    )
    .bind(&id)
    .bind(now)
    .bind(config.main_account.business_type.clone().unwrap_or_else(|| "company".to_string()))
    .bind(&config.main_account.email)
    .bind(config.main_account.country.clone().unwrap_or_else(|| "US".to_string()))
    .bind(identity.to_string())
    .bind(configuration.to_string())
    .bind(capabilities.to_string())
    .bind(requirements.to_string())
    .bind(defaults.to_string())
    .execute(pool)
    .await?;

    ensure_keys(pool, &id).await?;
    Ok(id)
}

/// Ensure both a secret and a publishable key exist for the account.
pub async fn ensure_keys(pool: &SqlitePool, account_id: &str) -> Result<(), sqlx::Error> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE account_id = ?1")
        .bind(account_id)
        .fetch_one(pool)
        .await?;
    if count > 0 {
        return Ok(());
    }
    let now = time::now_epoch();
    for (key, kind) in [
        (ids::secret_key(), "secret"),
        (ids::publishable_key(), "publishable"),
    ] {
        sqlx::query(
            "INSERT INTO api_keys (key, account_id, kind, created) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(key)
        .bind(account_id)
        .bind(kind)
        .bind(now)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Seed event destinations declared in config, owned by the main account.
/// Idempotent by `(account_id, name, event_payload)`, so a snapshot and a thin
/// destination may share a name: existing destinations are never modified,
/// so API edits and secrets persist across restarts.
pub async fn seed_event_destinations(
    pool: &SqlitePool,
    config: &Config,
    owner: &str,
) -> Result<(), sqlx::Error> {
    let now = time::now_epoch();
    for destination in &config.event_destinations {
        let enabled_events = serde_json::to_string(&destination.enabled_events)
            .unwrap_or_else(|_| "[\"*\"]".to_string());
        let events_from = serde_json::to_string(&destination.events_from)
            .unwrap_or_else(|_| "[\"@self\"]".to_string());
        let status = if destination.enabled {
            "enabled"
        } else {
            "disabled"
        };
        let url = destination
            .webhook_endpoint
            .as_ref()
            .map(|endpoint| endpoint.url.clone())
            .unwrap_or_default();
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM event_destinations WHERE account_id = ?1 AND name = ?2 AND event_payload = ?3 LIMIT 1")
                .bind(owner)
                .bind(&destination.name)
                .bind(&destination.event_payload)
                .fetch_optional(pool)
                .await?;
        let secret = ids::webhook_secret_for_url(&url);
        match existing {
            // Keep destinations that already exist so changes made through the API survive restarts.
            Some(_) => {}
            None => {
                let id = ids::event_destination_id();
                sqlx::query(
                    "INSERT INTO event_destinations (id, account_id, name, description, url, enabled_events, event_payload, events_from, status, metadata, snapshot_api_version, secret, created, updated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '{}', NULL, ?10, ?11, ?11)",
                )
                .bind(&id)
                .bind(owner)
                .bind(&destination.name)
                .bind(&destination.description)
                .bind(&url)
                .bind(enabled_events)
                .bind(&destination.event_payload)
                .bind(events_from)
                .bind(status)
                .bind(secret)
                .bind(now)
                .execute(pool)
                .await?;
            }
        }
    }
    Ok(())
}
