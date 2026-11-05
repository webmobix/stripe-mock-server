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
use crate::api::v1::{load_price, load_product, metadata_from, parse_currency};
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

// ---------------------------------------------------------------------------
// Invoice lines
// ---------------------------------------------------------------------------

/// A line item as it was requested: priced from the catalog, priced from an
/// inline price, or written directly.
#[derive(Debug, Clone)]
enum LineRequest {
    /// `price` with an optional `quantity`; the price is resolved from the
    /// catalog before the line is rendered.
    Catalog {
        price_id: String,
        quantity: i64,
        price: Option<Box<PriceRow>>,
    },
    /// `price_data` with an inline currency and amount; the product and the
    /// price it names are created for the line, as on Stripe.
    PriceData {
        currency: String,
        amount: i64,
        quantity: i64,
        description: Option<String>,
        objects: Box<InlineObjects>,
    },
    /// `amount` with an optional `description`.
    AdHoc {
        amount: i64,
        description: Option<String>,
    },
}

/// The catalog objects Stripe creates for an inline `price_data` line.
///
/// Stripe creates a product when the line describes one inline, and a price
/// for the line either way. The mock creates them too, so the invoice line
/// carries a price that can be read back like any other.
#[derive(Debug, Clone)]
struct InlineObjects {
    /// The product the line is priced from, when the line named or described
    /// one.
    product: Option<InlineProduct>,
    /// The price created for the line.
    price: InlinePrice,
}

/// The product an inline `price_data` line is priced from.
#[derive(Debug, Clone)]
struct InlineProduct {
    id: String,
    name: String,
    description: Option<String>,
    metadata: Value,
    /// Whether the product still has to be created in the catalog.
    mint: bool,
}

/// The price created for an inline `price_data` line.
#[derive(Debug, Clone)]
struct InlinePrice {
    id: String,
    product_id: String,
    currency: String,
    unit_amount: i64,
    created: i64,
    /// The price as it is rendered on the invoice line.
    json: Value,
}

/// A requested line and the inline tax rate it carries.
#[derive(Debug, Clone)]
struct LineItem {
    request: LineRequest,
    tax_rate: Option<Value>,
}

/// The parts of a rendered invoice line.
#[derive(Debug, Clone)]
struct LineDraft {
    amount: i64,
    currency: String,
    description: Option<String>,
    quantity: Option<i64>,
    price: Option<Value>,
    tax_rate: Option<Value>,
}

/// The totals an invoice derives from its lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InvoiceTotals {
    subtotal: i64,
    tax_total: i64,
    total: i64,
}

/// Parse a line item as catalog-priced (`price`, optional `quantity`), inline
/// priced (`price_data`), or ad-hoc (`amount`, optional `description`).
fn parse_line_request(item: &Value) -> Result<LineRequest, ApiError> {
    if let Some(price_id) = get_str(item, &["price"]) {
        let quantity = parse_quantity(item)?;
        return Ok(LineRequest::Catalog {
            price_id,
            quantity,
            price: None,
        });
    }
    if let Some(price_data) = item.get("price_data").filter(|data| !data.is_null()) {
        return parse_price_data_request(item, price_data);
    }
    if item.get("amount").is_none() {
        return Err(ApiError::invalid_request(
            v1(),
            "Each line must have a `price`, a `price_data`, or an `amount`",
        ));
    }
    let amount = get_i64(item, &["amount"])
        .filter(|amount| *amount >= 0)
        .ok_or_else(|| {
            ApiError::invalid_param(
                v1(),
                "lines[][amount]",
                "Invalid amount: must be a non-negative integer",
            )
        })?;
    Ok(LineRequest::AdHoc {
        amount,
        description: get_str(item, &["description"]),
    })
}

