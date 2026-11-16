use std::time::Duration;

use axum::Json;
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::AppState;
use crate::api::billing::{AdvanceRun, advance_subscription_periods};
use crate::api::{get_str, ok};
use crate::db;
use crate::error::ApiError;
use crate::form;
use crate::models::{
    AccountRow, DeliveryRow, EventDestinationRow, MeterEventRow, PaymentIntentRow, SubscriptionRow,
};
use crate::seed;
use crate::time;
use crate::util::list_envelope;

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

/// `GET /internal/meter_events`: inspect accepted raw meter events without
/// Stripe authentication. Optional `event_name` and `customer` filters narrow
/// the v1 list envelope the endpoint returns.
pub async fn list_meter_events(
    State(st): State<AppState>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let event_name = get_str(&params, &["event_name"]);
    let customer = get_str(&params, &["customer"]);
    let events: Vec<MeterEventRow> =
        sqlx::query_as("SELECT * FROM meter_events ORDER BY timestamp, rowid")
            .fetch_all(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let data: Vec<Value> = events
        .into_iter()
        .filter(|event| {
            event_name
                .as_deref()
                .is_none_or(|name| event.event_name == name)
        })
        .filter(|event| {
            customer
                .as_deref()
                .is_none_or(|id| event.customer.as_deref() == Some(id))
        })
        .map(|event| {
            json!({
                "identifier": event.identifier,
                "event_name": event.event_name,
                "customer": event.customer,
                "value": crate::models::usage_number(event.value).to_string(),
                "timestamp": event.timestamp,
            })
        })
        .collect();
    Ok(ok(list_envelope("/internal/meter_events", data, false)))
}

/// `POST /internal/subscriptions/{id}/advance`: end the current billing period
/// and collect the renewal invoice, or retry an open invoice after a failure.
///
/// `outcome` is `paid` or `failed`. When an open invoice already exists the
/// call retries it without moving the period, so a test can model failure and
/// then recovery. Otherwise the period advances, a `subscription_cycle`
/// invoice is raised, and the subscription becomes `active` or `past_due`.
/// A subscription with `cancel_at_period_end=true` is canceled at the boundary
/// instead, emits `customer.subscription.deleted`, and reports a null invoice.
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
    // A clock subscription runs on simulated time, so its retry and renewal
    // stamps follow the clock rather than wall-clock time.
    let now = match &subscription.test_clock {
        Some(clock_id) => {
            let frozen: Option<i64> = sqlx::query_scalar(
                "SELECT frozen_time FROM test_clocks WHERE id = ?1 AND account_id = ?2",
            )
            .bind(clock_id)
            .bind(&account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
            frozen.unwrap_or_else(time::now_epoch)
        }
        None => time::now_epoch(),
    };

    let mut event_ids = Vec::new();
    let invoice = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        let invoice = advance_subscription_periods(
            &mut tx,
            &subscription,
            &account_id,
            is_connected,
            AdvanceRun::OnePeriod,
            paid,
            now,
            &st.config.base_url(),
            &mut event_ids,
        )
        .await?;
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
        "invoice": invoice.map(|invoice| invoice.to_json(&st.config.base_url())).unwrap_or(Value::Null),
    })))
}
