use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::{Value, json};
use sqlx::SqlitePool;

use crate::AppState;
use crate::api::ok;
use crate::db;
use crate::error::ApiError;
use crate::models::{AccountRow, DeliveryRow, EventDestinationRow};
use crate::seed;

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
