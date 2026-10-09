use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Value, json};

use crate::AppState;
use crate::api::{V2Json, collect_includes, get_i64, get_str, ok, paginate};
use crate::auth::{self, AuthContext};
use crate::error::{ApiError, ApiVersion};
use crate::events;
use crate::ids;
use crate::models::{AccountRow, EventDestinationRow, EventRow};
use crate::time;
use crate::util::{empty_object, list_envelope_v2};

fn v2() -> ApiVersion {
    ApiVersion::V2
}

async fn auth(st: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    auth::authenticate(&st.pool, headers, v2()).await
}

fn default_requirements() -> Value {
    json!({
        "alternatives": [],
        "current_deadline": null,
        "currently_due": [],
        "disabled_reason": null,
        "eventually_due": [],
        "past_due": [],
        "pending_verification": [],
    })
}

pub async fn create_account(
    State(st): State<AppState>,
    headers: HeaderMap,
    V2Json(body): V2Json,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let configuration = body
        .get("configuration")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !configuration.is_object()
        || configuration
            .as_object()
            .map(|o| o.is_empty())
            .unwrap_or(true)
    {
        return Err(ApiError::invalid_param(
            v2(),
            "configuration",
            "At least one configuration is required",
        ));
    }
    let identity = body.get("identity").cloned().unwrap_or_else(|| json!({}));
    let defaults = body.get("defaults").cloned().unwrap_or_else(|| json!({}));
    let metadata = body.get("metadata").cloned().unwrap_or_else(|| json!({}));

    // Build the capability status map from requested capabilities.
    let mut capabilities = serde_json::Map::new();
    if let Some(config_obj) = configuration.as_object() {
        for section in config_obj.values() {
            if let Some(merchant_caps) = section.get("capabilities").and_then(|c| c.as_object()) {
                for name in merchant_caps.keys() {
                    capabilities.insert(name.clone(), json!("pending"));
                }
            }
        }
    }

    let id = ids::account_id();
    let now = time::now_epoch();

    // The account row and its `v2.core.account.created` thin event commit
    // together, then deliveries are dispatched after the transaction.
    let mut tx = st.pool.begin().await.map_err(ApiError::internal_v2)?;
    sqlx::query(
        "INSERT INTO accounts (id, created, updated, livemode, is_main, business_type, email, country, default_currency,
            identity, configuration, capabilities, requirements, defaults, details_submitted, charges_enabled, payouts_enabled, metadata)
         VALUES (?1, ?2, ?2, 0, 0, 'company', ?3, 'US', 'usd', ?4, ?5, ?6, ?7, ?8, 0, 0, 0, ?9)",
    )
    .bind(&id)
    .bind(now)
    .bind(get_str(&body, &["identity", "business_details", "email"]))
    .bind(identity.to_string())
    .bind(configuration.to_string())
    .bind(Value::Object(capabilities).to_string())
    .bind(default_requirements().to_string())
    .bind(defaults.to_string())
    .bind(metadata.to_string())
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal_v2)?;

    let account: AccountRow = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal_v2)?;

    let envelope = events::thin_envelope(
        events::account_related_object(&account.id),
        Value::Null,
        Value::Null,
    );
    let event_id = events::emit_thin(
        &mut tx,
        &ctx.key_account_id,
        Some(&account.id),
        "v2.core.account.created",
        envelope,
        None,
    )
    .await
    .map_err(ApiError::internal_v2)?;
    tx.commit().await.map_err(ApiError::internal_v2)?;
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    Ok(ok(account.to_v2(&[])))
}

pub async fn get_account(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    if !ctx.can_access_account(&id) {
        return Err(ApiError::not_found(v2(), format!("No such account: {id}")));
    }
    let includes = collect_includes(query.as_deref().unwrap_or(""));
    let account: Option<AccountRow> = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal_v2)?;
    let account =
        account.ok_or_else(|| ApiError::not_found(v2(), format!("No such account: {id}")))?;
    Ok(ok(account.to_v2(&includes)))
}

