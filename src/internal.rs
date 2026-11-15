use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::AppState;
use crate::api::billing::{
    advance_period, open_subscription_invoice, raise_subscription_invoice, subscription_interval,
};
use crate::api::ok;
use crate::db;
use crate::error::ApiError;
use crate::models::{
    AccountRow, DeliveryRow, EventDestinationRow, PaymentIntentRow, SubscriptionRow, parse_json,
};
use crate::seed;
use crate::time;

async fn account_with_keys(pool: &SqlitePool, account: &AccountRow) -> Value {
    let keys: Vec<(String, String)> =
        sqlx::query_as("SELECT key, kind FROM api_keys WHERE account_id = ?1")
            .bind(&account.id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let secret = keys
        .iter()
        .find(|(_, k)| k == "secret")
        .map(|(v, _)| v.clone());
    let publishable = keys
        .iter()
        .find(|(_, k)| k == "publishable")
        .map(|(v, _)| v.clone());
    json!({
        "id": account.id,
        "object": "account",
        "is_main": account.is_main != 0,
        "created": account.created,
        "details_submitted": account.details_submitted != 0,
        "charges_enabled": account.charges_enabled != 0,
        "payouts_enabled": account.payouts_enabled != 0,
        "secret_key": secret,
        "publishable_key": publishable,
    })
}

pub async fn main_account(State(st): State<AppState>) -> Result<Response, ApiError> {
    let account: AccountRow = sqlx::query_as("SELECT * FROM accounts WHERE is_main = 1 LIMIT 1")
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::internal("main account not seeded"))?;
    Ok(ok(account_with_keys(&st.pool, &account).await))
}

pub async fn list_accounts(State(st): State<AppState>) -> Result<Response, ApiError> {
    let accounts: Vec<AccountRow> =
        sqlx::query_as("SELECT * FROM accounts ORDER BY is_main DESC, created DESC, rowid DESC")
            .fetch_all(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let mut data = Vec::new();
    for account in &accounts {
        data.push(account_with_keys(&st.pool, account).await);
    }
    Ok(ok(json!({ "object": "list", "data": data })))
}

/// `POST /internal/accounts`: seed a connected account without Stripe auth.
/// Accepts `name`, optional `email`/`country`, and `state` (`active`/`pending`),
/// and returns the new account's `acct_` id plus freshly minted keys.
pub async fn create_account(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let state = body
        .get("state")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("pending");
    let active = match state {
        "active" => true,
        "pending" => false,
        other => {
            return Err(ApiError::invalid_param(
                crate::error::ApiVersion::V1,
                "state",
                format!("Invalid state: {other}. Expected 'active' or 'pending'."),
            ));
        }
    };
    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Mock Business");
    let email = body
        .get("email")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let country = body
        .get("country")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("US");

    let id = seed::seed_connected_account(&st.pool, name, email, country, active)
        .await
        .map_err(ApiError::internal)?;
    let account: AccountRow = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(account_with_keys(&st.pool, &account).await))
}

pub async fn account_secrets(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let account: Option<AccountRow> = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let account = account
        .ok_or_else(|| ApiError::not_found(crate::error::ApiVersion::V1, "No such account"))?;
    Ok(ok(account_with_keys(&st.pool, &account).await))
}

