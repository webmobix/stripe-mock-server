//! Shared billing calculations: tiered prices, metered usage, and the invoices
//! raised for a subscription period.

use serde_json::{Value, json};
use sqlx::{Row, Sqlite, SqliteConnection, Transaction};

use crate::error::ApiError;
use crate::events;
use crate::ids;
use crate::models::{InvoiceRow, SubscriptionRow, parse_json, usage_number};

/// Round to the nearest minor unit, halves away from zero.
fn round_money(raw: f64) -> i64 {
    if raw >= 0.0 {
        (raw + 0.5).floor() as i64
    } else {
        (raw - 0.5).ceil() as i64
    }
}

fn amount_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(raw) => raw.parse::<f64>().ok(),
        _ => None,
    }
}

/// Whether a rendered price tracks usage through a meter.
pub fn price_is_metered(price: &Value) -> bool {
    price
        .get("recurring")
        .and_then(|recurring| recurring.get("usage_type"))
        .and_then(Value::as_str)
        == Some("metered")
}

/// The meter a rendered metered price points at.
pub fn price_meter(price: &Value) -> Option<String> {
    price
        .get("recurring")
        .and_then(|recurring| recurring.get("meter"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The amount for `quantity` units of a rendered price.
///
/// `per_unit` prices multiply their unit amount. `tiered` prices use the
/// graduated bands or the single volume band that contains the quantity.
pub fn amount_for_quantity(price: &Value, quantity: f64) -> i64 {
    if quantity <= 0.0 {
        return 0;
    }
    if price.get("billing_scheme").and_then(Value::as_str) == Some("tiered") {
        return tiered_amount(price, quantity);
    }
    let unit_amount = price
        .get("unit_amount")
        .and_then(amount_value)
        .unwrap_or(0.0);
    round_money(unit_amount * quantity)
}

fn tiered_amount(price: &Value, quantity: f64) -> i64 {
    let tiers = price
        .get("tiers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if tiers.is_empty() {
        return 0;
    }
    let volume = price.get("tiers_mode").and_then(Value::as_str) == Some("volume");
    if volume {
        for tier in &tiers {
            let up_to = tier
                .get("up_to")
                .and_then(amount_value)
                .unwrap_or(f64::INFINITY);
            if quantity <= up_to {
                let unit = tier
                    .get("unit_amount")
                    .and_then(amount_value)
                    .unwrap_or(0.0);
                let flat = tier
                    .get("flat_amount")
                    .and_then(amount_value)
                    .unwrap_or(0.0);
                return round_money(unit * quantity + flat);
            }
        }
        return 0;
    }

    // Graduated: each band bills only the units that fall inside it.
    let mut total = 0.0;
    let mut lower = 0.0;
    for tier in &tiers {
        let up_to = tier
            .get("up_to")
            .and_then(amount_value)
            .unwrap_or(f64::INFINITY);
        let units = quantity.min(up_to) - lower;
        if units > 0.0 {
            let unit = tier
                .get("unit_amount")
                .and_then(amount_value)
                .unwrap_or(0.0);
            let flat = tier
                .get("flat_amount")
                .and_then(amount_value)
                .unwrap_or(0.0);
            total += units * unit + flat;
        }
        lower = up_to;
        if quantity <= up_to {
            break;
        }
    }
    round_money(total)
}

/// Add a recurring interval to a period start.
pub fn advance_period(start: i64, interval: &str, interval_count: i64) -> i64 {
    let count = interval_count.max(1);
    let seconds = match interval {
        "day" => 86_400,
        "week" => 7 * 86_400,
        "month" => 30 * 86_400,
        "year" => 365 * 86_400,
        _ => 30 * 86_400,
    };
    start + seconds * count
}

/// The subscription's price interval, defaulting to a month.
pub fn subscription_interval(subscription: &SubscriptionRow) -> (String, i64) {
    let items = parse_json(&Some(subscription.items.clone()));
    let first = items
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("price"))
        .and_then(|price| price.get("recurring"));
    let interval = first
        .and_then(|recurring| recurring.get("interval"))
        .and_then(Value::as_str)
        .unwrap_or("month")
        .to_string();
    let count = first
        .and_then(|recurring| recurring.get("interval_count"))
        .and_then(Value::as_i64)
        .unwrap_or(1);
    (interval, count)
}

/// Aggregate a customer's meter events over `[start, end)`.
pub async fn aggregate_meter_usage(
    conn: &mut SqliteConnection,
    meter_id: &str,
    customer: &str,
    start: i64,
    end: i64,
) -> Result<f64, sqlx::Error> {
    let formula: Option<String> =
        sqlx::query_scalar("SELECT aggregation_formula FROM billing_meters WHERE id = ?1")
            .bind(meter_id)
            .fetch_optional(&mut *conn)
            .await?;
    let rows = sqlx::query(
        "SELECT value, timestamp FROM meter_events
         WHERE meter_id = ?1 AND customer = ?2 AND timestamp >= ?3 AND timestamp < ?4
         ORDER BY timestamp, rowid",
    )
    .bind(meter_id)
    .bind(customer)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await?;
    let values: Vec<(f64, i64)> = rows
        .iter()
        .map(|row| (row.get::<f64, _>("value"), row.get::<i64, _>("timestamp")))
        .collect();
    Ok(match formula.as_deref().unwrap_or("sum") {
        "count" => values.len() as f64,
        "last" => values.last().map(|(value, _)| *value).unwrap_or(0.0),
        _ => values.iter().map(|(value, _)| *value).sum(),
    })
}

/// The line items a subscription raises for one period.
///
/// Licensed items bill their quantity. Metered items bill the customer's
/// aggregated usage for the meter over the period.
pub async fn subscription_invoice_lines(
    conn: &mut SqliteConnection,
    subscription: &SubscriptionRow,
    period_start: i64,
    period_end: i64,
    metered_start: i64,
    metered_end: i64,
) -> Result<Vec<Value>, ApiError> {
    let items = parse_json(&Some(subscription.items.clone()))
        .as_array()
        .cloned()
        .unwrap_or_default();
    let customer = subscription.customer.clone().unwrap_or_default();
    let mut lines = Vec::new();
    for item in items {
        let price = item.get("price").cloned().unwrap_or(Value::Null);
        let metered = price_is_metered(&price);
        let quantity = if metered {
            match price_meter(&price) {
                Some(meter) if !customer.is_empty() => {
                    aggregate_meter_usage(&mut *conn, &meter, &customer, metered_start, metered_end)
                        .await
                        .map_err(ApiError::internal)?
                }
                _ => 0.0,
            }
        } else {
            crate::api::get_i64(&item, &["quantity"])
                .filter(|quantity| *quantity >= 1)
                .unwrap_or(1) as f64
        };
        let amount = amount_for_quantity(&price, quantity);
        let (line_start, line_end) = if metered {
            (metered_start, metered_end)
        } else {
            (period_start, period_end)
        };
        lines.push(json!({
            "id": ids::id("il_"),
            "object": "line_item",
            "amount": amount,
            "currency": price.get("currency").cloned().unwrap_or(json!("usd")),
            "description": price.get("nickname").cloned().unwrap_or(Value::Null),
            "discountable": true,
            "period": { "start": line_start, "end": line_end },
            "price": price,
            "proration": false,
            "quantity": if metered {
                usage_number(quantity)
            } else {
                json!(quantity as i64)
            },
            "subscription": subscription.id,
            "subscription_item": item.get("id").cloned().unwrap_or(Value::Null),
            "tax_amounts": [],
            "tax_rates": [],
        }));
    }
    Ok(lines)
}

/// Insert the invoice a subscription raises for a period and emit its events.
///
/// `paid` invoices represent a successful charge; an unpaid invoice stays
/// `open` and carries an attempt. Returns the invoice, its event ids, and the
/// invoice id.
#[allow(clippy::too_many_arguments)]
pub async fn raise_subscription_invoice(
    tx: &mut Transaction<'_, Sqlite>,
    subscription: &SubscriptionRow,
    account_id: &str,
    is_connected: bool,
    period_start: i64,
    period_end: i64,
    metered_start: i64,
    metered_end: i64,
    billing_reason: &str,
    paid: bool,
    created_at: i64,
    base_url: &str,
) -> Result<(InvoiceRow, Vec<String>), ApiError> {
    let conn = &mut *tx;
    let lines = subscription_invoice_lines(
        conn,
        subscription,
        period_start,
        period_end,
        metered_start,
        metered_end,
    )
    .await?;
    let subtotal: i64 = lines
        .iter()
        .map(|line| line.get("amount").and_then(Value::as_i64).unwrap_or(0))
        .sum();
    let tax_total: i64 = lines
        .iter()
        .filter_map(|line| line.get("tax_amounts").and_then(Value::as_array))
        .flatten()
        .map(|tax| tax.get("amount").and_then(Value::as_i64).unwrap_or(0))
        .sum();
    let total = subtotal + tax_total;
    let currency = lines
        .first()
        .and_then(|line| line.get("currency"))
        .and_then(Value::as_str)
        .unwrap_or("usd")
        .to_string();
    let customer = subscription.customer.clone().unwrap_or_default();
    let id = ids::invoice_id();
    let now = created_at;

    sqlx::query(
        "INSERT INTO invoices (id, account_id, customer, status, currency, created, updated,
            finalized_at, paid_at, voided_at, due_date, amount_due, amount_paid, amount_remaining,
            subtotal, tax_total, total, lines, customer_tax_ids, description, metadata,
            subscription, billing_reason, period_start, period_end, collection_method,
            attempt_count, payment_intent, test_clock)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?6, ?7, NULL, NULL, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, '[]', NULL, '{}', ?15, ?16, ?17, ?18,
            'charge_automatically', 1, NULL, ?19)",
    )
    .bind(&id)
    .bind(account_id)
    .bind(&customer)
    .bind(if paid { "paid" } else { "open" })
    .bind(&currency)
    .bind(now)
    .bind(if paid { Some(now) } else { None })
    .bind(total)
    .bind(if paid { total } else { 0 })
    .bind(if paid { 0 } else { total })
    .bind(subtotal)
    .bind(tax_total)
    .bind(total)
    .bind(serde_json::to_string(&lines).unwrap_or_else(|_| "[]".to_string()))
    .bind(&subscription.id)
    .bind(billing_reason)
    .bind(period_start)
    .bind(period_end)
    .bind(&subscription.test_clock)
    .execute(&mut **tx)
    .await
    .map_err(ApiError::internal)?;

    let invoice: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
        .bind(&id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    let data = invoice.to_json(base_url);
    let mut event_ids = vec![
        events::emit_at(
            tx,
            account_id,
            is_connected,
            "invoice.created",
            data.clone(),
            created_at,
        )
        .await
        .map_err(ApiError::internal)?,
    ];
    if paid {
        event_ids.push(
            events::emit_at(
                tx,
                account_id,
                is_connected,
                "invoice.payment_succeeded",
                data.clone(),
                created_at,
            )
            .await
            .map_err(ApiError::internal)?,
        );
        event_ids.push(
            events::emit_at(
                tx,
                account_id,
                is_connected,
                "invoice.paid",
                data,
                created_at,
            )
            .await
            .map_err(ApiError::internal)?,
        );
    } else {
        event_ids.push(
            events::emit_at(
                tx,
                account_id,
                is_connected,
                "invoice.payment_failed",
                data,
                created_at,
            )
            .await
            .map_err(ApiError::internal)?,
        );
    }
    Ok((invoice, event_ids))
}

/// The open invoice for a subscription, if a previous collection failed.
pub async fn open_subscription_invoice(
    conn: &mut SqliteConnection,
    subscription_id: &str,
) -> Result<Option<InvoiceRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT * FROM invoices WHERE subscription = ?1 AND status = 'open'
         ORDER BY created DESC, rowid DESC LIMIT 1",
    )
    .bind(subscription_id)
    .fetch_optional(&mut *conn)
    .await
}