pub async fn update_account(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    V2Json(body): V2Json,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    if !ctx.can_access_account(&id) {
        return Err(ApiError::not_found(v2(), format!("No such account: {id}")));
    }
    let existing: Option<AccountRow> = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal_v2)?;
    let existing =
        existing.ok_or_else(|| ApiError::not_found(v2(), format!("No such account: {id}")))?;

    // Merge configuration sections and track what changed.
    let mut configuration = match crate::models::parse_json(&existing.configuration) {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let mut capabilities = match existing.capabilities() {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    let mut changed_sections: Vec<String> = Vec::new();
    let mut changed_capabilities: Vec<String> = Vec::new();
    if let Some(new_config) = body.get("configuration").and_then(|v| v.as_object()) {
        for (section, value) in new_config {
            changed_sections.push(section.clone());
            configuration.insert(section.clone(), value.clone());
            if let Some(caps) = value.get("capabilities").and_then(|c| c.as_object()) {
                for name in caps.keys() {
                    if !capabilities.contains_key(name) {
                        capabilities.insert(name.clone(), json!("pending"));
                        changed_capabilities.push(name.clone());
                    }
                }
            }
        }
    }
    let config_value = Value::Object(configuration);
    let caps_value = Value::Object(capabilities);
    let identity = body.get("identity").cloned();
    let defaults = body.get("defaults").cloned();
    let metadata = body.get("metadata").cloned();
    let now = time::now_epoch();

    let mut tx = st.pool.begin().await.map_err(ApiError::internal_v2)?;
    sqlx::query(
        "UPDATE accounts SET updated = ?1, identity = COALESCE(?2, identity), configuration = ?3,
            capabilities = ?4, defaults = COALESCE(?5, defaults), metadata = COALESCE(?6, metadata)
         WHERE id = ?7",
    )
    .bind(now)
    .bind(identity.as_ref().map(|v| v.to_string()))
    .bind(config_value.to_string())
    .bind(caps_value.to_string())
    .bind(defaults.as_ref().map(|v| v.to_string()))
    .bind(metadata.as_ref().map(|v| v.to_string()))
    .bind(&id)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal_v2)?;

    let account: AccountRow = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::internal_v2)?;

    let is_connected = account.is_main == 0;
    // Updating a v2 Account also emits its v1 event family: a snapshot
    // `account.updated` and a thin `v1.account.updated`. Both use the
    // "Connected accounts" scope, unlike the "Your account" v2 events below.
    let mut event_ids = Vec::new();
    event_ids.push(
        events::emit(
            &mut tx,
            &account.id,
            is_connected,
            "account.updated",
            account.to_v1(),
        )
        .await
        .map_err(ApiError::internal_v2)?,
    );
    let v1_envelope = events::thin_envelope(
        events::account_v1_related_object(&account.id),
        Value::Null,
        Value::Null,
    );
    event_ids.push(
        events::emit_thin(
            &mut tx,
            &account.id,
            if is_connected {
                Some(account.id.as_str())
            } else {
                None
            },
            "v1.account.updated",
            v1_envelope,
            None,
        )
        .await
        .map_err(ApiError::internal_v2)?,
    );
    for capability in &changed_capabilities {
        let section = events::capability_section(&config_value, capability);
        let event_type =
            format!("v2.core.account[configuration.{section}].capability_status_updated");
        let envelope = events::thin_envelope(
            events::account_related_object(&account.id),
            json!({ "updated_capability": capability }),
            Value::Null,
        );
        event_ids.push(
            events::emit_thin(
                &mut tx,
                &ctx.key_account_id,
                Some(&account.id),
                &event_type,
                envelope,
                None,
            )
            .await
            .map_err(ApiError::internal_v2)?,
        );
    }
    for section in &changed_sections {
        let event_type = format!("v2.core.account[configuration.{section}].updated");
        let envelope = events::thin_envelope(
            events::account_related_object(&account.id),
            json!({}),
            Value::Null,
        );
        event_ids.push(
            events::emit_thin(
                &mut tx,
                &ctx.key_account_id,
                Some(&account.id),
                &event_type,
                envelope,
                None,
            )
            .await
            .map_err(ApiError::internal_v2)?,
        );
    }
    let envelope = events::thin_envelope(
        events::account_related_object(&account.id),
        Value::Null,
        Value::Null,
    );
    event_ids.push(
        events::emit_thin(
            &mut tx,
            &ctx.key_account_id,
            Some(&account.id),
            "v2.core.account.updated",
            envelope,
            None,
        )
        .await
        .map_err(ApiError::internal_v2)?,
    );

    tx.commit().await.map_err(ApiError::internal_v2)?;
    for event_id in &event_ids {
        let _ = st.dispatcher.dispatch_event(event_id).await;
    }

    let includes = includes_from(query.as_deref(), &body);
    Ok(ok(account.to_v2(&includes)))
}

