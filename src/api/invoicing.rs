//! Invoice and payment intent endpoints.
//!
//! Invoices are priced from the account's catalog, move through a small
//! status machine (`draft` → `open` → `paid` / `void` / `uncollectible`), and
//! are settled by payment intents. The settlement rule lives in
//! [`settle_invoice_payment`] so that paying an invoice through
//! `POST /v1/invoices/{id}/pay` and settling a payment intent from a test
//! produce the same invoice state and the same events.

use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Value, json};

use crate::AppState;
use crate::api::v1::{load_price, metadata_from};
use crate::api::{get_array, get_i64, get_str, ok, paginate};
use crate::auth::{self, AuthContext};
use crate::error::{ApiError, ApiVersion};
use crate::events;
use crate::form;
use crate::ids;
use crate::models::*;
use crate::time;
use crate::util::list_envelope;

fn v1() -> ApiVersion {
    ApiVersion::V1
}

async fn auth(st: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    auth::authenticate(&st.pool, headers, v1()).await
}

/// Load an invoice owned by the account.
async fn load_invoice(st: &AppState, account_id: &str, id: &str) -> Result<InvoiceRow, ApiError> {
    let invoice: Option<InvoiceRow> =
        sqlx::query_as("SELECT * FROM invoices WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    invoice.ok_or_else(|| ApiError::not_found(v1(), format!("No such invoice: {id}")))
}

/// Load a payment intent owned by the account.
async fn load_payment_intent(
    st: &AppState,
    account_id: &str,
    id: &str,
) -> Result<PaymentIntentRow, ApiError> {
    let intent: Option<PaymentIntentRow> =
        sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    intent.ok_or_else(|| ApiError::not_found(v1(), format!("No such payment intent: {id}")))
}

/// Whether the account owning an invoice exists for this request's account.
async fn load_customer(st: &AppState, account_id: &str, id: &str) -> Result<CustomerRow, ApiError> {
    let customer: Option<CustomerRow> =
        sqlx::query_as("SELECT * FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    customer.ok_or_else(|| ApiError::not_found(v1(), format!("No such customer: {id}")))
}

/// Read a `quantity`: absent means 1, anything else must be a positive integer.
fn parse_quantity(item: &Value) -> Result<i64, ApiError> {
    match get_str(item, &["quantity"]) {
        None => Ok(1),
        Some(raw) => raw
            .parse::<i64>()
            .ok()
            .filter(|q| (1..=999_999).contains(q))
            .ok_or_else(|| {
                ApiError::invalid_param(
                    v1(),
                    "quantity",
                    "Invalid quantity: must be a positive integer",
                )
            }),
    }
}

/// Round a tax amount to the nearest minor unit, halves away from zero.
fn round_tax(amount: i64, percentage: f64) -> i64 {
    let raw = amount as f64 * percentage / 100.0;
    if raw >= 0.0 {
        (raw + 0.5).floor() as i64
    } else {
        (raw - 0.5).ceil() as i64
    }
}

/// Render a percentage the way Stripe does: whole numbers stay integers.
fn percentage_value(percentage: f64) -> Value {
    if percentage.fract() == 0.0 {
        json!(percentage as i64)
    } else {
        json!(percentage)
    }
}

/// Build one invoice line from a price, a quantity, and an optional tax rate.
///
/// The tax rate is given inline as `tax_rate[percentage]` (and an optional
/// `tax_rate[name]`); it is materialized as a `tax_rate` object on the line.
fn build_invoice_line(
    price: &PriceRow,
    quantity: i64,
    tax_rate: Option<&Value>,
) -> Result<(Value, i64, i64), ApiError> {
    let amount = price.unit_amount.unwrap_or(0).saturating_mul(quantity);
    let mut tax_amounts = Vec::new();
    let mut tax_rates = Vec::new();
    let mut tax_total = 0i64;

    if let Some(rate) = tax_rate {
        let percentage = get_str(rate, &["percentage"])
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .ok_or_else(|| {
                ApiError::invalid_param(
                    v1(),
                    "lines[][tax_rate][percentage]",
                    "Invalid tax rate percentage",
                )
            })?;
        if !(0.0..=100.0).contains(&percentage) {
            return Err(ApiError::invalid_param(
                v1(),
                "lines[][tax_rate][percentage]",
                "Invalid tax rate percentage: must be between 0 and 100",
            ));
        }
        let name = get_str(rate, &["name"]).unwrap_or_else(|| "Tax".to_string());
        let tax_amount = round_tax(amount, percentage);
        tax_total += tax_amount;
        let tax_rate = json!({
            "id": ids::tax_rate_id(),
            "object": "tax_rate",
            "active": true,
            "country": null,
            "created": time::now_epoch(),
            "description": name,
            "display_name": name,
            "inclusive": false,
            "livemode": false,
            "metadata": {},
            "name": name,
            "percentage": percentage_value(percentage),
            "state": null,
            "tax_type": null,
        });
        tax_rates.push(tax_rate.clone());
        tax_amounts.push(json!({
            "amount": tax_amount,
            "taxable_amount": amount,
            "tax_rate": tax_rate,
        }));
    }

    let line = json!({
        "id": ids::id("il_"),
        "object": "line_item",
        "amount": amount,
        "currency": price.currency,
        "description": price.nickname,
        "discountable": true,
        "period": {
            "start": time::now_epoch(),
            "end": time::now_epoch() + 30 * 86_400,
        },
        "price": price.to_json(),
        "proration": false,
        "quantity": quantity,
        "tax_amounts": tax_amounts,
        "tax_rates": tax_rates,
    });
    Ok((line, amount, tax_total))
}

// ---------------------------------------------------------------------------
// Invoices
// ---------------------------------------------------------------------------

pub async fn create_invoice(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let customer_id =
        get_str(&params, &["customer"]).ok_or_else(|| ApiError::missing_param(v1(), "customer"))?;
    let customer = load_customer(&st, &ctx.account_id, &customer_id).await?;

    let raw_lines = get_array(&params, &["lines"]).cloned().unwrap_or_default();
    if raw_lines.is_empty() {
        return Err(ApiError::missing_param(v1(), "lines"));
    }

    let mut lines = Vec::new();
    let mut subtotal = 0i64;
    let mut tax_total = 0i64;
    let mut currency: Option<String> = None;
    for item in &raw_lines {
        let price_id = get_str(item, &["price"])
            .ok_or_else(|| ApiError::missing_param(v1(), "lines[][price]"))?;
        let quantity = parse_quantity(item)?;
        let price = load_price(&st, &ctx.account_id, &price_id, "lines[][price]").await?;
        match &currency {
            Some(existing) if *existing != price.currency => {
                return Err(ApiError::invalid_param(
                    v1(),
                    "lines[][price]",
                    "All invoice lines must use the same currency",
                ));
            }
            Some(_) => {}
            None => currency = Some(price.currency.clone()),
        }
        let (line, amount, line_tax) = build_invoice_line(&price, quantity, item.get("tax_rate"))?;
        subtotal += amount;
        tax_total += line_tax;
        lines.push(line);
    }

    let total = subtotal + tax_total;
    let id = ids::invoice_id();
    let now = time::now_epoch();
    let customer_tax_ids =
        serde_json::to_string(&customer.tax_id_list()).unwrap_or_else(|_| "[]".to_string());

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query(
            "INSERT INTO invoices (id, account_id, customer, status, currency, created, updated,
                finalized_at, paid_at, voided_at, due_date, amount_due, amount_paid, amount_remaining,
                subtotal, tax_total, total, lines, customer_tax_ids, description, metadata)
             VALUES (?1, ?2, ?3, 'draft', ?4, ?5, ?5, NULL, NULL, NULL, ?6, ?7, 0, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(&customer_id)
        .bind(currency.unwrap_or_else(|| "usd".to_string()))
        .bind(now)
        .bind(get_i64(&params, &["due_date"]))
        .bind(total)
        .bind(subtotal)
        .bind(tax_total)
        .bind(total)
        .bind(serde_json::to_string(&lines).unwrap_or_else(|_| "[]".to_string()))
        .bind(customer_tax_ids)
        .bind(get_str(&params, &["description"]))
        .bind(metadata_from(&params).to_string())
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        let invoice: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "invoice.created",
            invoice.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let invoice: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(invoice.to_json()))
}

pub async fn get_invoice(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    Ok(ok(invoice.to_json()))
}

pub async fn list_invoices(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let invoices: Vec<InvoiceRow> = sqlx::query_as(
        "SELECT * FROM invoices WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let customer = get_str(&params, &["customer"]);
    let status = get_str(&params, &["status"]);
    let invoices: Vec<InvoiceRow> = invoices
        .into_iter()
        .filter(|i| match &customer {
            Some(c) => i.customer == *c,
            None => true,
        })
        .filter(|i| match &status {
            Some(s) => i.status == *s,
            None => true,
        })
        .collect();
    let (page, has_more) = paginate(
        invoices,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |i| i.id.clone(),
    );
    let data = page.iter().map(|i| i.to_json()).collect();
    Ok(ok(list_envelope("/v1/invoices", data, has_more)))
}

pub async fn get_invoice_lines(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    Ok(ok(list_envelope(
        &format!("/v1/invoices/{id}/lines"),
        invoice.line_list(),
        false,
    )))
}

// ---------------------------------------------------------------------------
// Invoice status transitions
// ---------------------------------------------------------------------------

/// The invoice status transitions the server allows.
///
/// `paid`, `void`, and `uncollectible` are terminal.
fn transition_allowed(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("draft", "open")
            | ("open", "paid")
            | ("draft", "void")
            | ("open", "void")
            | ("open", "uncollectible")
    )
}

/// Reject a transition that is not allowed, leaving the invoice unchanged.
fn ensure_transition(from: &str, to: &str) -> Result<(), ApiError> {
    if transition_allowed(from, to) {
        return Ok(());
    }
    Err(ApiError::invalid_request(
        v1(),
        format!("Cannot transition an invoice from {from} to {to}"),
    ))
}

/// Move an invoice to `to`, applying the status-specific fields, and emit
/// `event_type` for the resulting invoice.
async fn transition_invoice(
    st: &AppState,
    ctx: &AuthContext,
    id: &str,
    to: &str,
    event_type: &str,
) -> Result<InvoiceRow, ApiError> {
    let invoice = load_invoice(st, ctx.account_id.as_str(), id).await?;
    ensure_transition(&invoice.status, to)?;

    let now = time::now_epoch();
    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        // Claim the invoice first so concurrent transitions cannot both proceed.
        let claimed = sqlx::query("UPDATE invoices SET status = ?1 WHERE id = ?2 AND status = ?3")
            .bind(to)
            .bind(id)
            .bind(&invoice.status)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        if claimed.rows_affected() == 0 {
            return Err(ApiError::invalid_request(
                v1(),
                format!(
                    "Cannot transition an invoice from {} to {to}",
                    invoice.status
                ),
            ));
        }
        match to {
            "open" => {
                // `amount_due` becomes the invoice total; the remaining balance
                // accounts for anything already paid against it.
                sqlx::query("UPDATE invoices SET finalized_at = ?1, amount_due = ?2, amount_remaining = ?3 WHERE id = ?4")
                    .bind(now)
                    .bind(invoice.total)
                    .bind((invoice.total - invoice.amount_paid).max(0))
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            }
            "void" => {
                sqlx::query("UPDATE invoices SET voided_at = ?1 WHERE id = ?2")
                    .bind(now)
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            }
            _ => {}
        }
        let updated: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            event_type,
            updated.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    load_invoice(st, ctx.account_id.as_str(), id).await
}

pub async fn finalize_invoice(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = transition_invoice(&st, &ctx, &id, "open", "invoice.finalized").await?;
    Ok(ok(invoice.to_json()))
}

pub async fn void_invoice(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = transition_invoice(&st, &ctx, &id, "void", "invoice.voided").await?;
    Ok(ok(invoice.to_json()))
}

pub async fn mark_invoice_uncollectible(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = transition_invoice(
        &st,
        &ctx,
        &id,
        "uncollectible",
        "invoice.marked_uncollectible",
    )
    .await?;
    Ok(ok(invoice.to_json()))
}

// ---------------------------------------------------------------------------
// Payment intents
// ---------------------------------------------------------------------------

pub async fn create_payment_intent(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let amount =
        get_i64(&params, &["amount"]).ok_or_else(|| ApiError::missing_param(v1(), "amount"))?;
    if amount < 0 {
        return Err(ApiError::invalid_param(
            v1(),
            "amount",
            "Invalid amount: must not be negative",
        ));
    }
    let currency = get_str(&params, &["currency"]).unwrap_or_else(|| "usd".to_string());
    let customer = get_str(&params, &["customer"]);
    if let Some(customer_id) = &customer {
        // A customer must exist for the account.
        load_customer(&st, &ctx.account_id, customer_id).await?;
    }
    let invoice_id = get_str(&params, &["invoice"]);
    if let Some(invoice) = &invoice_id {
        load_invoice(&st, &ctx.account_id, invoice).await?;
    }

    let id = ids::payment_intent_id();
    let now = time::now_epoch();
    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query(
            "INSERT INTO payment_intents (id, account_id, amount, currency, status, customer, invoice,
                created, checkout_session, metadata)
             VALUES (?1, ?2, ?3, ?4, 'requires_payment_method', ?5, ?6, ?7, NULL, ?8)",
        )
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(amount)
        .bind(&currency)
        .bind(&customer)
        .bind(&invoice_id)
        .bind(now)
        .bind(metadata_from(&params).to_string())
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        let intent: PaymentIntentRow =
            sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "payment_intent.created",
            intent.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let intent: PaymentIntentRow = sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(intent.to_json()))
}