/// How far a subscription should advance.
#[derive(Debug, Clone, Copy)]
pub enum AdvanceRun {
    /// Retry an open invoice, otherwise advance exactly one period.
    OnePeriod,
    /// Retry an open invoice, then advance every period ending at or before
    /// `target`. Used by clock-wide advancement.
    Through(i64),
}

/// Advance a subscription's periods, retrying an open invoice first.
///
/// `retry_at` stamps the open-invoice retry and the final status change.
/// Renewal invoices are stamped with the boundary they cross. The returned
/// invoice is the last one paid or raised, when any.
#[allow(clippy::too_many_arguments)]
pub async fn advance_subscription_periods(
    tx: &mut Transaction<'_, Sqlite>,
    subscription: &SubscriptionRow,
    account_id: &str,
    is_connected: bool,
    run: AdvanceRun,
    paid: bool,
    retry_at: i64,
    base_url: &str,
    event_ids: &mut Vec<String>,
) -> Result<Option<InvoiceRow>, ApiError> {
    let mut current = subscription.clone();
    let mut last_invoice: Option<InvoiceRow> = None;

    // A failed collection is retried before any period is advanced.
    if let Some(invoice) = open_subscription_invoice(tx, &current.id)
        .await
        .map_err(ApiError::internal)?
    {
        let attempt = invoice.attempt_count + 1;
        let status = if paid { "paid" } else { "open" };
        sqlx::query(
            "UPDATE invoices SET status = ?1, amount_paid = ?2, amount_remaining = ?3,
                paid_at = ?4, attempt_count = ?5, updated = ?6 WHERE id = ?7",
        )
        .bind(status)
        .bind(if paid { invoice.amount_due } else { 0 })
        .bind(if paid { 0 } else { invoice.remaining() })
        .bind(if paid { Some(retry_at) } else { None })
        .bind(attempt)
        .bind(retry_at)
        .bind(&invoice.id)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
        let updated: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
            .bind(&invoice.id)
            .fetch_one(&mut **tx)
            .await
            .map_err(ApiError::internal)?;
        let data = updated.to_json(base_url);
        if paid {
            event_ids.push(
                events::emit_at(
                    tx,
                    account_id,
                    is_connected,
                    "invoice.payment_succeeded",
                    data.clone(),
                    retry_at,
                )
                .await
                .map_err(ApiError::internal)?,
            );
            event_ids.push(
                events::emit_at(tx, account_id, is_connected, "invoice.paid", data, retry_at)
                    .await
                    .map_err(ApiError::internal)?,
            );
        } else {
            event_ids.push(
                events::emit_at(
                    tx,
                    account_id,
                    is_connected,
                    "invoice.payment_failed",
                    data,
                    retry_at,
                )
                .await
                .map_err(ApiError::internal)?,
            );
        }
        current.status = if paid { "active" } else { "past_due" }.to_string();
        sqlx::query("UPDATE subscriptions SET status = ?1 WHERE id = ?2")
            .bind(&current.status)
            .bind(&current.id)
            .execute(&mut **tx)
            .await
            .map_err(ApiError::internal)?;
        event_ids.push(
            events::emit_at(
                tx,
                account_id,
                is_connected,
                "customer.subscription.updated",
                current.to_json(),
                retry_at,
            )
            .await
            .map_err(ApiError::internal)?,
        );
        last_invoice = Some(updated);
        if matches!(run, AdvanceRun::OnePeriod) || !paid {
            return Ok(last_invoice);
        }
    }

    let (interval, interval_count) = subscription_interval(&current);
    let status = if paid { "active" } else { "past_due" };

    match run {
        AdvanceRun::OnePeriod => {
            let now = retry_at;
            let old_start = current.current_period_start.unwrap_or(now);
            let old_end = current.current_period_end.unwrap_or(now);
            let period_start = current.current_period_end.unwrap_or(now);
            let period_end = advance_period(period_start, &interval, interval_count);
            let items = with_period(&current.items, period_start, period_end);
            sqlx::query(
                "UPDATE subscriptions SET current_period_start = ?1, current_period_end = ?2,
                    status = ?3, items = ?4 WHERE id = ?5",
            )
            .bind(period_start)
            .bind(period_end)
            .bind(status)
            .bind(items)
            .bind(&current.id)
            .execute(&mut **tx)
            .await
            .map_err(ApiError::internal)?;
            current = load_subscription(tx, &current.id).await?;
            let (invoice, invoice_events) = raise_subscription_invoice(
                tx,
                &current,
                account_id,
                is_connected,
                period_start,
                period_end,
                old_start,
                old_end,
                "subscription_cycle",
                paid,
                now,
                base_url,
            )
            .await?;
            event_ids.extend(invoice_events);
            event_ids.push(
                events::emit_at(
                    tx,
                    account_id,
                    is_connected,
                    "customer.subscription.updated",
                    current.to_json(),
                    now,
                )
                .await
                .map_err(ApiError::internal)?,
            );
            last_invoice = Some(invoice);
        }
        AdvanceRun::Through(target) => {
            // Each crossing renews the subscription for the next period.
            for _ in 0..1_000 {
                let Some(boundary) = current.current_period_end else {
                    break;
                };
                if boundary > target {
                    break;
                }
                if current.cancel_at_period_end != 0 {
                    sqlx::query("UPDATE subscriptions SET status = 'canceled' WHERE id = ?1")
                        .bind(&current.id)
                        .execute(&mut **tx)
                        .await
                        .map_err(ApiError::internal)?;
                    current = load_subscription(tx, &current.id).await?;
                    event_ids.push(
                        events::emit_at(
                            tx,
                            account_id,
                            is_connected,
                            "customer.subscription.deleted",
                            current.to_json(),
                            boundary,
                        )
                        .await
                        .map_err(ApiError::internal)?,
                    );
                    break;
                }
                let old_start = current.current_period_start.unwrap_or(boundary);
                let old_end = boundary;
                let period_start = boundary;
                let period_end = advance_period(boundary, &interval, interval_count);
                let items = with_period(&current.items, period_start, period_end);
                sqlx::query(
                    "UPDATE subscriptions SET current_period_start = ?1, current_period_end = ?2,
                        status = ?3, items = ?4 WHERE id = ?5",
                )
                .bind(period_start)
                .bind(period_end)
                .bind(status)
                .bind(items)
                .bind(&current.id)
                .execute(&mut **tx)
                .await
                .map_err(ApiError::internal)?;
                current = load_subscription(tx, &current.id).await?;
                let (invoice, invoice_events) = raise_subscription_invoice(
                    tx,
                    &current,
                    account_id,
                    is_connected,
                    period_start,
                    period_end,
                    old_start,
                    old_end,
                    "subscription_cycle",
                    paid,
                    old_end,
                    base_url,
                )
                .await?;
                event_ids.extend(invoice_events);
                event_ids.push(
                    events::emit_at(
                        tx,
                        account_id,
                        is_connected,
                        "customer.subscription.updated",
                        current.to_json(),
                        old_end,
                    )
                    .await
                    .map_err(ApiError::internal)?,
                );
                last_invoice = Some(invoice);
            }
        }
    }

    Ok(last_invoice)
}