pub async fn list_accounts(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let query_str = query.as_deref().unwrap_or("");
    let params = crate::form::parse_form(query_str);
    let accounts: Vec<AccountRow> = if ctx.key_is_main {
        sqlx::query_as("SELECT * FROM accounts WHERE is_main = 0 ORDER BY created DESC, rowid DESC")
            .fetch_all(&st.pool)
            .await
    } else {
        sqlx::query_as("SELECT * FROM accounts WHERE id = ?1 ORDER BY created DESC, rowid DESC")
            .bind(&ctx.account_id)
            .fetch_all(&st.pool)
            .await
    }
    .map_err(ApiError::internal_v2)?;
    let limit = get_i64(&params, &["limit"]);
    let (page, has_more) = paginate(
        accounts,
        limit,
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |a| a.id.clone(),
    );
    let data = page.iter().map(|a| a.to_v2(&[])).collect();
    let next_page_url = if has_more {
        page.last().map(|a| {
            format!(
                "{}/v2/core/accounts?starting_after={}",
                st.config.base_url(),
                a.id
            )
        })
    } else {
        None
    };
    Ok(ok(list_envelope_v2(
        "/v2/core/accounts",
        data,
        has_more,
        next_page_url,
    )))
}

fn includes_from(query: Option<&str>, body: &Value) -> Vec<String> {
    let mut includes = collect_includes(query.unwrap_or(""));
    if let Some(arr) = body.get("include").and_then(|v| v.as_array()) {
        for value in arr {
            if let Some(s) = value.as_str() {
                includes.push(s.to_string());
            }
        }
    }
    includes
}

fn include_url(query: Option<&str>) -> bool {
    collect_includes(query.unwrap_or(""))
        .iter()
        .any(|i| i == "webhook_endpoint.url")
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value.and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect::<Vec<String>>()
    })
}

/// Validate `events_from`: a non-empty array of `@self` / `@accounts`.
fn validate_events_from(value: &Value) -> Result<Value, ApiError> {
    let entries = value.as_array().filter(|a| !a.is_empty()).ok_or_else(|| {
        ApiError::invalid_param(v2(), "events_from", "events_from must be a non-empty array")
    })?;
    for entry in entries {
        if !matches!(entry.as_str(), Some("@self") | Some("@accounts")) {
            return Err(ApiError::invalid_param(
                v2(),
                "events_from",
                "events_from entries must be \"@self\" or \"@accounts\"",
            ));
        }
    }
    Ok(value.clone())
}

async fn load_destination(
    st: &AppState,
    account_id: &str,
    id: &str,
) -> Result<EventDestinationRow, ApiError> {
    sqlx::query_as("SELECT * FROM event_destinations WHERE id = ?1 AND account_id = ?2")
        .bind(id)
        .bind(account_id)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal_v2)?
        .ok_or_else(|| ApiError::not_found(v2(), format!("No such event destination: {id}")))
}