pub async fn get_payment_intent(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let intent = load_payment_intent(&st, &ctx.account_id, &id).await?;
    Ok(ok(intent.to_json()))
}

pub async fn list_payment_intents(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let intents: Vec<PaymentIntentRow> = sqlx::query_as(
        "SELECT * FROM payment_intents WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let customer = get_str(&params, &["customer"]);
    let invoice = get_str(&params, &["invoice"]);
    let intents: Vec<PaymentIntentRow> = intents
        .into_iter()
        .filter(|p| match &customer {
            Some(c) => p.customer.as_deref() == Some(c.as_str()),
            None => true,
        })
        .filter(|p| match &invoice {
            Some(i) => p.invoice.as_deref() == Some(i.as_str()),
            None => true,
        })
        .collect();
    let (page, has_more) = paginate(
        intents,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |p| p.id.clone(),
    );
    let data = page.iter().map(|p| p.to_json()).collect();
    Ok(ok(list_envelope("/v1/payment_intents", data, has_more)))
}

// ---------------------------------------------------------------------------
// Settlement
// ---------------------------------------------------------------------------

/// Apply a successful payment to the invoice it is attached to.
///
/// The invoice's `amount_paid` grows by the payment amount, `amount_remaining`
/// shrinks accordingly (never below zero), and the invoice becomes `paid` once
/// the balance is covered. Emits `invoice.payment_succeeded` and, on full
/// coverage, `invoice.paid`.
///
/// A closed invoice (`paid`, `void`, `uncollectible`) is terminal and is never
/// credited or reopened.
///
/// Returns the event ids to dispatch after the transaction commits.
async fn settle_invoice_payment(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
    is_connected: bool,
    invoice: &InvoiceRow,
    amount: i64,
) -> Result<Vec<String>, sqlx::Error> {
    if matches!(invoice.status.as_str(), "paid" | "void" | "uncollectible") {
        return Ok(Vec::new());
    }

    let amount_paid = invoice.amount_paid + amount;
    let amount_remaining = (invoice.amount_due - amount_paid).max(0);
    let fully_covered = amount_paid >= invoice.amount_due;
    let now = time::now_epoch();

    sqlx::query(
        "UPDATE invoices SET amount_paid = ?1, amount_remaining = ?2, status = ?3, paid_at = ?4, updated = ?5 WHERE id = ?6",
    )
    .bind(amount_paid)
    .bind(amount_remaining)
    .bind(if fully_covered { "paid" } else { invoice.status.as_str() })
    .bind(if fully_covered { Some(now) } else { invoice.paid_at })
    .bind(now)
    .bind(&invoice.id)
    .execute(&mut **tx)
    .await?;

    let updated: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
        .bind(&invoice.id)
        .fetch_one(&mut **tx)
        .await?;

    let mut event_ids = Vec::new();
    event_ids.push(
        events::emit(
            tx,
            account_id,
            is_connected,
            "invoice.payment_succeeded",
            updated.to_json(),
        )
        .await?,
    );
    if fully_covered {
        event_ids.push(
            events::emit(
                tx,
                account_id,
                is_connected,
                "invoice.paid",
                updated.to_json(),
            )
            .await?,
        );
    }
    Ok(event_ids)
}

/// Settle a payment intent and, when it carries an invoice, settle that invoice.
///
/// This is the single settlement path: `POST /v1/invoices/{id}/pay` and
/// `POST /internal/payment_intents/{id}/resolve` both go through it.
pub async fn resolve_payment_intent(
    st: &AppState,
    id: &str,
    status: &str,
    account_id: &str,
    is_connected: bool,
) -> Result<PaymentIntentRow, ApiError> {
    let (event_type, invoice_event_type) = match status {
        "succeeded" => (
            "payment_intent.succeeded",
            Some("invoice.payment_succeeded"),
        ),
        "failed" => (
            "payment_intent.payment_failed",
            Some("invoice.payment_failed"),
        ),
        "canceled" => ("payment_intent.canceled", None),
        other => {
            return Err(ApiError::invalid_param(
                v1(),
                "status",
                format!("Invalid status: {other}. Expected 'succeeded', 'failed', or 'canceled'."),
            ));
        }
    };

    let intent = load_payment_intent(st, account_id, id).await?;
    let invoice = match &intent.invoice {
        Some(invoice_id) => {
            let invoice: Option<InvoiceRow> =
                sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
                    .bind(invoice_id)
                    .fetch_optional(&st.pool)
                    .await
                    .map_err(ApiError::internal)?;
            invoice
        }
        None => None,
    };

    let event_ids = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query("UPDATE payment_intents SET status = ?1 WHERE id = ?2")
            .bind(status)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let updated: PaymentIntentRow =
            sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;

        let mut event_ids = vec![
            events::emit(
                &mut tx,
                account_id,
                is_connected,
                event_type,
                updated.to_json(),
            )
            .await
            .map_err(ApiError::internal)?,
        ];

        if let Some(invoice) = &invoice {
            match status {
                "succeeded" => {
                    event_ids.extend(
                        settle_invoice_payment(
                            &mut tx,
                            account_id,
                            is_connected,
                            invoice,
                            updated.amount.unwrap_or(0),
                        )
                        .await
                        .map_err(ApiError::internal)?,
                    );
                }
                "failed" => {
                    // A failed payment leaves the invoice amounts and status alone.
                    event_ids.push(
                        events::emit(
                            &mut tx,
                            account_id,
                            is_connected,
                            invoice_event_type.unwrap_or("invoice.payment_failed"),
                            invoice.to_json(),
                        )
                        .await
                        .map_err(ApiError::internal)?,
                    );
                }
                _ => {}
            }
        }
        tx.commit().await.map_err(ApiError::internal)?;
        event_ids
    };
    for event_id in &event_ids {
        let _ = st.dispatcher.dispatch_event(event_id).await;
    }

    load_payment_intent(st, account_id, id).await
}