pub async fn list_destinations(State(st): State<AppState>) -> Result<Response, ApiError> {
    let destinations: Vec<EventDestinationRow> =
        sqlx::query_as("SELECT * FROM event_destinations ORDER BY created, rowid")
            .fetch_all(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let data: Vec<Value> = destinations.iter().map(|d| d.to_internal_json()).collect();
    Ok(ok(json!({ "object": "list", "data": data })))
}

pub async fn list_deliveries(State(st): State<AppState>) -> Result<Response, ApiError> {
    let deliveries: Vec<DeliveryRow> =
        sqlx::query_as("SELECT * FROM deliveries ORDER BY created, rowid")
            .fetch_all(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let data: Vec<Value> = deliveries.iter().map(|d| d.to_json()).collect();
    Ok(ok(json!({ "object": "list", "data": data })))
}

pub async fn retry_delivery(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let existed = st.dispatcher.retry(&id).await.map_err(ApiError::internal)?;
    Ok(ok(json!({ "retried": existed, "id": id })))
}

pub async fn wait_delivery(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let event_id = body
        .get("event_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ApiError::invalid_param(
                crate::error::ApiVersion::V1,
                "event_id",
                "event_id is required",
            )
        })?;
    let timeout = body
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5_000);
    match st
        .dispatcher
        .wait_for_event(event_id, Duration::from_millis(timeout))
        .await
    {
        Ok(rows) => {
            let data: Vec<Value> = rows.iter().map(|d| d.to_json()).collect();
            Ok(ok(
                json!({ "event": event_id, "settled": true, "deliveries": data }),
            ))
        }
        Err(()) => Ok(ok(
            json!({ "event": event_id, "settled": false, "deliveries": [] }),
        )),
    }
}

pub async fn flush_deliveries(
    State(st): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let timeout = body
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(5_000);
    match st.dispatcher.flush(Duration::from_millis(timeout)).await {
        Ok(()) => Ok(ok(json!({ "settled": true }))),
        Err(()) => Ok(ok(json!({ "settled": false }))),
    }
}