/// Parse a `price_data` line: an inline price with its own currency and
/// amount.
///
/// The product is either named by id or described inline; an inline product
/// is created for the line, and its name becomes the line's description, as
/// on Stripe.
fn parse_price_data_request(item: &Value, price_data: &Value) -> Result<LineRequest, ApiError> {
    let currency = get_str(price_data, &["currency"])
        .map(|currency| currency.trim().to_ascii_lowercase())
        .filter(|currency| currency.len() == 3 && currency.chars().all(|c| c.is_ascii_alphabetic()))
        .ok_or_else(|| {
            ApiError::invalid_param(
                v1(),
                "lines[][price_data][currency]",
                "Invalid currency: must be a three-letter ISO code",
            )
        })?;

    let amount = match get_i64(price_data, &["unit_amount"]) {
        Some(amount) if amount >= 0 => amount,
        Some(_) => {
            return Err(ApiError::invalid_param(
                v1(),
                "lines[][price_data][unit_amount]",
                "Invalid amount: must be a non-negative integer",
            ));
        }
        None => match get_str(price_data, &["unit_amount_decimal"]) {
            Some(decimal) => parse_unit_amount_decimal(&decimal).ok_or_else(|| {
                ApiError::invalid_param(
                    v1(),
                    "lines[][price_data][unit_amount_decimal]",
                    "Invalid amount: must be a non-negative decimal",
                )
            })?,
            None => {
                return Err(ApiError::invalid_param(
                    v1(),
                    "lines[][price_data][unit_amount]",
                    "Invalid amount: must be a non-negative integer",
                ));
            }
        },
    };

    let quantity = parse_quantity(item)?;
    let product_data = price_data
        .get("product_data")
        .filter(|data| !data.is_null());
    let product = match get_str(price_data, &["product"]) {
        Some(id) => InlineProduct {
            id,
            name: String::new(),
            description: None,
            metadata: json!({}),
            // The product already exists; it is only checked, never created.
            mint: false,
        },
        None => InlineProduct {
            id: ids::product_id(),
            name: product_data
                .and_then(|data| get_str(data, &["name"]))
                .unwrap_or_else(|| "Product".to_string()),
            description: product_data.and_then(|data| get_str(data, &["description"])),
            metadata: product_data
                .and_then(|data| data.get("metadata").cloned())
                .filter(|metadata| metadata.is_object())
                .unwrap_or_else(|| json!({})),
            mint: true,
        },
    };

    let created = time::now_epoch();
    let price = InlinePrice {
        id: ids::price_id(),
        product_id: product.id.clone(),
        currency: currency.clone(),
        unit_amount: amount,
        created,
        json: Value::Null,
    };
    let price = InlinePrice {
        json: PriceRow {
            id: price.id.clone(),
            account_id: String::new(),
            product_id: price.product_id.clone(),
            currency: price.currency.clone(),
            unit_amount: Some(price.unit_amount),
            price_type: "one_time".to_string(),
            recurring_interval: None,
            recurring_interval_count: None,
            nickname: None,
            active: 1,
            created,
            metadata: Some("{}".to_string()),
        }
        .to_json(),
        ..price
    };

    Ok(LineRequest::PriceData {
        currency,
        amount,
        quantity,
        description: product_data
            .and_then(|data| get_str(data, &["name"]))
            .or_else(|| get_str(item, &["description"])),
        objects: Box::new(InlineObjects {
            product: Some(product),
            price,
        }),
    })
}

/// Parse a decimal amount such as `20.50` into minor units.
///
/// The amount is rounded to the nearest minor unit, halves away from zero.
fn parse_unit_amount_decimal(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (whole, fraction) = match raw.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (raw, ""),
    };
    if !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: i64 = if whole.is_empty() {
        0
    } else {
        whole.parse().ok()?
    };
    if whole < 0 {
        return None;
    }
    let mut decimals: Vec<char> = fraction.chars().take(3).collect();
    while decimals.len() < 3 {
        decimals.push('0');
    }
    let hundredths: i64 = decimals[..2].iter().collect::<String>().parse().ok()?;
    let thousandths = decimals[2].to_digit(10)? as i64;
    let minor = whole.checked_mul(100)?.checked_add(hundredths)?;
    Some(if thousandths >= 5 { minor + 1 } else { minor })
}

/// Parse the `lines` array of a request into line items.
fn parse_line_items(raw: &[Value]) -> Result<Vec<LineItem>, ApiError> {
    raw.iter()
        .map(|item| {
            Ok(LineItem {
                request: parse_line_request(item)?,
                tax_rate: item.get("tax_rate").filter(|rate| !rate.is_null()).cloned(),
            })
        })
        .collect()
}

/// Build a `tax_rate` object from an inline `tax_rate` parameter.
///
/// The percentage is required; the name is optional.
fn build_tax_rate(rate: &Value) -> Result<Value, ApiError> {
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
    Ok(json!({
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
    }))
}

/// Recompute a line's `tax_amounts` from its amount and the percentages held
/// in its `tax_rates`.
///
/// A line without a tax rate has no tax amounts.
fn recompute_tax_amounts(line: &mut Value) -> Result<(), ApiError> {
    let amount = get_i64(line, &["amount"]).unwrap_or(0);
    let rates = line
        .get("tax_rates")
        .and_then(|rates| rates.as_array())
        .cloned()
        .unwrap_or_default();

    let mut tax_amounts = Vec::new();
    for rate in &rates {
        let percentage = get_str(rate, &["percentage"])
            .and_then(|raw| raw.trim().parse::<f64>().ok())
            .ok_or_else(|| {
                ApiError::invalid_param(
                    v1(),
                    "lines[][tax_rate][percentage]",
                    "Invalid tax rate percentage",
                )
            })?;
        tax_amounts.push(json!({
            "amount": round_tax(amount, percentage),
            "taxable_amount": amount,
            "tax_rate": rate.clone(),
        }));
    }
    line["tax_amounts"] = Value::Array(tax_amounts);
    Ok(())
}