pub async fn create_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
    V2Json(body): V2Json,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let name = get_str(&body, &["name"]).ok_or_else(|| ApiError::missing_param(v2(), "name"))?;
    let dtype = get_str(&body, &["type"]).ok_or_else(|| ApiError::missing_param(v2(), "type"))?;
    if dtype != "webhook_endpoint" {
        return Err(ApiError::invalid_param(
            v2(),
            "type",
            "Only webhook_endpoint destinations are supported",
        ));
    }
    let event_payload = get_str(&body, &["event_payload"])
        .ok_or_else(|| ApiError::missing_param(v2(), "event_payload"))?;
    if event_payload != "thin" && event_payload != "snapshot" {
        return Err(ApiError::invalid_param(
            v2(),
            "event_payload",
            "event_payload must be \"snapshot\" or \"thin\"",
        ));
    }
    let enabled_events = string_array(body.get("enabled_events")).unwrap_or_default();
    if enabled_events.is_empty() {
        return Err(ApiError::missing_param(v2(), "enabled_events"));
    }
    let url = get_str(&body, &["webhook_endpoint", "url"])
        .ok_or_else(|| ApiError::missing_param(v2(), "webhook_endpoint.url"))?;
    let description = get_str(&body, &["description"]);
    let metadata = body.get("metadata").cloned().unwrap_or_else(|| json!({}));
    let events_from = match body.get("events_from") {
        Some(value) => validate_events_from(value)?,
        None => json!(["@self"]),
    };
    let snapshot_api_version = get_str(&body, &["snapshot_api_version"]);
    let includes = includes_from(query.as_deref(), &body);
    let include_url = includes.iter().any(|i| i == "webhook_endpoint.url");
    let include_secret = includes
        .iter()
        .any(|i| i == "webhook_endpoint.signing_secret");

    let id = ids::event_destination_id();
    let secret = ids::webhook_secret_for_url(&url);
    let now = time::now_epoch();
    sqlx::query(
        "INSERT INTO event_destinations (id, account_id, name, description, url, enabled_events, event_payload, events_from, status, metadata, snapshot_api_version, secret, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'enabled', ?9, ?10, ?11, ?12, ?12)",
    )
    .bind(&id)
    .bind(&ctx.key_account_id)
    .bind(&name)
    .bind(&description)
    .bind(&url)
    .bind(serde_json::to_string(&enabled_events).unwrap_or_else(|_| "[]".to_string()))
    .bind(&event_payload)
    .bind(events_from.to_string())
    .bind(metadata.to_string())
    .bind(&snapshot_api_version)
    .bind(&secret)
    .bind(now)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;

    let destination = load_destination(&st, &ctx.key_account_id, &id).await?;
    Ok(ok(destination.to_json(include_url, include_secret)))
}

pub async fn list_event_destinations(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let query_str = query.as_deref().unwrap_or("");
    let include_url = include_url(query.as_deref());
    let destinations: Vec<EventDestinationRow> = sqlx::query_as(
        "SELECT * FROM event_destinations WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.key_account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;
    let params = crate::form::parse_form(query_str);
    let limit = get_i64(&params, &["limit"]);
    let (page, has_more) = paginate(
        destinations,
        limit,
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |d| d.id.clone(),
    );
    let data = page.iter().map(|d| d.to_json(include_url, false)).collect();
    let next_page_url = if has_more {
        page.last().map(|d| {
            format!(
                "{}/v2/core/event_destinations?starting_after={}",
                st.config.base_url(),
                d.id
            )
        })
    } else {
        None
    };
    Ok(ok(list_envelope_v2(
        "/v2/core/event_destinations",
        data,
        has_more,
        next_page_url,
    )))
}

pub async fn get_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let destination = load_destination(&st, &ctx.key_account_id, &id).await?;
    Ok(ok(destination.to_json(include_url(query.as_deref()), false)))
}

