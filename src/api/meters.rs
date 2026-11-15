//! Billing meters, meter events, and meter event summaries.

use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Map, Value, json};
use sqlx::Row;

use crate::AppState;
use crate::api::{get_i64, get_str, ok, paginate};
use crate::auth::{self, AuthContext};
use crate::error::{ApiError, ApiVersion};
use crate::form;
use crate::ids;
use crate::models::{MeterEventRow, MeterRow, usage_number};
use crate::time;
use crate::util::list_envelope;

fn v1() -> ApiVersion {
    ApiVersion::V1
}

async fn auth(st: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    auth::authenticate(&st.pool, headers, v1()).await
}

fn invalid(param: &str, message: impl Into<String>) -> ApiError {
    ApiError::invalid_param(v1(), param, message)
}

fn parse_formula(params: &Value) -> Result<String, ApiError> {
    let formula = get_str(params, &["default_aggregation", "formula"])
        .ok_or_else(|| ApiError::missing_param(v1(), "default_aggregation.formula"))?;
    match formula.as_str() {
        "sum" | "count" | "last" => Ok(formula),
        other => Err(invalid(
            "default_aggregation.formula",
            format!("Invalid aggregation formula: {other}. Expected sum, count, or last."),
        )),
    }
}

async fn load_meter(st: &AppState, account_id: &str, id: &str) -> Result<MeterRow, ApiError> {
    let meter: Option<MeterRow> =
        sqlx::query_as("SELECT * FROM billing_meters WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    meter.ok_or_else(|| ApiError::not_found(v1(), format!("No such meter: {id}")))
}

pub async fn create_meter(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let display_name = get_str(&params, &["display_name"])
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::missing_param(v1(), "display_name"))?;
    let event_name = get_str(&params, &["event_name"])
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::missing_param(v1(), "event_name"))?;
    if event_name.len() > 100 {
        return Err(invalid(
            "event_name",
            "event_name must be at most 100 characters",
        ));
    }
    let formula = parse_formula(&params)?;
    let customer_payload_key = get_str(&params, &["customer_mapping", "event_payload_key"])
        .unwrap_or_else(|| "stripe_customer_id".to_string());
    if let Some(mapping_type) = get_str(&params, &["customer_mapping", "type"])
        && mapping_type != "by_id"
    {
        return Err(invalid(
            "customer_mapping.type",
            "customer_mapping.type must be by_id",
        ));
    }
    let value_payload_key = get_str(&params, &["value_settings", "event_payload_key"])
        .unwrap_or_else(|| "value".to_string());
    let event_time_window = get_str(&params, &["event_time_window"]);
    if let Some(window) = &event_time_window
        && !["day", "hour"].contains(&window.as_str())
    {
        return Err(invalid(
            "event_time_window",
            "event_time_window must be day or hour",
        ));
    }

    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM billing_meters WHERE account_id = ?1 AND event_name = ?2 AND status = 'active' LIMIT 1",
    )
    .bind(&ctx.account_id)
    .bind(&event_name)
    .fetch_optional(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    if existing.is_some() {
        return Err(invalid(
            "event_name",
            format!("An active meter for event name {event_name} already exists"),
        ));
    }

    let id = ids::meter_id();
    let now = time::now_epoch();
    sqlx::query(
        "INSERT INTO billing_meters (id, account_id, display_name, event_name, aggregation_formula,
            customer_payload_key, value_payload_key, event_time_window, status, deactivated_at, created, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', NULL, ?9, ?9)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(&display_name)
    .bind(&event_name)
    .bind(&formula)
    .bind(&customer_payload_key)
    .bind(&value_payload_key)
    .bind(&event_time_window)
    .bind(now)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let meter = load_meter(&st, &ctx.account_id, &id).await?;
    Ok(ok(meter.to_json()))
}

pub async fn get_meter(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let meter = load_meter(&st, &ctx.account_id, &id).await?;
    Ok(ok(meter.to_json()))
}

pub async fn list_meters(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let meters: Vec<MeterRow> = sqlx::query_as(
        "SELECT * FROM billing_meters WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let status = get_str(&params, &["status"]);
    let meters: Vec<MeterRow> = meters
        .into_iter()
        .filter(|meter| {
            status
                .as_deref()
                .is_none_or(|status| meter.status == status)
        })
        .collect();
    let (page, has_more) = paginate(
        meters,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |meter| meter.id.clone(),
    );
    let data = page.iter().map(MeterRow::to_json).collect();
    Ok(ok(list_envelope("/v1/billing/meters", data, has_more)))
}

pub async fn update_meter(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let existing = load_meter(&st, &ctx.account_id, &id).await?;
    let params = form::parse_form(&body);
    let display_name = get_str(&params, &["display_name"])
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(existing.display_name);
    sqlx::query("UPDATE billing_meters SET display_name = ?1, updated = ?2 WHERE id = ?3")
        .bind(&display_name)
        .bind(time::now_epoch())
        .bind(&id)
        .execute(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let meter = load_meter(&st, &ctx.account_id, &id).await?;
    Ok(ok(meter.to_json()))
}

async fn set_meter_status(
    st: &AppState,
    account_id: &str,
    id: &str,
    status: &str,
) -> Result<Response, ApiError> {
    let meter = load_meter(st, account_id, id).await?;
    if meter.status == status {
        return Ok(ok(meter.to_json()));
    }
    let now = time::now_epoch();
    sqlx::query(
        "UPDATE billing_meters SET status = ?1, deactivated_at = ?2, updated = ?3 WHERE id = ?4",
    )
    .bind(status)
    .bind(if status == "inactive" {
        Some(now)
    } else {
        None
    })
    .bind(now)
    .bind(id)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let meter = load_meter(st, account_id, id).await?;
    Ok(ok(meter.to_json()))
}

pub async fn deactivate_meter(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    set_meter_status(&st, &ctx.account_id, &id, "inactive").await
}

pub async fn reactivate_meter(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let meter = load_meter(&st, &ctx.account_id, &id).await?;
    // Only one active meter may exist for an event name.
    let conflict: Option<String> = sqlx::query_scalar(
        "SELECT id FROM billing_meters WHERE account_id = ?1 AND event_name = ?2
         AND status = 'active' AND id <> ?3 LIMIT 1",
    )
    .bind(&ctx.account_id)
    .bind(&meter.event_name)
    .bind(&id)
    .fetch_optional(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    if conflict.is_some() {
        return Err(invalid(
            "event_name",
            format!(
                "An active meter for event name {} already exists",
                meter.event_name
            ),
        ));
    }
    set_meter_status(&st, &ctx.account_id, &id, "active").await
}

fn stringify_payload(payload: &Value) -> Option<Map<String, Value>> {
    let object = payload.as_object()?;
    let mut out = Map::new();
    for (key, value) in object {
        let rendered = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return None,
        };
        out.insert(key.clone(), json!(rendered));
    }
    Some(out)
}

pub async fn create_meter_event(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let event_name = get_str(&params, &["event_name"])
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| ApiError::missing_param(v1(), "event_name"))?;
    let payload = params
        .get("payload")
        .filter(|payload| payload.is_object())
        .and_then(stringify_payload)
        .ok_or_else(|| ApiError::missing_param(v1(), "payload"))?;

    let meter: Option<MeterRow> = sqlx::query_as(
        "SELECT * FROM billing_meters WHERE account_id = ?1 AND event_name = ?2 AND status = 'active' LIMIT 1",
    )
    .bind(&ctx.account_id)
    .bind(&event_name)
    .fetch_optional(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let meter = meter.ok_or_else(|| {
        invalid(
            "event_name",
            format!("No active meter found for event name: {event_name}"),
        )
        .with_code("no_meter")
    })?;

    let customer = payload
        .get(&meter.customer_payload_key)
        .and_then(Value::as_str)
        .filter(|customer| !customer.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            invalid(
                &format!("payload[{}]", meter.customer_payload_key),
                format!(
                    "The meter payload must contain {}",
                    meter.customer_payload_key
                ),
            )
        })?;
    let value_raw = payload
        .get(&meter.value_payload_key)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid(
                &format!("payload[{}]", meter.value_payload_key),
                format!("The meter payload must contain {}", meter.value_payload_key),
            )
        })?;
    let value = value_raw
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| {
            invalid(
                &format!("payload[{}]", meter.value_payload_key),
                "Invalid meter event value: must be a non-negative number",
            )
        })?;

    let now = time::now_epoch();
    let timestamp = get_i64(&params, &["timestamp"]).unwrap_or(now);
    if timestamp < now - 35 * 86_400 || timestamp > now + 300 {
        return Err(invalid(
            "timestamp",
            "timestamp must be within the past 35 days or up to 5 minutes in the future",
        ));
    }
    let identifier = get_str(&params, &["identifier"]).unwrap_or_else(ids::meter_event_identifier);
    let duplicate: Option<String> =
        sqlx::query_scalar("SELECT id FROM meter_events WHERE account_id = ?1 AND identifier = ?2")
            .bind(&ctx.account_id)
            .bind(&identifier)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    if duplicate.is_some() {
        return Err(ApiError::invalid_request(
            v1(),
            format!("A meter event with identifier {identifier} has already been submitted."),
        )
        .with_code("duplicate_meter_event"));
    }

    let id = ids::id("mre_");
    sqlx::query(
        "INSERT INTO meter_events (id, account_id, meter_id, event_name, identifier, customer,
            payload, value, timestamp, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(&meter.id)
    .bind(&event_name)
    .bind(&identifier)
    .bind(&customer)
    .bind(Value::Object(payload).to_string())
    .bind(value)
    .bind(timestamp)
    .bind(now)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let event: MeterEventRow = sqlx::query_as("SELECT * FROM meter_events WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(event.to_json()))
}

/// Aggregate the values in one summary bucket.
fn aggregate(values: &[(f64, i64)], formula: &str) -> f64 {
    match formula {
        "count" => values.len() as f64,
        "last" => values.last().map(|(value, _)| *value).unwrap_or(0.0),
        _ => values.iter().map(|(value, _)| *value).sum(),
    }
}

pub async fn list_event_summaries(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let meter = load_meter(&st, &ctx.account_id, &id).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let customer =
        get_str(&params, &["customer"]).ok_or_else(|| ApiError::missing_param(v1(), "customer"))?;
    let start_time = get_i64(&params, &["start_time"])
        .ok_or_else(|| ApiError::missing_param(v1(), "start_time"))?;
    let end_time =
        get_i64(&params, &["end_time"]).ok_or_else(|| ApiError::missing_param(v1(), "end_time"))?;
    if start_time >= end_time {
        return Err(invalid(
            "end_time",
            "end_time must be later than start_time",
        ));
    }
    if start_time % 60 != 0 || end_time % 60 != 0 {
        return Err(invalid(
            "start_time",
            "start_time and end_time must be aligned with minute boundaries",
        ));
    }
    let window = get_str(&params, &["value_grouping_window"]);
    let bucket_seconds = match window.as_deref() {
        None => None,
        Some("hour") => {
            if start_time % 3600 != 0 || end_time % 3600 != 0 {
                return Err(invalid(
                    "start_time",
                    "hourly summaries require start_time and end_time aligned with hour boundaries",
                ));
            }
            Some(3600)
        }
        Some("day") => {
            if start_time % 86_400 != 0 || end_time % 86_400 != 0 {
                return Err(invalid(
                    "start_time",
                    "daily summaries require start_time and end_time aligned with UTC day boundaries",
                ));
            }
            Some(86_400)
        }
        Some(other) => {
            return Err(invalid(
                "value_grouping_window",
                format!("Invalid value_grouping_window: {other}. Expected hour or day."),
            ));
        }
    };

    let rows = sqlx::query(
        "SELECT value, timestamp FROM meter_events
         WHERE meter_id = ?1 AND customer = ?2 AND timestamp >= ?3 AND timestamp < ?4
         ORDER BY timestamp, rowid",
    )
    .bind(&meter.id)
    .bind(&customer)
    .bind(start_time)
    .bind(end_time)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let mut events: Vec<(f64, i64)> = rows
        .iter()
        .map(|row| (row.get::<f64, _>("value"), row.get::<i64, _>("timestamp")))
        .collect();
    events.sort_by_key(|(_, timestamp)| *timestamp);

    let mut summaries: Vec<(i64, i64, f64)> = Vec::new();
    match bucket_seconds {
        None => {
            if !events.is_empty() {
                summaries.push((
                    start_time,
                    end_time,
                    aggregate(&events, &meter.aggregation_formula),
                ));
            }
        }
        Some(seconds) => {
            let mut index = 0;
            while index < events.len() {
                let bucket_start = events[index].1.div_euclid(seconds) * seconds;
                let bucket_end = bucket_start + seconds;
                let mut bucket = Vec::new();
                while index < events.len() && events[index].1 < bucket_end {
                    bucket.push(events[index]);
                    index += 1;
                }
                summaries.push((
                    bucket_start,
                    bucket_end,
                    aggregate(&bucket, &meter.aggregation_formula),
                ));
            }
            summaries.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
        }
    }

    let data: Vec<Value> = summaries
        .into_iter()
        .map(|(start, end, value)| {
            json!({
                "id": ids::meter_event_summary_id(&meter.id, &customer, start, end),
                "object": "billing.meter_event_summary",
                "aggregated_value": usage_number(value),
                "end_time": end,
                "livemode": false,
                "meter": meter.id,
                "start_time": start,
            })
        })
        .collect();
    let limit = get_i64(&params, &["limit"]).unwrap_or(10).clamp(1, 100) as usize;
    let has_more = data.len() > limit;
    let data: Vec<Value> = data.into_iter().take(limit).collect();
    Ok(ok(list_envelope(
        &format!("/v1/billing/meters/{}/event_summaries", meter.id),
        data,
        has_more,
    )))
}