/// Render an invoice line from its draft.
///
/// A catalog-priced line and an ad-hoc line carry the same fields; an ad-hoc
/// line has a null `price` and is priced in the invoice's currency.
fn render_line(draft: &LineDraft) -> Result<Value, ApiError> {
    let mut line = json!({
        "id": ids::id("il_"),
        "object": "line_item",
        "amount": draft.amount,
        "currency": draft.currency,
        "description": draft.description,
        "discountable": true,
        "period": {
            "start": time::now_epoch(),
            "end": time::now_epoch() + 30 * 86_400,
        },
        "price": draft.price.clone().unwrap_or(Value::Null),
        "proration": false,
        "quantity": draft.quantity,
        "tax_amounts": Vec::<Value>::new(),
        "tax_rates": Vec::<Value>::new(),
    });
    if let Some(rate) = &draft.tax_rate {
        line["tax_rates"] = Value::Array(vec![build_tax_rate(rate)?]);
    }
    recompute_tax_amounts(&mut line)?;
    Ok(line)
}

/// Render a requested line in the invoice's currency.
///
/// The catalog price of a catalog-priced line must already be resolved and
/// must be in the invoice's currency. An inline `price_data` line names its
/// own currency, which has to be the invoice's.
fn render_item(item: &LineItem, currency: &str) -> Result<Value, ApiError> {
    let draft = match &item.request {
        LineRequest::Catalog {
            quantity, price, ..
        } => {
            let price = price.as_ref().ok_or_else(|| {
                ApiError::internal("catalog price of an invoice line was not resolved")
            })?;
            LineDraft {
                amount: price.unit_amount.unwrap_or(0).saturating_mul(*quantity),
                currency: currency.to_string(),
                description: price.nickname.clone(),
                quantity: Some(*quantity),
                price: Some(price.to_json()),
                tax_rate: item.tax_rate.clone(),
            }
        }
        LineRequest::PriceData {
            currency: price_currency,
            amount,
            quantity,
            description,
            objects,
        } => {
            if price_currency != currency {
                return Err(ApiError::invalid_request(
                    v1(),
                    format!(
                        "The price_data is in {price_currency} but the invoice is in {currency}"
                    ),
                ));
            }
            LineDraft {
                amount: amount.saturating_mul(*quantity),
                currency: currency.to_string(),
                description: description.clone(),
                quantity: Some(*quantity),
                price: Some(objects.price.json.clone()),
                tax_rate: item.tax_rate.clone(),
            }
        }
        LineRequest::AdHoc {
            amount,
            description,
        } => LineDraft {
            amount: *amount,
            currency: currency.to_string(),
            description: description.clone(),
            quantity: None,
            price: None,
            tax_rate: item.tax_rate.clone(),
        },
    };
    render_line(&draft)
}

/// Derive an invoice's `subtotal`, `tax_total`, and `total` from its lines.
///
/// The subtotal is the sum of the line amounts, the tax total the sum of the
/// line tax amounts, and the total the two added together.
fn compute_totals(lines: &[Value]) -> InvoiceTotals {
    let mut subtotal = 0i64;
    let mut tax_total = 0i64;
    for line in lines {
        subtotal += get_i64(line, &["amount"]).unwrap_or(0);
        if let Some(taxes) = get_array(line, &["tax_amounts"]) {
            for tax in taxes {
                tax_total += get_i64(tax, &["amount"]).unwrap_or(0);
            }
        }
    }
    InvoiceTotals {
        subtotal,
        tax_total,
        total: subtotal + tax_total,
    }
}

/// The currency an invoice is created in.
///
/// The request's `currency` wins, then the customer's currency, then `usd`.
/// The choice is made once, at creation, so that the lines added afterwards
/// are validated against it.
fn resolve_invoice_currency(params: &Value, customer: &CustomerRow) -> Result<String, ApiError> {
    if let Some(currency) = parse_currency(params)? {
        return Ok(currency);
    }
    Ok(customer
        .currency
        .clone()
        .filter(|currency| !currency.is_empty())
        .unwrap_or_else(|| "usd".to_string()))
}

/// Reject an invoice that is not a draft: only a draft invoice can have its
/// lines changed.
fn ensure_draft(status: &str) -> Result<(), ApiError> {
    if status == "draft" {
        return Ok(());
    }
    Err(ApiError::invalid_request(
        v1(),
        format!("Only a draft invoice can have its lines changed; this invoice is {status}"),
    ))
}

/// Accept only the `delete` and `unassign` line behaviors.
///
/// Both detach the line: the mock has no invoice item for `unassign` to leave
/// behind.
fn ensure_behavior(behavior: Option<&str>) -> Result<(), ApiError> {
    match behavior {
        None | Some("delete") | Some("unassign") => Ok(()),
        Some(other) => Err(ApiError::invalid_param(
            v1(),
            "lines[][behavior]",
            format!("Invalid behavior: {other}. Expected 'delete' or 'unassign'."),
        )),
    }
}