pub async fn update_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
    V2Json(body): V2Json,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let existing = load_destination(&st, &ctx.key_account_id, &id).await?;

    let name = get_str(&body, &["name"]).unwrap_or_else(|| existing.name.clone());
    let description = match body.get("description") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) => None,
        _ => existing.description.clone(),
    };
    let enabled_events = match string_array(body.get("enabled_events")) {
        Some(events) if events.is_empty() => {
            return Err(ApiError::missing_param(v2(), "enabled_events"));
        }
        Some(events) => {
            serde_json::to_string(&events).unwrap_or_else(|_| existing.enabled_events.clone())
        }
        None => existing.enabled_events.clone(),
    };
    let metadata = body
        .get("metadata")
        .cloned()
        .unwrap_or_else(|| crate::models::parse_json_or_empty(&existing.metadata));
    let url = get_str(&body, &["webhook_endpoint", "url"]).unwrap_or_else(|| existing.url.clone());
    let secret = ids::webhook_secret_for_url(&url);
    let events_from = match body.get("events_from") {
        Some(value) => Some(validate_events_from(value)?.to_string()),
        None => existing.events_from.clone(),
    };
    let snapshot_api_version = match body.get("snapshot_api_version") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Null) => None,
        _ => existing.snapshot_api_version.clone(),
    };
    if let Some(payload) = get_str(&body, &["event_payload"]) {
        if payload != existing.event_payload {
            return Err(ApiError::invalid_param(
                v2(),
                "event_payload",
                "event_payload cannot be changed",
            ));
        }
    }
    let now = time::now_epoch();

    sqlx::query(
        "UPDATE event_destinations SET name = ?1, description = ?2, enabled_events = ?3, metadata = ?4, url = ?5, secret = ?6, updated = ?7, events_from = ?10, snapshot_api_version = ?11 WHERE id = ?8 AND account_id = ?9",
    )
    .bind(&name)
    .bind(&description)
    .bind(&enabled_events)
    .bind(metadata.to_string())
    .bind(&url)
    .bind(&secret)
    .bind(now)
    .bind(&id)
    .bind(&ctx.key_account_id)
    .bind(&events_from)
    .bind(&snapshot_api_version)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;

    let destination = load_destination(&st, &ctx.key_account_id, &id).await?;
    Ok(ok(destination.to_json(include_url(query.as_deref()), false)))
}

pub async fn delete_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let result = sqlx::query("DELETE FROM event_destinations WHERE id = ?1 AND account_id = ?2")
        .bind(&id)
        .bind(&ctx.key_account_id)
        .execute(&st.pool)
        .await
        .map_err(ApiError::internal_v2)?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found(
            v2(),
            format!("No such event destination: {id}"),
        ));
    }
    let mut obj = empty_object();
    obj.insert("id".into(), json!(id));
    obj.insert("object".into(), json!("v2.core.event_destination"));
    obj.insert("deleted".into(), json!(true));
    Ok(ok(Value::Object(obj)))
}

async fn set_destination_status(
    st: &AppState,
    ctx: &AuthContext,
    id: &str,
    status: &str,
    query: Option<&str>,
) -> Result<Response, ApiError> {
    let _ = load_destination(st, &ctx.key_account_id, id).await?;
    sqlx::query(
        "UPDATE event_destinations SET status = ?1, updated = ?2 WHERE id = ?3 AND account_id = ?4",
    )
    .bind(status)
    .bind(time::now_epoch())
    .bind(id)
    .bind(&ctx.key_account_id)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;
    let destination = load_destination(st, &ctx.key_account_id, id).await?;
    Ok(ok(destination.to_json(include_url(query), false)))
}

pub async fn enable_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    set_destination_status(&st, &ctx, &id, "enabled", query.as_deref()).await
}

pub async fn disable_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    set_destination_status(&st, &ctx, &id, "disabled", query.as_deref()).await
}

pub async fn ping_event_destination(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let destination = load_destination(&st, &ctx.key_account_id, &id).await?;

    let envelope = json!({
        "related_object": {
            "id": destination.id,
            "type": "v2.core.event_destination",
            "url": format!("/v2/core/event_destinations/{}", destination.id),
        },
        "data": Value::Null,
        "changes": Value::Null,
        "context": Value::Null,
        "reason": Value::Null,
    });
    let mut tx = st.pool.begin().await.map_err(ApiError::internal_v2)?;
    let event_id = events::emit_thin(
        &mut tx,
        &ctx.key_account_id,
        None,
        "v2.core.event_destination.ping",
        envelope,
        Some(&destination.id),
    )
    .await
    .map_err(ApiError::internal_v2)?;
    tx.commit().await.map_err(ApiError::internal_v2)?;
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let event: EventRow = sqlx::query_as("SELECT * FROM events WHERE id = ?1")
        .bind(&event_id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal_v2)?;
    Ok(ok(event.to_v2_full()))
}