/// Pay an open invoice by creating and settling a payment intent for it.
pub async fn pay_invoice(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    ensure_transition(&invoice.status, "paid")?;

    // Create the payment intent for the outstanding balance.
    let amount = invoice.remaining();
    let currency = invoice.currency.clone();
    let intent_id = ids::payment_intent_id();
    let now = time::now_epoch();

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query(
            "INSERT INTO payment_intents (id, account_id, amount, currency, status, customer, invoice,
                created, checkout_session, metadata)
             VALUES (?1, ?2, ?3, ?4, 'requires_payment_method', ?5, ?6, ?7, NULL, '{}')",
        )
        .bind(&intent_id)
        .bind(&ctx.account_id)
        .bind(amount)
        .bind(&currency)
        .bind(&invoice.customer)
        .bind(&invoice.id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        let intent: PaymentIntentRow =
            sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1")
                .bind(&intent_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "payment_intent.created",
            intent.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    // Settle it, which updates the invoice through the shared settlement path.
    let intent = resolve_payment_intent(
        &st,
        &intent_id,
        "succeeded",
        &ctx.account_id,
        ctx.is_connected,
    )
    .await?;

    // The response is the invoice, as Stripe returns, with the payment intent
    // it settled as an expandable id.
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    let mut body = invoice.to_json();
    if let Some(map) = body.as_object_mut() {
        map.insert("payment_intent".into(), json!(intent.id));
    }
    Ok(ok(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_transitions() {
        assert!(transition_allowed("draft", "open"));
        assert!(transition_allowed("open", "paid"));
        assert!(transition_allowed("draft", "void"));
        assert!(transition_allowed("open", "void"));
        assert!(transition_allowed("open", "uncollectible"));
    }

    #[test]
    fn rejected_transitions() {
        assert!(!transition_allowed("draft", "paid"));
        assert!(!transition_allowed("draft", "uncollectible"));
        assert!(!transition_allowed("open", "open"));
        assert!(!transition_allowed("paid", "open"));
        assert!(!transition_allowed("void", "paid"));
        assert!(!transition_allowed("uncollectible", "paid"));
    }

    #[test]
    fn tax_rounds_to_the_nearest_minor_unit() {
        assert_eq!(round_tax(1000, 10.0), 100);
        assert_eq!(round_tax(1000, 8.25), 83);
        assert_eq!(round_tax(0, 10.0), 0);
        assert_eq!(round_tax(1999, 10.0), 200);
    }

    #[test]
    fn transition_rejection_is_an_invalid_request() {
        let err = ensure_transition("paid", "open").unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.error_type, "invalid_request_error");
    }
}