/// Check that a line update changes at least one field.
///
/// An update that names a line but changes nothing is rejected rather than
/// treated as a no-op.
fn ensure_updatable(item: &Value) -> Result<(), ApiError> {
    let has = |key: &str| item.get(key).is_some();
    let has_tax_rate = item.get("tax_rate").is_some_and(|rate| !rate.is_null());
    if has("quantity") || has("amount") || has("description") || has("price") || has_tax_rate {
        return Ok(());
    }
    Err(ApiError::invalid_request(
        v1(),
        "Each line to update must change at least one of quantity, amount, description, price, or tax_rate",
    ))
}

// ---------------------------------------------------------------------------
// Invoices
// ---------------------------------------------------------------------------

/// Resolve the catalog objects the requested lines need.
///
/// A catalog-priced line's price is loaded from the catalog and has to be in
/// the invoice's currency; an inline `price_data` line that names a product
/// has to name one that exists.
async fn resolve_line_objects(
    st: &AppState,
    account_id: &str,
    items: &mut [LineItem],
    currency: &str,
) -> Result<(), ApiError> {
    for item in items.iter_mut() {
        match &mut item.request {
            LineRequest::Catalog {
                price_id, price, ..
            } => {
                let id = price_id.clone();
                let loaded = load_price(st, account_id, &id, "lines[][price]").await?;
                if loaded.currency != currency {
                    return Err(ApiError::invalid_request(
                        v1(),
                        format!(
                            "The price {id} is in {} but the invoice is in {currency}",
                            loaded.currency
                        ),
                    ));
                }
                *price = Some(Box::new(loaded));
            }
            LineRequest::PriceData { objects, .. } => {
                if let Some(product) = &objects.product
                    && !product.mint
                {
                    load_product(st, account_id, &product.id, "lines[][price_data][product]")
                        .await?;
                }
            }
            LineRequest::AdHoc { .. } => {}
        }
    }
    Ok(())
}

/// The catalog objects the given lines need created.
///
/// Stripe creates a product and a price for every inline `price_data` line.
fn inline_objects(items: &[LineItem]) -> Vec<InlineObjects> {
    items
        .iter()
        .filter_map(|item| match &item.request {
            LineRequest::PriceData { objects, .. } => Some((**objects).clone()),
            _ => None,
        })
        .collect()
}

/// Create the products and prices Stripe creates for inline `price_data` lines.
///
/// A product described inline is created; a product named by id already
/// exists. The price is created either way.
async fn create_inline_objects(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    account_id: &str,
    objects: &[InlineObjects],
) -> Result<(), ApiError> {
    for object in objects {
        if let Some(product) = &object.product
            && product.mint
        {
            sqlx::query(
                "INSERT INTO products (id, account_id, name, description, active, created, default_price, metadata)
                 VALUES (?1, ?2, ?3, ?4, 1, ?5, NULL, ?6)",
            )
            .bind(&product.id)
            .bind(account_id)
            .bind(&product.name)
            .bind(&product.description)
            .bind(time::now_epoch())
            .bind(product.metadata.to_string())
            .execute(&mut **tx)
            .await
            .map_err(ApiError::internal)?;
        }
        sqlx::query(
            "INSERT INTO prices (id, account_id, product_id, currency, unit_amount, price_type,
                recurring_interval, recurring_interval_count, nickname, active, created, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, 'one_time', NULL, NULL, NULL, 1, ?6, '{}')",
        )
        .bind(&object.price.id)
        .bind(account_id)
        .bind(&object.price.product_id)
        .bind(&object.price.currency)
        .bind(object.price.unit_amount)
        .bind(object.price.created)
        .execute(&mut **tx)
        .await
        .map_err(ApiError::internal)?;
    }
    Ok(())
}

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
    let mut items = parse_line_items(&raw_lines)?;

    // The invoice's currency is fixed at creation: the request's `currency`,
    // then the customer's, then `usd`. It never follows the lines, so an
    // invoice can be created before it has any.
    let currency = resolve_invoice_currency(&params, &customer)?;

    resolve_line_objects(&st, &ctx.account_id, &mut items, &currency).await?;
    let objects = inline_objects(&items);

    let mut lines = Vec::new();
    for item in &items {
        lines.push(render_item(item, &currency)?);
    }
    let totals = compute_totals(&lines);

    let id = ids::invoice_id();
    let now = time::now_epoch();
    let customer_tax_ids =
        serde_json::to_string(&customer.tax_id_list()).unwrap_or_else(|_| "[]".to_string());

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        create_inline_objects(&mut tx, &ctx.account_id, &objects).await?;
        sqlx::query(
            "INSERT INTO invoices (id, account_id, customer, status, currency, created, updated,
                finalized_at, paid_at, voided_at, due_date, amount_due, amount_paid, amount_remaining,
                subtotal, tax_total, total, lines, customer_tax_ids, description, metadata)
             VALUES (?1, ?2, ?3, 'draft', ?4, ?5, ?5, NULL, NULL, NULL, ?6, ?7, 0, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        )
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(&customer_id)
        .bind(currency)
        .bind(now)
        .bind(get_i64(&params, &["due_date"]))
        .bind(totals.total)
        .bind(totals.subtotal)
        .bind(totals.tax_total)
        .bind(totals.total)
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