pub async fn list_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let query_str = query.as_deref().unwrap_or("");
    let params = crate::form::parse_form(query_str);
    let mut events: Vec<EventRow> = sqlx::query_as(&format!(
        "SELECT * FROM events WHERE version = 'v2' AND {} ORDER BY created DESC, rowid DESC",
        events::EVENT_VISIBLE_SQL
    ))
    .bind(&ctx.key_account_id)
    .bind(ctx.key_is_main as i64)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;

    if let Some(object_id) = get_str(&params, &["object_id"]) {
        events.retain(|e| e.v2_related_id().as_deref() == Some(object_id.as_str()));
    }
    if let Some(types) = params.get("types") {
        let types: Vec<String> = match types {
            Value::Array(a) => a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Value::String(s) => s.split(',').map(str::to_string).collect(),
            _ => Vec::new(),
        };
        if !types.is_empty() {
            events.retain(|e| types.iter().any(|t| t == &e.event_type));
        }
    }

    let limit = get_i64(&params, &["limit"]);
    let (page, has_more) = paginate(
        events,
        limit,
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |e| e.id.clone(),
    );
    let data = page.iter().map(|e| e.to_v2_full()).collect();
    let next_page_url = if has_more {
        page.last().map(|e| {
            format!(
                "{}/v2/core/events?starting_after={}",
                st.config.base_url(),
                e.id
            )
        })
    } else {
        None
    };
    Ok(ok(list_envelope_v2(
        "/v2/core/events",
        data,
        has_more,
        next_page_url,
    )))
}

pub async fn get_event(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let event: Option<EventRow> = sqlx::query_as(&format!(
        "SELECT * FROM events WHERE id = ?3 AND version = 'v2' AND {}",
        events::EVENT_VISIBLE_SQL
    ))
    .bind(&ctx.key_account_id)
    .bind(ctx.key_is_main as i64)
    .bind(&id)
    .fetch_optional(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;
    let event = event.ok_or_else(|| ApiError::not_found(v2(), format!("No such event: {id}")))?;
    Ok(ok(event.to_v2_full()))
}

pub async fn create_account_link(
    State(st): State<AppState>,
    headers: HeaderMap,
    V2Json(body): V2Json,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let account =
        get_str(&body, &["account"]).ok_or_else(|| ApiError::missing_param(v2(), "account"))?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM accounts WHERE id = ?1")
        .bind(&account)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal_v2)?;
    if exists.is_none() || !ctx.can_access_account(&account) {
        return Err(ApiError::not_found(
            v2(),
            format!("No such account: {account}"),
        ));
    }
    let use_case = body.get("use_case").cloned().unwrap_or_else(|| json!({}));
    let (link_type, refresh_url, return_url) =
        if let Some(onboarding) = use_case.get("account_onboarding") {
            (
                "account_onboarding".to_string(),
                get_str(onboarding, &["refresh_url"]),
                get_str(onboarding, &["return_url"]),
            )
        } else if let Some(update) = use_case.get("account_update") {
            (
                "account_update".to_string(),
                get_str(update, &["refresh_url"]),
                get_str(update, &["return_url"]),
            )
        } else {
            return Err(ApiError::invalid_param(
                v2(),
                "use_case",
                "Unknown use case",
            ));
        };

    let token = ids::token();
    let now = time::now_epoch();
    let expires_at = now + 300;
    sqlx::query(
        "INSERT INTO account_links (token, account_id, link_type, refresh_url, return_url, created, expires_at, used)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
    )
    .bind(&token)
    .bind(&account)
    .bind(&link_type)
    .bind(&refresh_url)
    .bind(&return_url)
    .bind(now)
    .bind(expires_at)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal_v2)?;

    let url = format!("{}/mock/connect/{}", st.config.base_url(), token);
    let mut obj = crate::util::empty_object();
    obj.insert("id".into(), json!(ids::id("aclink_")));
    obj.insert("object".into(), json!("v2.core.account_link"));
    obj.insert("account".into(), json!(account));
    obj.insert("created".into(), json!(time::epoch_to_rfc3339_ms(now)));
    obj.insert(
        "expires_at".into(),
        json!(time::epoch_to_rfc3339_ms(expires_at)),
    );
    obj.insert("url".into(), json!(url));
    let mut use_case_obj = crate::util::empty_object();
    use_case_obj.insert(
        link_type,
        json!({ "refresh_url": refresh_url, "return_url": return_url }),
    );
    obj.insert("use_case".into(), Value::Object(use_case_obj));
    Ok(ok(Value::Object(obj)))
}