/// Reload a subscription inside the caller's transaction.
async fn load_subscription(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<SubscriptionRow, ApiError> {
    sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(ApiError::internal)
}

/// Copy a subscription's items with new period bounds.
fn with_period(items: &str, period_start: i64, period_end: i64) -> String {
    match parse_json(&Some(items.to_string())) {
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
        )
        .to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn price(scheme: &str, tiers_mode: Option<&str>, tiers: Value) -> Value {
        json!({
            "billing_scheme": scheme,
            "unit_amount": if scheme == "per_unit" { json!(100) } else { Value::Null },
            "tiers_mode": tiers_mode,
            "tiers": tiers,
        })
    }

    #[test]
    fn per_unit_multiplies_the_unit_amount() {
        let price = price("per_unit", None, Value::Null);
        assert_eq!(amount_for_quantity(&price, 3.0), 300);
        assert_eq!(amount_for_quantity(&price, 0.0), 0);
    }

    #[test]
    fn graduated_tiers_bill_each_band() {
        let price = price(
            "tiered",
            Some("graduated"),
            json!([
                { "up_to": 10, "unit_amount": 100, "flat_amount": null },
                { "up_to": null, "unit_amount": 50, "flat_amount": null },
            ]),
        );
        // 10 * 100 + 15 * 50
        assert_eq!(amount_for_quantity(&price, 25.0), 1750);
        // Only the first band is used.
        assert_eq!(amount_for_quantity(&price, 4.0), 400);
    }

    #[test]
    fn volume_tiers_bill_the_containing_band() {
        let price = price(
            "tiered",
            Some("volume"),
            json!([
                { "up_to": 10, "unit_amount": 100, "flat_amount": null },
                { "up_to": null, "unit_amount": 50, "flat_amount": null },
            ]),
        );
        assert_eq!(amount_for_quantity(&price, 5.0), 500);
        assert_eq!(amount_for_quantity(&price, 15.0), 750);
    }

    #[test]
    fn flat_tier_amounts_are_charged_once_per_used_band() {
        let price = price(
            "tiered",
            Some("graduated"),
            json!([
                { "up_to": 10, "unit_amount": 0, "flat_amount": 500 },
                { "up_to": null, "unit_amount": 10, "flat_amount": 200 },
            ]),
        );
        assert_eq!(amount_for_quantity(&price, 15.0), 500 + 5 * 10 + 200);
    }

    #[test]
    fn months_advance_by_thirty_days() {
        assert_eq!(advance_period(1_000, "month", 1), 1_000 + 30 * 86_400);
        assert_eq!(advance_period(1_000, "week", 2), 1_000 + 14 * 86_400);
        assert_eq!(advance_period(1_000, "year", 1), 1_000 + 365 * 86_400);
    }
}