/// Delete a draft invoice.
///
/// Stripe only lets a draft invoice be deleted; an invoice that has been
/// finalized has to be voided instead. The invoice is removed and the
/// tombstone Stripe returns is given back: the `id` of the invoice, its
/// `object`, and `deleted` set to true.
pub async fn delete_invoice(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    if invoice.status != "draft" {
        return Err(ApiError::invalid_request(
            v1(),
            format!(
                "Only a draft invoice can be deleted; this invoice is {}",
                invoice.status
            ),
        ));
    }

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        // Claim the invoice first so a concurrent request cannot delete it twice.
        let claimed = sqlx::query("DELETE FROM invoices WHERE id = ?1 AND status = 'draft'")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        if claimed.rows_affected() == 0 {
            return Err(ApiError::invalid_request(
                v1(),
                format!(
                    "Only a draft invoice can be deleted; this invoice is {}",
                    invoice.status
                ),
            ));
        }
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "invoice.deleted",
            invoice.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    // Stripe returns a deleted tombstone, not the full invoice.
    Ok(ok(json!({
        "id": invoice.id,
        "object": "invoice",
        "deleted": true,
    })))
}

// ---------------------------------------------------------------------------
// Invoice line editing
// ---------------------------------------------------------------------------

/// Write a new line list to a draft invoice and emit `invoice.updated`.
///
/// The write is a compare-and-swap on the line list the caller read, so a
/// concurrent change is reported instead of being overwritten. The invoice's
/// totals follow the new lines, and its `updated` timestamp moves. The
/// catalog objects the new lines need are created in the same transaction.
async fn write_invoice_lines(
    st: &AppState,
    ctx: &AuthContext,
    invoice: &InvoiceRow,
    lines: Vec<Value>,
    objects: &[InlineObjects],
) -> Result<InvoiceRow, ApiError> {
    let totals = compute_totals(&lines);
    let lines_json = serde_json::to_string(&lines).unwrap_or_else(|_| "[]".to_string());
    let now = time::now_epoch().max(invoice.updated + 1);
    let amount_remaining = (totals.total - invoice.amount_paid).max(0);

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        create_inline_objects(&mut tx, &ctx.account_id, objects).await?;
        let updated = sqlx::query(
            "UPDATE invoices SET lines = ?1, subtotal = ?2, tax_total = ?3, total = ?4,
                amount_due = ?5, amount_remaining = ?6, updated = ?7
             WHERE id = ?8 AND status = 'draft' AND lines = ?9",
        )
        .bind(&lines_json)
        .bind(totals.subtotal)
        .bind(totals.tax_total)
        .bind(totals.total)
        .bind(totals.total)
        .bind(amount_remaining)
        .bind(now)
        .bind(&invoice.id)
        .bind(&invoice.lines)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if updated.rows_affected() == 0 {
            return Err(ApiError::invalid_request(
                v1(),
                format!(
                    "Invoice {} was changed by another request; please retry",
                    invoice.id
                ),
            ));
        }
        let invoice: InvoiceRow = sqlx::query_as("SELECT * FROM invoices WHERE id = ?1")
            .bind(&invoice.id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "invoice.updated",
            invoice.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    load_invoice(st, &ctx.account_id, &invoice.id).await
}

/// Read the `lines` array of a line-editing request.
fn request_lines(params: &Value) -> Result<Vec<Value>, ApiError> {
    let lines = get_array(params, &["lines"]).cloned().unwrap_or_default();
    if lines.is_empty() {
        return Err(ApiError::missing_param(v1(), "lines"));
    }
    Ok(lines)
}

/// Add line items to a draft invoice.
pub async fn add_invoice_lines(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    ensure_draft(&invoice.status)?;

    let params = form::parse_form(&body);
    let raw_lines = request_lines(&params)?;
    let mut items = parse_line_items(&raw_lines)?;

    resolve_line_objects(&st, &ctx.account_id, &mut items, &invoice.currency).await?;
    let objects = inline_objects(&items);

    let mut lines = invoice.line_list();
    for item in &items {
        lines.push(render_item(item, &invoice.currency)?);
    }
    let invoice = write_invoice_lines(&st, &ctx, &invoice, lines, &objects).await?;
    Ok(ok(invoice.to_json()))
}

/// Remove line items from a draft invoice.
pub async fn remove_invoice_lines(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    ensure_draft(&invoice.status)?;

    let params = form::parse_form(&body);
    let raw_lines = request_lines(&params)?;

    let mut removals = Vec::new();
    for item in &raw_lines {
        let line_id = get_str(item, &["id"]).ok_or_else(|| {
            ApiError::invalid_request(v1(), "Each line to remove must have an `id`")
        })?;
        let behavior = get_str(item, &["behavior"]);
        ensure_behavior(behavior.as_deref())?;
        removals.push(line_id);
    }

    let mut lines = invoice.line_list();
    for line_id in &removals {
        let before = lines.len();
        lines.retain(|line| get_str(line, &["id"]).as_deref() != Some(line_id.as_str()));
        if lines.len() == before {
            return Err(ApiError::invalid_request(
                v1(),
                format!("No such invoice line: {line_id}"),
            ));
        }
    }

    let invoice = write_invoice_lines(&st, &ctx, &invoice, lines, &[]).await?;
    Ok(ok(invoice.to_json()))
}

/// Update line items on a draft invoice.
pub async fn update_invoice_lines(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    ensure_draft(&invoice.status)?;

    let params = form::parse_form(&body);
    let raw_lines = request_lines(&params)?;

    let mut lines = invoice.line_list();
    for item in &raw_lines {
        let line_id = get_str(item, &["id"]).ok_or_else(|| {
            ApiError::invalid_request(v1(), "Each line to update must have an `id`")
        })?;
        let index = lines
            .iter()
            .position(|line| get_str(line, &["id"]).as_deref() == Some(line_id.as_str()))
            .ok_or_else(|| {
                ApiError::invalid_request(v1(), format!("No such invoice line: {line_id}"))
            })?;
        apply_line_updates(&st, &ctx, &invoice, &mut lines[index], item).await?;
    }

    let invoice = write_invoice_lines(&st, &ctx, &invoice, lines, &[]).await?;
    Ok(ok(invoice.to_json()))
}

/// Apply the updatable fields of a line item to one of an invoice's lines.
///
/// `quantity` recomputes the amount from the line's price, `amount` sets the
/// amount directly, `description` sets the description, `price` replaces the
/// catalog price, and `tax_rate` replaces the tax rate. The line's tax
/// amounts follow its amount and tax rate.
async fn apply_line_updates(
    st: &AppState,
    ctx: &AuthContext,
    invoice: &InvoiceRow,
    line: &mut Value,
    item: &Value,
) -> Result<(), ApiError> {
    let (has_quantity, has_amount, has_description, has_price, has_tax_rate) = {
        let has = |key: &str| item.get(key).is_some();
        let has_tax_rate = item.get("tax_rate").is_some_and(|rate| !rate.is_null());
        (
            has("quantity"),
            has("amount"),
            has("description"),
            has("price"),
            has_tax_rate,
        )
    };
    ensure_updatable(item)?;

    if has_price {
        let price_id = get_str(item, &["price"]).ok_or_else(|| {
            ApiError::invalid_param(v1(), "lines[][price]", "Missing required param: price")
        })?;
        let price = load_price(st, &ctx.account_id, &price_id, "lines[][price]").await?;
        if price.currency != invoice.currency {
            return Err(ApiError::invalid_request(
                v1(),
                format!(
                    "The price {price_id} is in {} but the invoice is in {}",
                    price.currency, invoice.currency
                ),
            ));
        }
        line["price"] = price.to_json();
    }

    if has_quantity {
        let quantity = parse_quantity(item)?;
        line["quantity"] = json!(quantity);
    }

    // A catalog-priced line's amount follows its price and quantity.
    if has_price || (has_quantity && !line["price"].is_null()) {
        let unit_amount = line["price"]
            .get("unit_amount")
            .and_then(|amount| amount.as_i64())
            .unwrap_or(0);
        let quantity = get_i64(line, &["quantity"]).unwrap_or(1);
        line["amount"] = json!(unit_amount.saturating_mul(quantity));
    }

    if has_amount {
        let amount = get_i64(item, &["amount"])
            .filter(|amount| *amount >= 0)
            .ok_or_else(|| {
                ApiError::invalid_param(
                    v1(),
                    "lines[][amount]",
                    "Invalid amount: must be a non-negative integer",
                )
            })?;
        line["amount"] = json!(amount);
    }

    if has_description {
        line["description"] = match get_str(item, &["description"]) {
            Some(description) => json!(description),
            None => Value::Null,
        };
    }

    if has_tax_rate {
        let rate = build_tax_rate(&item["tax_rate"])?;
        let mut rates = line
            .get("tax_rates")
            .and_then(|rates| rates.as_array())
            .cloned()
            .unwrap_or_default();
        if rates.is_empty() {
            rates.push(rate);
        } else {
            rates[0] = rate;
        }
        line["tax_rates"] = Value::Array(rates);
    }

    recompute_tax_amounts(line)?;
    Ok(())
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
    let invoice = load_invoice(&st, &ctx.account_id, &id).await?;
    // The lines may have been added after the invoice was created, so they are
    // checked here rather than at creation.
    if invoice.line_list().is_empty() {
        return Err(ApiError::invalid_request(
            v1(),
            "You cannot finalize an invoice with no lines",
        ));
    }
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

    /// A customer with the given default currency.
    fn customer_with_currency(currency: Option<&str>) -> CustomerRow {
        CustomerRow {
            id: "cus_1".into(),
            account_id: "acct_1".into(),
            email: None,
            name: None,
            currency: currency.map(|c| c.to_string()),
            created: 10,
            tax_ids: None,
            metadata: None,
        }
    }

    #[test]
    fn the_invoice_currency_follows_the_request_then_the_customer() {
        let customer = customer_with_currency(Some("eur"));

        // The request wins.
        assert_eq!(
            resolve_invoice_currency(&json!({"currency": "GBP"}), &customer).unwrap(),
            "gbp"
        );
        // Then the customer.
        assert_eq!(
            resolve_invoice_currency(&json!({}), &customer).unwrap(),
            "eur"
        );
        // Then usd.
        assert_eq!(
            resolve_invoice_currency(&json!({}), &customer_with_currency(None)).unwrap(),
            "usd"
        );
    }

    #[test]
    fn a_malformed_invoice_currency_is_rejected() {
        let err =
            resolve_invoice_currency(&json!({"currency": "euro"}), &customer_with_currency(None))
                .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.error_type, "invalid_request_error");
        assert_eq!(err.param.as_deref(), Some("currency"));
    }

    #[test]
    fn parses_catalog_and_ad_hoc_line_items() {
        let catalog = parse_line_request(&json!({"price": "price_1", "quantity": "3"})).unwrap();
        match catalog {
            LineRequest::Catalog {
                price_id, quantity, ..
            } => {
                assert_eq!(price_id, "price_1");
                assert_eq!(quantity, 3);
            }
            LineRequest::AdHoc { .. } | LineRequest::PriceData { .. } => {
                panic!("expected a catalog-priced line")
            }
        }

        let ad_hoc =
            parse_line_request(&json!({"amount": "799", "description": "Consulting"})).unwrap();
        match ad_hoc {
            LineRequest::AdHoc {
                amount,
                description,
            } => {
                assert_eq!(amount, 799);
                assert_eq!(description.as_deref(), Some("Consulting"));
            }
            LineRequest::Catalog { .. } | LineRequest::PriceData { .. } => {
                panic!("expected an ad-hoc line")
            }
        }

        // Neither a price nor an amount.
        let err = parse_line_request(&json!({})).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");

        // An amount that is not a non-negative integer.
        let err = parse_line_request(&json!({"amount": "-1"})).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
    }

    #[test]
    fn renders_a_catalog_line_and_an_ad_hoc_line_the_same_way() {
        let catalog = render_line(&LineDraft {
            amount: 4000,
            currency: "usd".to_string(),
            description: Some("Widget".to_string()),
            quantity: Some(2),
            price: Some(json!({"id": "price_1", "object": "price", "currency": "usd"})),
            tax_rate: None,
        })
        .unwrap();
        assert_eq!(catalog["amount"], 4000);
        assert_eq!(catalog["description"], "Widget");
        assert_eq!(catalog["price"]["id"], "price_1");
        assert_eq!(catalog["currency"], "usd");

        let ad_hoc = render_line(&LineDraft {
            amount: 799,
            currency: "usd".to_string(),
            description: Some("Consulting".to_string()),
            quantity: None,
            price: None,
            tax_rate: None,
        })
        .unwrap();
        assert_eq!(ad_hoc["amount"], 799);
        assert_eq!(ad_hoc["description"], "Consulting");
        assert!(ad_hoc["price"].is_null());
        assert_eq!(ad_hoc["currency"], "usd");

        // Both forms carry the same set of fields.
        let fields = |line: &Value| {
            let mut fields: Vec<String> = line
                .as_object()
                .unwrap()
                .keys()
                .map(|key| key.to_string())
                .collect();
            fields.sort();
            fields
        };
        assert_eq!(fields(&catalog), fields(&ad_hoc));
    }

    #[test]
    fn parses_an_inline_price_data_line() {
        let request = parse_line_request(&json!({
            "price_data": {
                "currency": "EUR",
                "unit_amount": 2500,
                "product_data": {"name": "Consulting"}
            },
            "quantity": "2",
        }))
        .unwrap();
        match request {
            LineRequest::PriceData {
                currency,
                amount,
                quantity,
                description,
                objects,
            } => {
                assert_eq!(currency, "eur");
                assert_eq!(amount, 2500);
                assert_eq!(quantity, 2);
                assert_eq!(description.as_deref(), Some("Consulting"));
                assert_eq!(objects.price.unit_amount, 2500);
                assert_eq!(objects.price.currency, "eur");
                assert_eq!(objects.price.json["object"], "price");
                assert_eq!(objects.price.json["unit_amount"], 2500);
                // The product is described inline, so it is created with the line.
                let product = objects.product.expect("an inline product");
                assert!(product.mint);
                assert!(product.id.starts_with("prod_"));
                assert_eq!(product.name, "Consulting");
            }
            _ => panic!("expected an inline price"),
        }
    }

    #[test]
    fn parses_a_price_data_line_with_a_decimal_amount() {
        let request = parse_line_request(&json!({
            "price_data": {"currency": "usd", "unit_amount_decimal": "20.50"}
        }))
        .unwrap();
        match request {
            LineRequest::PriceData { amount, .. } => assert_eq!(amount, 2050),
            _ => panic!("expected an inline price"),
        }
    }

    #[test]
    fn an_inline_price_needs_a_currency_and_an_amount() {
        let err = parse_line_request(&json!({"price_data": {"unit_amount": 100}})).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
        assert_eq!(err.param.as_deref(), Some("lines[][price_data][currency]"));

        let err = parse_line_request(&json!({"price_data": {"currency": "usd"}})).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
        assert_eq!(
            err.param.as_deref(),
            Some("lines[][price_data][unit_amount]")
        );

        let err =
            parse_line_request(&json!({"price_data": {"currency": "usd", "unit_amount": -1}}))
                .unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
    }

    #[test]
    fn an_inline_price_in_another_currency_is_rejected() {
        let item = LineItem {
            request: parse_line_request(&json!({
                "price_data": {"currency": "eur", "unit_amount": 100}
            }))
            .unwrap(),
            tax_rate: None,
        };
        let err = render_item(&item, "usd").unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
        assert!(err.message.contains("eur"), "{}", err.message);
    }

    #[test]
    fn parses_a_decimal_amount_into_minor_units() {
        assert_eq!(parse_unit_amount_decimal("20.50"), Some(2050));
        assert_eq!(parse_unit_amount_decimal("20.5"), Some(2050));
        assert_eq!(parse_unit_amount_decimal("20"), Some(2000));
        assert_eq!(parse_unit_amount_decimal("0.05"), Some(5));
        assert_eq!(parse_unit_amount_decimal(".99"), Some(99));
        assert_eq!(parse_unit_amount_decimal("20.505"), Some(2051));
        assert_eq!(parse_unit_amount_decimal("20.504"), Some(2050));
        assert_eq!(parse_unit_amount_decimal("-1.00"), None);
        assert_eq!(parse_unit_amount_decimal("abc"), None);
        assert_eq!(parse_unit_amount_decimal("1.2.3"), None);
        assert_eq!(parse_unit_amount_decimal(""), None);
    }

    #[test]
    fn derives_the_totals_from_the_lines() {
        assert_eq!(
            compute_totals(&[]),
            InvoiceTotals {
                subtotal: 0,
                tax_total: 0,
                total: 0
            }
        );

        let lines = vec![
            json!({"amount": 2000, "tax_amounts": [{"amount": 200}]}),
            json!({"amount": 500, "tax_amounts": []}),
        ];
        assert_eq!(
            compute_totals(&lines),
            InvoiceTotals {
                subtotal: 2500,
                tax_total: 200,
                total: 2700
            }
        );
    }

    #[test]
    fn recomputes_a_line_tax_from_its_rate() {
        let mut line = json!({
            "amount": 1000,
            "tax_rates": [{"percentage": 10}],
        });
        recompute_tax_amounts(&mut line).unwrap();
        assert_eq!(line["tax_amounts"][0]["amount"], 100);
        assert_eq!(line["tax_amounts"][0]["taxable_amount"], 1000);

        // A line without a tax rate has no tax amounts.
        let mut line = json!({"amount": 1000, "tax_rates": []});
        recompute_tax_amounts(&mut line).unwrap();
        assert_eq!(line["tax_amounts"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn rejects_a_non_draft_invoice_and_a_missing_lines_array() {
        assert!(ensure_draft("draft").is_ok());
        let err = ensure_draft("open").unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.error_type, "invalid_request_error");

        let err = request_lines(&json!({})).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
        assert_eq!(err.param.as_deref(), Some("lines"));
    }

    #[test]
    fn rejects_an_unknown_line_behavior() {
        assert!(ensure_behavior(None).is_ok());
        assert!(ensure_behavior(Some("delete")).is_ok());
        assert!(ensure_behavior(Some("unassign")).is_ok());
        let err = ensure_behavior(Some("archive")).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
    }

    #[test]
    fn rejects_a_line_update_that_changes_nothing() {
        assert!(ensure_updatable(&json!({"id": "il_1", "quantity": "2"})).is_ok());
        assert!(ensure_updatable(&json!({"id": "il_1", "amount": "250"})).is_ok());
        assert!(ensure_updatable(&json!({"id": "il_1", "description": "Revised"})).is_ok());
        assert!(ensure_updatable(&json!({"id": "il_1", "price": "price_1"})).is_ok());
        assert!(ensure_updatable(&json!({"id": "il_1", "tax_rate": {"percentage": "10"}})).is_ok());

        let err = ensure_updatable(&json!({"id": "il_1"})).unwrap_err();
        assert_eq!(err.error_type, "invalid_request_error");
    }
}