pub async fn reset(State(st): State<AppState>) -> Result<Response, ApiError> {
    db::reset_state(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    seed::seed(&st.pool, &st.config)
        .await
        .map_err(ApiError::internal)?;
    st.dispatcher.start().await.map_err(ApiError::internal)?;
    Ok(ok(json!({ "status": "ok" })))
}

/// `POST /internal/payment_intents/{id}/resolve`: settle a payment intent as
/// `succeeded`, `failed`, or `canceled`. The related invoice is updated by the
/// same settlement path the public invoice-payment endpoint uses.
pub async fn resolve_payment_intent(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::missing_param(crate::error::ApiVersion::V1, "status"))?;

    // The payment intent may belong to any account; the settlement uses the
    // intent's own account so the events are scoped like any other.
    let intent: Option<PaymentIntentRow> =
        sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let intent = intent.ok_or_else(|| {
        ApiError::not_found(
            crate::error::ApiVersion::V1,
            format!("No such payment intent: {id}"),
        )
    })?;

    let is_main: i64 = sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
        .bind(&intent.account_id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let is_connected = is_main == 0;

    let updated = crate::api::invoicing::resolve_payment_intent(
        &st,
        &id,
        status,
        &intent.account_id,
        is_connected,
    )
    .await?;

    Ok(ok(json!({
        "id": updated.id,
        "object": "payment_intent",
        "status": updated.status,
        "invoice": updated.invoice,
    })))
}

/// `POST /internal/subscriptions/{id}/advance`: end the current billing period
/// and collect the renewal invoice, or retry an open invoice after a failure.
///
/// `outcome` is `paid` or `failed`. When an open invoice already exists the
/// call retries it without moving the period, so a test can model failure and
/// then recovery. Otherwise the period advances, a `subscription_cycle`
/// invoice is raised, and the subscription becomes `active` or `past_due`.
pub async fn advance_subscription(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let outcome = body
        .get("outcome")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|outcome| !outcome.is_empty())
        .ok_or_else(|| ApiError::missing_param(crate::error::ApiVersion::V1, "outcome"))?;
    if !["paid", "failed"].contains(&outcome) {
        return Err(ApiError::invalid_param(
            crate::error::ApiVersion::V1,
            "outcome",
            "outcome must be paid or failed",
        ));
    }
    let paid = outcome == "paid";

    let subscription: Option<SubscriptionRow> =
        sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let subscription = subscription.ok_or_else(|| {
        ApiError::not_found(
            crate::error::ApiVersion::V1,
            format!("No such subscription: {id}"),
        )
    })?;
    if subscription.status == "canceled" {
        return Err(ApiError::invalid_request(
            crate::error::ApiVersion::V1,
            "A canceled subscription cannot advance",
        ));
    }
    let account_id = subscription.account_id.clone();
    let is_main: i64 = sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
        .bind(&account_id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let is_connected = is_main == 0;
    let now = time::now_epoch();

    let mut event_ids = Vec::new();
    let invoice = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        let open = open_subscription_invoice(&mut tx, &id)
            .await
            .map_err(ApiError::internal)?;

        let invoice = if let Some(invoice) = open {
            // Retry the invoice raised by a previous failed collection.
            let attempt = invoice.attempt_count + 1;
            let status = if paid { "paid" } else { "open" };
            sqlx::query(
                "UPDATE invoices SET status = ?1, amount_paid = ?2, amount_remaining = ?3,
                    paid_at = ?4, attempt_count = ?5, updated = ?6 WHERE id = ?7",
            )
            .bind(status)
            .bind(if paid { invoice.amount_due } else { 0 })
            .bind(if paid { 0 } else { invoice.remaining() })
            .bind(if paid { Some(now) } else { None })
            .bind(attempt)
            .bind(now)
            .bind(&invoice.id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
            let updated: crate::models::InvoiceRow =
                sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
                    .bind(&invoice.id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            let data = updated.to_json(&st.config.base_url());
            if paid {
                event_ids.push(
                    crate::events::emit(
                        &mut tx,
                        &account_id,
                        is_connected,
                        "invoice.payment_succeeded",
                        data.clone(),
                    )
                    .await
                    .map_err(ApiError::internal)?,
                );
                event_ids.push(
                    crate::events::emit(&mut tx, &account_id, is_connected, "invoice.paid", data)
                        .await
                        .map_err(ApiError::internal)?,
                );
            } else {
                event_ids.push(
                    crate::events::emit(
                        &mut tx,
                        &account_id,
                        is_connected,
                        "invoice.payment_failed",
                        data,
                    )
                    .await
                    .map_err(ApiError::internal)?,
                );
            }
            sqlx::query("UPDATE subscriptions SET status = ?1 WHERE id = ?2")
                .bind(if paid { "active" } else { "past_due" })
                .bind(&id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            updated
        } else {
            // End the period and raise its renewal invoice.
            let old_start = subscription.current_period_start.unwrap_or(now);
            let old_end = subscription.current_period_end.unwrap_or(now);
            let period_start = subscription.current_period_end.unwrap_or(now);
            let (interval, interval_count) = subscription_interval(&subscription);
            let period_end = advance_period(period_start, &interval, interval_count);
            let items = parse_json(&Some(subscription.items.clone()));
            let items = match items {
                Value::Array(items) => Value::Array(
                    items
                        .into_iter()
                        .map(|mut item| {
                            if let Value::Object(map) = &mut item {
                                map.insert("current_period_start".into(), json!(period_start));
                                map.insert("current_period_end".into(), json!(period_end));
                            }
                            item
                        })
                        .collect(),
                ),
                other => other,
            };
            sqlx::query(
                "UPDATE subscriptions SET current_period_start = ?1, current_period_end = ?2,
                    status = ?3, items = ?4 WHERE id = ?5",
            )
            .bind(period_start)
            .bind(period_end)
            .bind(if paid { "active" } else { "past_due" })
            .bind(items.to_string())
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
            let updated: SubscriptionRow =
                sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
                    .bind(&id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            let (invoice, invoice_events) = raise_subscription_invoice(
                &mut tx,
                &updated,
                &account_id,
                is_connected,
                period_start,
                period_end,
                old_start,
                old_end,
                "subscription_cycle",
                paid,
                &st.config.base_url(),
            )
            .await?;
            event_ids.extend(invoice_events);
            invoice
        };

        let updated: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        event_ids.push(
            crate::events::emit(
                &mut tx,
                &account_id,
                is_connected,
                "customer.subscription.updated",
                updated.to_json(),
            )
            .await
            .map_err(ApiError::internal)?,
        );
        tx.commit().await.map_err(ApiError::internal)?;
        invoice
    };
    for event_id in event_ids {
        let _ = st.dispatcher.dispatch_event(&event_id).await;
    }

    let subscription: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(json!({
        "outcome": outcome,
        "subscription": subscription.to_json(),
        "invoice": invoice.to_json(&st.config.base_url()),
    })))
}
