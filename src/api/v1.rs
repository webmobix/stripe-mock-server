use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::AppState;
use crate::api::expand::expand_response;
use crate::api::{
    collect_form_list, collect_list_param, get_array, get_bool, get_i64, get_str, ok, paginate,
};
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

pub(crate) fn metadata_from(params: &Value) -> Value {
    match params.get("metadata") {
        Some(Value::Object(_)) => params["metadata"].clone(),
        _ => json!({}),
    }
}

/// Read a `currency` parameter and normalize it to a lower-case ISO code.
///
/// Absent means `None`, so a caller can tell "not given" from "given as usd".
pub(crate) fn parse_currency(params: &Value) -> Result<Option<String>, ApiError> {
    let Some(raw) = get_str(params, &["currency"]) else {
        return Ok(None);
    };
    let currency = raw.trim().to_ascii_lowercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::invalid_param(
            v1(),
            "currency",
            "Invalid currency: must be a three-letter ISO code",
        ));
    }
    Ok(Some(currency))
}

// ---------------------------------------------------------------------------
// Accounts
// ---------------------------------------------------------------------------

pub async fn create_account(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let now = time::now_epoch();
    let id = ids::account_id();
    let business_type =
        get_str(&params, &["business_type"]).unwrap_or_else(|| "company".to_string());
    let configuration = json!({
        "merchant": { "capabilities": { "card_payments": { "requested": true }, "transfers": { "requested": true } } }
    });
    let capabilities = json!({ "card_payments": "pending", "transfers": "pending" });
    let requirements = json!({
        "alternatives": [], "current_deadline": null, "currently_due": [], "disabled_reason": null,
        "eventually_due": [], "past_due": [], "pending_verification": [],
    });
    sqlx::query(
        "INSERT INTO accounts (id, created, updated, livemode, is_main, business_type, email, country, default_currency,
            identity, configuration, capabilities, requirements, defaults, details_submitted, charges_enabled, payouts_enabled, metadata)
         VALUES (?1, ?2, ?2, 0, 0, ?3, ?4, ?5, 'usd', '{}', ?6, ?7, ?8, '{}', 0, 0, 0, ?9)",
    )
    .bind(&id)
    .bind(now)
    .bind(&business_type)
    .bind(get_str(&params, &["email"]))
    .bind(get_str(&params, &["country"]).unwrap_or_else(|| "US".to_string()))
    .bind(configuration.to_string())
    .bind(capabilities.to_string())
    .bind(requirements.to_string())
    .bind(metadata_from(&params).to_string())
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let account: AccountRow = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(account.to_v1()))
}

pub async fn get_account(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    if !ctx.can_access_account(&id) {
        return Err(ApiError::not_found(v1(), format!("No such account: {id}")));
    }
    let account: Option<AccountRow> = sqlx::query_as("SELECT * FROM accounts WHERE id = ?1")
        .bind(&id)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let account =
        account.ok_or_else(|| ApiError::not_found(v1(), format!("No such account: {id}")))?;
    Ok(ok(account.to_v1()))
}

pub async fn list_accounts(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
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
    .map_err(ApiError::internal)?;
    let limit = get_i64(&params, &["limit"]);
    let (page, has_more) = paginate(
        accounts,
        limit,
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |a| a.id.clone(),
    );
    let data = page.iter().map(|a| a.to_v1()).collect();
    Ok(ok(list_envelope("/v1/accounts", data, has_more)))
}

pub async fn create_account_link(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let account =
        get_str(&params, &["account"]).ok_or_else(|| ApiError::missing_param(v1(), "account"))?;
    let exists: Option<String> = sqlx::query_scalar("SELECT id FROM accounts WHERE id = ?1")
        .bind(&account)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    if exists.is_none() || !ctx.can_access_account(&account) {
        return Err(ApiError::not_found(
            v1(),
            format!("No such account: {account}"),
        ));
    }
    let link_type = get_str(&params, &["type"]).unwrap_or_else(|| "account_onboarding".to_string());
    let refresh_url = get_str(&params, &["refresh_url"]);
    let return_url = get_str(&params, &["return_url"]);
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
    .map_err(ApiError::internal)?;

    let url = format!("{}/mock/connect/{}", st.config.base_url(), token);
    Ok(ok(json!({
        "id": ids::id("aclink_"),
        "object": "account_link",
        "account": account,
        "created": now,
        "expires_at": expires_at,
        "url": url,
    })))
}

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

pub async fn create_product(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let name = get_str(&params, &["name"]).ok_or_else(|| ApiError::missing_param(v1(), "name"))?;
    let id = ids::product_id();
    let now = time::now_epoch();
    let active = get_bool(&params, &["active"]).unwrap_or(true);
    sqlx::query(
        "INSERT INTO products (id, account_id, name, description, active, created, default_price, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(&name)
    .bind(get_str(&params, &["description"]))
    .bind(active as i64)
    .bind(now)
    .bind(metadata_from(&params).to_string())
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let product: ProductRow = sqlx::query_as("SELECT * FROM products WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(product.to_json()))
}

pub async fn get_product(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let product: Option<ProductRow> =
        sqlx::query_as("SELECT * FROM products WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let product =
        product.ok_or_else(|| ApiError::not_found(v1(), format!("No such product: {id}")))?;
    Ok(ok(product.to_json()))
}

pub async fn list_products(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let products: Vec<ProductRow> = sqlx::query_as(
        "SELECT * FROM products WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let products: Vec<ProductRow> = if let Some(active) = get_bool(&params, &["active"]) {
        products
            .into_iter()
            .filter(|p| (p.active != 0) == active)
            .collect()
    } else {
        products
    };
    let (page, has_more) = paginate(
        products,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |p| p.id.clone(),
    );
    let data = page.iter().map(|p| p.to_json()).collect();
    Ok(ok(list_envelope("/v1/products", data, has_more)))
}

pub async fn create_price(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let product_id =
        get_str(&params, &["product"]).ok_or_else(|| ApiError::missing_param(v1(), "product"))?;
    let product: Option<ProductRow> =
        sqlx::query_as("SELECT * FROM products WHERE id = ?1 AND account_id = ?2")
            .bind(&product_id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    if product.is_none() {
        return Err(ApiError::invalid_param(v1(), "product", "No such product"));
    }
    let currency = get_str(&params, &["currency"]).unwrap_or_else(|| "usd".to_string());
    let currency = currency.trim().to_ascii_lowercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::invalid_param(
            v1(),
            "currency",
            "Invalid currency: must be a three-letter ISO code",
        ));
    }
    let unit_amount = match get_str(&params, &["unit_amount"]) {
        None => None,
        Some(raw) => match raw.parse::<i64>() {
            Ok(amount) if amount >= 0 => Some(amount),
            _ => {
                return Err(ApiError::invalid_param(
                    v1(),
                    "unit_amount",
                    "Invalid non-negative integer",
                ));
            }
        },
    };
    let recurring_interval = get_str(&params, &["recurring", "interval"]);
    let price_type = if recurring_interval.is_some() {
        "recurring"
    } else {
        "one_time"
    };
    let lookup_key = get_str(&params, &["lookup_key"]).filter(|key| !key.is_empty());
    let transfer_lookup_key = get_bool(&params, &["transfer_lookup_key"]).unwrap_or(false);
    let id = ids::price_id();
    let now = time::now_epoch();

    // Lookup keys are unique among the account's active prices. A transfer
    // clears the previous holder inside the same transaction as the insert.
    let mut previous_holder = None;
    if let Some(key) = &lookup_key {
        previous_holder = sqlx::query_scalar::<_, String>(
            "SELECT id FROM prices WHERE account_id = ?1 AND lookup_key = ?2 AND active = 1 LIMIT 1",
        )
        .bind(&ctx.account_id)
        .bind(key)
        .fetch_optional(&st.pool)
        .await
        .map_err(ApiError::internal)?;
        if previous_holder.is_some() && !transfer_lookup_key {
            return Err(ApiError::invalid_param(
                v1(),
                "lookup_key",
                format!("Lookup key {key} is already in use"),
            ));
        }
    }

    let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
    if let Some(previous) = &previous_holder {
        sqlx::query("UPDATE prices SET lookup_key = NULL WHERE id = ?1")
            .bind(previous)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
    }
    sqlx::query(
        "INSERT INTO prices (id, account_id, product_id, currency, unit_amount, price_type, recurring_interval,
            recurring_interval_count, nickname, active, created, metadata, lookup_key)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11, ?12)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(&product_id)
    .bind(&currency)
    .bind(unit_amount)
    .bind(price_type)
    .bind(&recurring_interval)
    .bind(get_i64(&params, &["recurring", "interval_count"]))
    .bind(get_str(&params, &["nickname"]))
    .bind(now)
    .bind(metadata_from(&params).to_string())
    .bind(&lookup_key)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::internal)?;

    sqlx::query("UPDATE products SET default_price = ?1 WHERE id = ?2 AND default_price IS NULL")
        .bind(&id)
        .bind(&product_id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
    tx.commit().await.map_err(ApiError::internal)?;

    let price: PriceRow = sqlx::query_as("SELECT * FROM prices WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        price.to_json(),
        &collect_form_list(&params, "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn get_price(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let price: Option<PriceRow> =
        sqlx::query_as("SELECT * FROM prices WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let price = price.ok_or_else(|| ApiError::not_found(v1(), format!("No such price: {id}")))?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        price.to_json(),
        &collect_list_param(query.as_deref().unwrap_or(""), "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn list_prices(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let product_filter = get_str(&params, &["product"]);
    let prices: Vec<PriceRow> = sqlx::query_as(
        "SELECT * FROM prices WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let prices: Vec<PriceRow> = match product_filter {
        Some(product) => prices
            .into_iter()
            .filter(|p| p.product_id == product)
            .collect(),
        None => prices,
    };
    let lookup_keys = collect_list_param(query.as_deref().unwrap_or(""), "lookup_keys");
    let prices: Vec<PriceRow> = if lookup_keys.is_empty() {
        prices
    } else {
        prices
            .into_iter()
            .filter(|p| {
                p.lookup_key
                    .as_deref()
                    .is_some_and(|key| lookup_keys.iter().any(|filter| filter == key))
            })
            .collect()
    };
    let (page, has_more) = paginate(
        prices,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |p| p.id.clone(),
    );
    let expands = collect_list_param(query.as_deref().unwrap_or(""), "expand");
    let mut data = Vec::new();
    for price in &page {
        data.push(expand_response(&st.pool, &ctx.account_id, price.to_json(), &expands).await);
    }
    Ok(ok(list_envelope("/v1/prices", data, has_more)))
}

// ---------------------------------------------------------------------------
// Billing
// ---------------------------------------------------------------------------

/// Parse `tax_id_data[]` entries into `tax_id` objects.
///
/// An entry needs both a `type` and a `value`; a partial entry is rejected
/// with the name of the missing field.
fn parse_tax_id_data(params: &Value, created: i64) -> Result<Vec<Value>, ApiError> {
    let raw = get_array(params, &["tax_id_data"])
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for entry in &raw {
        let tax_type = get_str(entry, &["type"])
            .ok_or_else(|| ApiError::missing_param(v1(), "tax_id_data[][type]"))?;
        let value = get_str(entry, &["value"])
            .ok_or_else(|| ApiError::missing_param(v1(), "tax_id_data[][value]"))?;
        out.push(crate::models::tax_id_entry(
            &ids::tax_id(),
            &tax_type,
            &value,
            created,
        ));
    }
    Ok(out)
}

pub async fn create_customer(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let id = ids::customer_id();
    let now = time::now_epoch();
    let tax_ids = parse_tax_id_data(&params, now)?;
    let currency = parse_currency(&params)?;

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query("INSERT INTO customers (id, account_id, email, name, currency, created, tax_ids, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
            .bind(&id)
            .bind(&ctx.account_id)
            .bind(get_str(&params, &["email"]))
            .bind(get_str(&params, &["name"]))
            .bind(&currency)
            .bind(now)
            .bind(serde_json::to_string(&tax_ids).unwrap_or_else(|_| "[]".to_string()))
            .bind(metadata_from(&params).to_string())
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let customer: CustomerRow = sqlx::query_as("SELECT * FROM customers WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "customer.created",
            customer.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let customer: CustomerRow = sqlx::query_as("SELECT * FROM customers WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(customer.to_json()))
}

pub async fn get_customer(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let customer: Option<CustomerRow> =
        sqlx::query_as("SELECT * FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let customer =
        customer.ok_or_else(|| ApiError::not_found(v1(), format!("No such customer: {id}")))?;
    Ok(ok(customer.to_json()))
}

pub async fn list_customers(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let customers: Vec<CustomerRow> = sqlx::query_as(
        "SELECT * FROM customers WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let email = get_str(&params, &["email"]);
    let customers: Vec<CustomerRow> = customers
        .into_iter()
        .filter(|c| match &email {
            Some(filter) => c.email.as_deref() == Some(filter.as_str()),
            None => true,
        })
        .collect();
    let (page, has_more) = paginate(
        customers,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |c| c.id.clone(),
    );
    let data = page.iter().map(|c| c.to_json()).collect();
    Ok(ok(list_envelope("/v1/customers", data, has_more)))
}

pub async fn update_customer(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let existing: Option<CustomerRow> =
        sqlx::query_as("SELECT * FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let existing =
        existing.ok_or_else(|| ApiError::not_found(v1(), format!("No such customer: {id}")))?;

    let now = time::now_epoch();
    // Only the fields the request supplied are changed.
    let email = params
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(existing.email.clone());
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(existing.name.clone());
    // The customer's currency only changes when the request names one.
    let currency = parse_currency(&params)?.or(existing.currency.clone());
    let metadata = match params.get("metadata") {
        Some(_) => metadata_from(&params).to_string(),
        None => existing
            .metadata
            .clone()
            .unwrap_or_else(|| "{}".to_string()),
    };
    // A tax id update appends to the existing entries.
    let mut tax_ids = existing.tax_id_list();
    tax_ids.extend(parse_tax_id_data(&params, now)?);

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query(
            "UPDATE customers SET email = ?1, name = ?2, currency = ?3, metadata = ?4, tax_ids = ?5 WHERE id = ?6",
        )
        .bind(&email)
        .bind(&name)
        .bind(&currency)
        .bind(&metadata)
        .bind(serde_json::to_string(&tax_ids).unwrap_or_else(|_| "[]".to_string()))
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        let customer: CustomerRow = sqlx::query_as("SELECT * FROM customers WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "customer.updated",
            customer.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let customer: CustomerRow = sqlx::query_as("SELECT * FROM customers WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(customer.to_json()))
}

pub async fn delete_customer(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let existing: Option<CustomerRow> =
        sqlx::query_as("SELECT * FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let existing =
        existing.ok_or_else(|| ApiError::not_found(v1(), format!("No such customer: {id}")))?;

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query("DELETE FROM customers WHERE id = ?1")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "customer.deleted",
            existing.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    // Stripe returns a deleted tombstone, not the full customer.
    Ok(ok(json!({
        "id": existing.id,
        "object": "customer",
        "deleted": true,
    })))
}

pub(crate) async fn load_price(
    st: &AppState,
    account_id: &str,
    price_id: &str,
    param: &str,
) -> Result<PriceRow, ApiError> {
    let price: Option<PriceRow> =
        sqlx::query_as("SELECT * FROM prices WHERE id = ?1 AND account_id = ?2")
            .bind(price_id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    price.ok_or_else(|| ApiError::invalid_param(v1(), param, format!("No such price: {price_id}")))
}

/// Load a product owned by the account.
pub(crate) async fn load_product(
    st: &AppState,
    account_id: &str,
    product_id: &str,
    param: &str,
) -> Result<ProductRow, ApiError> {
    let product: Option<ProductRow> =
        sqlx::query_as("SELECT * FROM products WHERE id = ?1 AND account_id = ?2")
            .bind(product_id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    product.ok_or_else(|| {
        ApiError::invalid_param(v1(), param, format!("No such product: {product_id}"))
    })
}

/// Read an optional `quantity`: absent means 1, anything else must be a positive integer.
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

fn build_subscription_item(
    price: &PriceRow,
    quantity: i64,
    subscription_id: &str,
    period_start: i64,
    period_end: i64,
) -> Value {
    json!({
        "id": ids::id("si_"),
        "object": "subscription_item",
        "created": time::now_epoch(),
        "current_period_start": period_start,
        "current_period_end": period_end,
        "price": price.to_json(),
        "quantity": quantity,
        "subscription": subscription_id,
    })
}

pub async fn create_subscription(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let customer = get_str(&params, &["customer"]);
    let customer_account = get_str(&params, &["customer_account"]);
    if customer.is_none() && customer_account.is_none() {
        return Err(ApiError::invalid_param(
            v1(),
            "customer",
            "A customer or customer_account is required",
        ));
    }
    let items = get_array(&params, &["items"]).cloned().unwrap_or_default();
    if items.is_empty() {
        return Err(ApiError::missing_param(v1(), "items"));
    }
    let id = ids::subscription_id();
    let now = time::now_epoch();
    let period_end = now + 30 * 86_400;
    let mut built_items = Vec::new();
    for item in &items {
        let price_id = get_str(item, &["price"])
            .ok_or_else(|| ApiError::missing_param(v1(), "items[][price]"))?;
        let quantity = parse_quantity(item)?;
        let price = load_price(&st, &ctx.account_id, &price_id, "items[0][price]").await?;
        built_items.push(build_subscription_item(&price, quantity, &id, now, period_end));
    }

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query(
            "INSERT INTO subscriptions (id, account_id, customer, customer_account, status, created, current_period_start,
                current_period_end, cancel_at_period_end, items, metadata)
             VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5, ?6, 0, ?7, ?8)",
        )
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(&customer)
        .bind(&customer_account)
        .bind(now)
        .bind(period_end)
        .bind(serde_json::to_string(&built_items).unwrap())
        .bind(metadata_from(&params).to_string())
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;

        let subscription: SubscriptionRow =
            sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "customer.subscription.created",
            subscription.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let subscription: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        subscription.to_json(),
        &collect_form_list(&params, "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn get_subscription(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let subscription: Option<SubscriptionRow> =
        sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let subscription = subscription
        .ok_or_else(|| ApiError::not_found(v1(), format!("No such subscription: {id}")))?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        subscription.to_json(),
        &collect_list_param(query.as_deref().unwrap_or(""), "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn list_subscriptions(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let subscriptions: Vec<SubscriptionRow> = sqlx::query_as(
        "SELECT * FROM subscriptions WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let customer = get_str(&params, &["customer"]);
    let customer_account = get_str(&params, &["customer_account"]);
    let subscriptions: Vec<SubscriptionRow> = subscriptions
        .into_iter()
        .filter(|s| match &customer {
            Some(c) => s.customer.as_deref() == Some(c.as_str()),
            None => true,
        })
        .filter(|s| match &customer_account {
            Some(c) => s.customer_account.as_deref() == Some(c.as_str()),
            None => true,
        })
        .collect();
    let (page, has_more) = paginate(
        subscriptions,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |s| s.id.clone(),
    );
    let expands = collect_list_param(query.as_deref().unwrap_or(""), "expand");
    let mut data = Vec::new();
    for subscription in &page {
        data.push(
            expand_response(
                &st.pool,
                &ctx.account_id,
                subscription.to_json(),
                &expands,
            )
            .await,
        );
    }
    Ok(ok(list_envelope("/v1/subscriptions", data, has_more)))
}

pub async fn cancel_subscription(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let subscription: Option<SubscriptionRow> =
        sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let subscription = subscription
        .ok_or_else(|| ApiError::not_found(v1(), format!("No such subscription: {id}")))?;
    if subscription.status == "canceled" {
        return Err(ApiError::not_found(
            v1(),
            format!("No such subscription: {id}"),
        ));
    }

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query("UPDATE subscriptions SET status = 'canceled' WHERE id = ?1")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let updated: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "customer.subscription.deleted",
            updated.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let updated: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(updated.to_json()))
}

/// The prorated amount for a quantity change over the remaining period.
///
/// `amount = round(unit_amount * quantity_delta * remaining / period_length)`,
/// with `remaining` clamped to the period and rounding half away from zero.
fn proration_amount(
    unit_amount: i64,
    quantity_delta: i64,
    period_start: i64,
    period_end: i64,
    now: i64,
) -> i64 {
    let period_length = (period_end - period_start).max(0);
    if period_length == 0 {
        return 0;
    }
    let remaining = (period_end - now).clamp(0, period_length);
    let raw = unit_amount as f64 * quantity_delta as f64 * remaining as f64 / period_length as f64;
    if raw >= 0.0 {
        (raw + 0.5).floor() as i64
    } else {
        (raw - 0.5).ceil() as i64
    }
}

/// Update a subscription's item quantities, cancellation flag, or metadata.
///
/// Item entries are matched by `id`, or by `price` when the subscription has
/// exactly one item. Proration figures are returned on the response and are
/// never persisted or invoiced.
pub async fn update_subscription(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let proration_behavior = get_str(&params, &["proration_behavior"])
        .unwrap_or_else(|| "create_prorations".to_string());
    if !["create_prorations", "none", "always_invoice"].contains(&proration_behavior.as_str()) {
        return Err(ApiError::invalid_param(
            v1(),
            "proration_behavior",
            "Invalid proration_behavior: must be create_prorations, none, or always_invoice",
        ));
    }

    let subscription: Option<SubscriptionRow> =
        sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let subscription = subscription
        .ok_or_else(|| ApiError::not_found(v1(), format!("No such subscription: {id}")))?;

    let requested_items = get_array(&params, &["items"]).cloned().unwrap_or_default();
    let mut stored_items = parse_json(&Some(subscription.items.clone()))
        .as_array()
        .cloned()
        .unwrap_or_default();
    let now = time::now_epoch();
    let period_start = subscription
        .current_period_start
        .unwrap_or(subscription.created);
    let period_end = subscription.current_period_end.unwrap_or(now);
    let mut prorations = Vec::new();

    for (index, requested) in requested_items.iter().enumerate() {
        let quantity = get_str(requested, &["quantity"])
            .and_then(|raw| raw.trim().parse::<i64>().ok())
            .filter(|quantity| *quantity > 0)
            .ok_or_else(|| {
                ApiError::invalid_param(
                    v1(),
                    "quantity",
                    "Invalid quantity: must be a positive integer",
                )
            })?;
        let requested_item_id = get_str(requested, &["id"]);
        let requested_price_id = get_str(requested, &["price"]);

        if let Some(price_id) = &requested_price_id {
            // Rejects an unknown price for the account before any mutation.
            load_price(&st, &ctx.account_id, price_id, "items[0][price]").await?;
        }

        let position = if let Some(item_id) = &requested_item_id {
            stored_items
                .iter()
                .position(|item| get_str(item, &["id"]).as_deref() == Some(item_id.as_str()))
                .ok_or_else(|| {
                    ApiError::invalid_param(
                        v1(),
                        &format!("items[{index}][id]"),
                        format!("No such subscription item: {item_id}"),
                    )
                })?
        } else if stored_items.len() == 1 {
            if let Some(price_id) = &requested_price_id {
                let item_price = get_str(&stored_items[0], &["price", "id"]);
                if item_price.as_deref() != Some(price_id.as_str()) {
                    return Err(ApiError::invalid_param(
                        v1(),
                        &format!("items[{index}][price]"),
                        format!("No such subscription item for price: {price_id}"),
                    ));
                }
            }
            0
        } else {
            return Err(ApiError::invalid_param(
                v1(),
                &format!("items[{index}][id]"),
                "An item id is required when the subscription has more than one item",
            ));
        };

        if let Some(price_id) = &requested_price_id {
            let item_price = get_str(&stored_items[position], &["price", "id"]);
            if item_price.as_deref() != Some(price_id.as_str()) {
                return Err(ApiError::invalid_param(
                    v1(),
                    &format!("items[{index}][price]"),
                    format!("No such subscription item for price: {price_id}"),
                ));
            }
        }

        let item_id = get_str(&stored_items[position], &["id"]).unwrap_or_default();
        let item_price_id =
            get_str(&stored_items[position], &["price", "id"]).unwrap_or_default();
        let unit_amount = get_i64(&stored_items[position], &["price", "unit_amount"]).unwrap_or(0);
        let currency = get_str(&stored_items[position], &["price", "currency"])
            .unwrap_or_else(|| "usd".to_string());
        let previous_quantity = get_i64(&stored_items[position], &["quantity"]).unwrap_or(1);
        let quantity_delta = quantity - previous_quantity;

        if let Value::Object(map) = &mut stored_items[position] {
            map.insert("quantity".into(), json!(quantity));
            map.entry("subscription")
                .or_insert_with(|| json!(subscription.id.clone()));
            map.entry("current_period_start")
                .or_insert(json!(subscription.current_period_start));
            map.entry("current_period_end")
                .or_insert(json!(subscription.current_period_end));
        }

        if quantity_delta != 0 && proration_behavior != "none" {
            let amount = proration_amount(
                unit_amount,
                quantity_delta,
                period_start,
                period_end,
                now,
            );
            prorations.push(json!({
                "id": ids::id("pror_"),
                "object": "proration",
                "item": item_id,
                "price": item_price_id,
                "quantity_delta": quantity_delta,
                "amount": amount,
                "currency": currency,
                "period_start": period_start,
                "period_end": period_end,
            }));
        }
    }

    let cancel_at_period_end = get_bool(&params, &["cancel_at_period_end"])
        .map(i64::from)
        .unwrap_or(subscription.cancel_at_period_end);
    let metadata = if params.get("metadata").is_some() {
        metadata_from(&params).to_string()
    } else {
        subscription
            .metadata
            .clone()
            .unwrap_or_else(|| "{}".to_string())
    };

    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        sqlx::query(
            "UPDATE subscriptions SET items = ?1, cancel_at_period_end = ?2, metadata = ?3 WHERE id = ?4",
        )
        .bind(serde_json::to_string(&stored_items).unwrap_or_else(|_| "[]".to_string()))
        .bind(cancel_at_period_end)
        .bind(&metadata)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        let updated: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "customer.subscription.updated",
            updated.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let updated: SubscriptionRow = sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let mut value = expand_response(
        &st.pool,
        &ctx.account_id,
        updated.to_json(),
        &collect_form_list(&params, "expand"),
    )
    .await;
    if let Value::Object(map) = &mut value {
        map.insert("prorations".into(), Value::Array(prorations));
    }
    Ok(ok(value))
}

// ---------------------------------------------------------------------------
// Checkout
// ---------------------------------------------------------------------------

fn build_line_item(price: &PriceRow, quantity: i64) -> Value {
    json!({
        "id": ids::id("li_"),
        "object": "item",
        "amount_total": price.unit_amount.unwrap_or(0).saturating_mul(quantity),
        "currency": price.currency,
        "description": price.nickname,
        "price": price.to_json(),
        "quantity": quantity,
    })
}

/// Round a monetary value to the nearest minor unit, halves away from zero.
fn round_money(raw: f64) -> i64 {
    if raw >= 0.0 {
        (raw + 0.5).floor() as i64
    } else {
        (raw - 0.5).ceil() as i64
    }
}

/// Apply coupons sequentially against the remaining subtotal, stopping at zero.
fn coupon_discounts(coupons: &[CouponRow], subtotal: i64) -> Vec<i64> {
    let mut remaining = subtotal.max(0);
    coupons
        .iter()
        .map(|coupon| {
            let requested = match (coupon.percent_off, coupon.amount_off) {
                (Some(percentage), _) => round_money(remaining as f64 * percentage / 100.0),
                (None, Some(amount)) => amount,
                (None, None) => 0,
            };
            let discount = requested.clamp(0, remaining);
            remaining -= discount;
            discount
        })
        .collect()
}

/// Allocate a total discount across lines in proportion to their amounts,
/// assigning any rounding remainder to the largest lines (ties by order).
fn allocate_discount(line_amounts: &[i64], discount: i64) -> Vec<i64> {
    let subtotal: i64 = line_amounts.iter().copied().sum();
    if subtotal <= 0 || discount <= 0 {
        return vec![0; line_amounts.len()];
    }
    let discount = discount.min(subtotal);
    let mut allocated: Vec<i64> = line_amounts
        .iter()
        .map(|amount| {
            let share = discount as i128 * *amount as i128 / subtotal as i128;
            share as i64
        })
        .collect();
    let mut remainder = discount - allocated.iter().copied().sum::<i64>();
    let mut order: Vec<usize> = (0..line_amounts.len()).collect();
    order.sort_by_key(|index| (std::cmp::Reverse(line_amounts[*index]), *index));
    for index in order {
        if remainder <= 0 {
            break;
        }
        allocated[index] += 1;
        remainder -= 1;
    }
    allocated
}

/// Tax on a (discounted) line amount. Inclusive tax is extracted from the
/// amount; exclusive tax is added on top.
fn tax_amount(amount: i64, percentage: f64, inclusive: bool) -> i64 {
    if inclusive {
        round_money(amount as f64 * percentage / (100.0 + percentage))
    } else {
        round_money(amount as f64 * percentage / 100.0)
    }
}

/// Load an active tax rate owned by the account.
async fn load_tax_rate(
    st: &AppState,
    account_id: &str,
    tax_rate_id: &str,
    param: &str,
) -> Result<TaxRateRow, ApiError> {
    let tax_rate: Option<TaxRateRow> =
        sqlx::query_as("SELECT * FROM tax_rates WHERE id = ?1 AND account_id = ?2")
            .bind(tax_rate_id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let tax_rate = tax_rate.ok_or_else(|| {
        ApiError::invalid_param(v1(), param, format!("No such tax rate: {tax_rate_id}"))
    })?;
    if tax_rate.active == 0 {
        return Err(ApiError::invalid_param(
            v1(),
            param,
            format!("Tax rate {tax_rate_id} is inactive"),
        ));
    }
    Ok(tax_rate)
}

pub async fn create_checkout_session(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let mode = get_str(&params, &["mode"]).ok_or_else(|| ApiError::missing_param(v1(), "mode"))?;
    if !["payment", "subscription", "setup"].contains(&mode.as_str()) {
        return Err(ApiError::invalid_param(v1(), "mode", "Invalid mode"));
    }
    let now = time::now_epoch();
    let raw_items = get_array(&params, &["line_items"])
        .cloned()
        .unwrap_or_default();
    let mut priced = Vec::new();
    let mut amount_subtotal = 0i64;
    let mut currency = "usd".to_string();
    for item in &raw_items {
        let price_id = get_str(item, &["price"])
            .ok_or_else(|| ApiError::missing_param(v1(), "line_items[][price]"))?;
        let quantity = parse_quantity(item)?;
        let price = load_price(&st, &ctx.account_id, &price_id, "items[0][price]").await?;
        let line_amount = price
            .unit_amount
            .unwrap_or(0)
            .checked_mul(quantity)
            .ok_or_else(|| ApiError::invalid_param(v1(), "line_items", "Amount too large"))?;
        amount_subtotal = amount_subtotal
            .checked_add(line_amount)
            .ok_or_else(|| ApiError::invalid_param(v1(), "line_items", "Amount too large"))?;
        currency = price.currency.clone();
        priced.push((price, quantity, line_amount));
    }
    let line_amounts: Vec<i64> = priced.iter().map(|(_, _, amount)| *amount).collect();
    let product_ids: Vec<String> = priced
        .iter()
        .map(|(price, _, _)| price.product_id.clone())
        .collect();

    // Validate and resolve coupons in request order.
    let coupon_ids: Vec<String> = get_array(&params, &["discounts"])
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| get_str(entry, &["coupon"]))
                .collect()
        })
        .unwrap_or_default();
    let mut coupons = Vec::new();
    for (index, coupon_id) in coupon_ids.iter().enumerate() {
        let coupon: Option<CouponRow> =
            sqlx::query_as("SELECT * FROM coupons WHERE id = ?1 AND account_id = ?2")
                .bind(coupon_id)
                .bind(&ctx.account_id)
                .fetch_optional(&st.pool)
                .await
                .map_err(ApiError::internal)?;
        let coupon = coupon.ok_or_else(|| {
            ApiError::invalid_param(
                v1(),
                &format!("discounts[{index}][coupon]"),
                format!("No such coupon: {coupon_id}"),
            )
        })?;
        if !coupon.is_valid(now) {
            return Err(ApiError::invalid_request(
                v1(),
                format!("Coupon {coupon_id} is not valid"),
            ));
        }
        if coupon.amount_off.is_some()
            && coupon.currency.as_deref() != Some(currency.as_str())
        {
            return Err(ApiError::invalid_request(
                v1(),
                format!(
                    "Coupon {coupon_id} is in {} but the session is in {currency}",
                    coupon.currency.as_deref().unwrap_or("an unknown currency")
                ),
            ));
        }
        let restricted = coupon.applies_to_products();
        if !restricted.is_empty()
            && product_ids
                .iter()
                .any(|product| !restricted.contains(product))
        {
            return Err(ApiError::invalid_request(
                v1(),
                format!("Coupon {coupon_id} does not apply to every line item product"),
            ));
        }
        coupons.push(coupon);
    }
    let discount_amounts = coupon_discounts(&coupons, amount_subtotal);
    let amount_discount: i64 = discount_amounts.iter().sum();
    let line_discounts = allocate_discount(&line_amounts, amount_discount);

    // Session-level tax rates apply unless a line names its own.
    let session_tax_ids = collect_form_list(&params, "tax_rates");
    let mut session_rates = Vec::new();
    for tax_rate_id in &session_tax_ids {
        session_rates.push(load_tax_rate(&st, &ctx.account_id, tax_rate_id, "tax_rates").await?);
    }

    let mut line_items = Vec::new();
    let mut amount_tax = 0i64;
    let mut exclusive_tax = 0i64;
    for (index, (price, quantity, line_amount)) in priced.iter().enumerate() {
        let discounted = line_amount.saturating_sub(line_discounts[index]);
        let line_tax_ids = collect_form_list(&raw_items[index], "tax_rates");
        let mut rates = Vec::new();
        if line_tax_ids.is_empty() {
            rates.clone_from(&session_rates);
        } else {
            for tax_rate_id in &line_tax_ids {
                rates.push(
                    load_tax_rate(
                        &st,
                        &ctx.account_id,
                        tax_rate_id,
                        &format!("line_items[{index}][tax_rates]"),
                    )
                    .await?,
                );
            }
        }
        let mut tax_rate_values = Vec::new();
        let mut tax_amounts = Vec::new();
        for rate in &rates {
            let tax = tax_amount(discounted, rate.percentage, rate.inclusive != 0);
            amount_tax += tax;
            if rate.inclusive == 0 {
                exclusive_tax += tax;
            }
            tax_rate_values.push(rate.to_json());
            tax_amounts.push(json!({
                "amount": tax,
                "taxable_amount": discounted,
                "tax_rate": rate.to_json(),
            }));
        }
        let mut line = build_line_item(price, *quantity);
        line["tax_rates"] = Value::Array(tax_rate_values);
        line["tax_amounts"] = Value::Array(tax_amounts);
        line_items.push(line);
    }
    let amount_total = amount_subtotal
        .saturating_sub(amount_discount)
        .saturating_add(exclusive_tax);
    let discount_snapshots: Vec<Value> = coupons
        .iter()
        .zip(discount_amounts.iter())
        .map(|(coupon, amount)| {
            json!({
                "id": ids::id("di_"),
                "amount": amount,
                "coupon": coupon.to_json(),
            })
        })
        .collect();

    let id = ids::checkout_session_id();
    let expires_at = now + 86_400;
    let url = format!("{}/mock/checkout/{}", st.config.base_url(), id);
    let success_url = get_str(&params, &["success_url"]);
    let cancel_url = get_str(&params, &["cancel_url"]);
    let supplied_customer = get_str(&params, &["customer"]);
    let customer_account = get_str(&params, &["customer_account"]);
    let supplied_email = get_str(&params, &["customer_email"]);

    let mut event_ids = Vec::new();
    {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;

        // An email-only session immediately gains a linked customer.
        let customer = if let Some(customer) = supplied_customer {
            Some(customer)
        } else if supplied_email.is_some() && customer_account.is_none() {
            let customer_id = ids::customer_id();
            sqlx::query(
                "INSERT INTO customers (id, account_id, email, name, currency, created, tax_ids, metadata)
                 VALUES (?1, ?2, ?3, NULL, NULL, ?4, '[]', '{}')",
            )
            .bind(&customer_id)
            .bind(&ctx.account_id)
            .bind(&supplied_email)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
            let created: CustomerRow = sqlx::query_as("SELECT * FROM customers WHERE id = ?1")
                .bind(&customer_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            event_ids.push(
                events::emit(
                    &mut tx,
                    &ctx.account_id,
                    ctx.is_connected,
                    "customer.created",
                    created.to_json(),
                )
                .await
                .map_err(ApiError::internal)?,
            );
            Some(customer_id)
        } else {
            None
        };

        // Fall back to the linked customer's email when the request omits it.
        let customer_email = match supplied_email {
            Some(email) => Some(email),
            None => match &customer {
                Some(customer_id) => sqlx::query_scalar::<_, Option<String>>(
                    "SELECT email FROM customers WHERE id = ?1 AND account_id = ?2",
                )
                .bind(customer_id)
                .bind(&ctx.account_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(ApiError::internal)?
                .flatten(),
                None => None,
            },
        };

        sqlx::query(
            "INSERT INTO checkout_sessions (id, account_id, mode, status, created, expires_at, success_url, cancel_url,
                customer, customer_account, customer_email, line_items, payment_intent, subscription, amount_subtotal,
                amount_total, amount_discount, amount_tax, discounts, currency, url, metadata)
             VALUES (?1, ?2, ?3, 'open', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        )
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(&mode)
        .bind(now)
        .bind(expires_at)
        .bind(&success_url)
        .bind(&cancel_url)
        .bind(&customer)
        .bind(&customer_account)
        .bind(&customer_email)
        .bind(serde_json::to_string(&line_items).unwrap())
        .bind(amount_subtotal)
        .bind(amount_total)
        .bind(amount_discount)
        .bind(amount_tax)
        .bind(serde_json::to_string(&discount_snapshots).unwrap_or_else(|_| "[]".to_string()))
        .bind(&currency)
        .bind(&url)
        .bind(metadata_from(&params).to_string())
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;

        tx.commit().await.map_err(ApiError::internal)?;
    }
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    for event_id in event_ids {
        let _ = st.dispatcher.dispatch_event(&event_id).await;
    }

    let session: CheckoutSessionRow =
        sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1")
            .bind(&id)
            .fetch_one(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        session.to_json(),
        &collect_form_list(&params, "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn get_checkout_session(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let session = load_session(&st, &ctx.account_id, &id).await?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        session.to_json(),
        &collect_list_param(query.as_deref().unwrap_or(""), "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn get_checkout_line_items(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let session = load_session(&st, &ctx.account_id, &id).await?;
    let value = list_envelope(
        &format!("/v1/checkout/sessions/{id}/line_items"),
        session.line_items(),
        false,
    );
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        value,
        &collect_list_param(query.as_deref().unwrap_or(""), "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn expire_checkout_session(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let session = load_session(&st, &ctx.account_id, &id).await?;
    if session.status != "open" {
        return Err(ApiError::invalid_request(
            v1(),
            "Only open sessions can be expired",
        ));
    }
    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        let claimed = sqlx::query(
            "UPDATE checkout_sessions SET status = 'expired' WHERE id = ?1 AND status = 'open'",
        )
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if claimed.rows_affected() == 0 {
            return Err(ApiError::invalid_request(
                v1(),
                "Only open sessions can be expired",
            ));
        }
        let session: CheckoutSessionRow =
            sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1")
                .bind(&id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "checkout.session.expired",
            session.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    let _ = st.dispatcher.dispatch_event(&event_id).await;
    let session = load_session(&st, &ctx.account_id, &id).await?;
    Ok(ok(session.to_json()))
}

pub async fn load_session(
    st: &AppState,
    account_id: &str,
    id: &str,
) -> Result<CheckoutSessionRow, ApiError> {
    let session: Option<CheckoutSessionRow> =
        sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    session.ok_or_else(|| ApiError::not_found(v1(), format!("No such checkout session: {id}")))
}

/// Complete a checkout session transactionally, creating payment intents and/or
/// subscriptions plus their events. Used by the mock checkout page.
pub async fn complete_checkout_session(
    st: &AppState,
    session_id: &str,
) -> Result<CheckoutSessionRow, ApiError> {
    let session: CheckoutSessionRow =
        sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1")
            .bind(session_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?
            .ok_or_else(|| {
                ApiError::not_found(v1(), format!("No such checkout session: {session_id}"))
            })?;
    if session.status != "open" {
        return Err(ApiError::invalid_request(v1(), "Session is not open"));
    }
    if session.expires_at < time::now_epoch() {
        sqlx::query(
            "UPDATE checkout_sessions SET status = 'expired' WHERE id = ?1 AND status = 'open'",
        )
        .bind(session_id)
        .execute(&st.pool)
        .await
        .map_err(ApiError::internal)?;
        return Err(ApiError::invalid_request(v1(), "Session has expired"));
    }
    let account_id = session.account_id.clone();
    let is_connected: i64 = sqlx::query_scalar("SELECT is_main FROM accounts WHERE id = ?1")
        .bind(&account_id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let is_connected = is_connected == 0;

    let mut event_ids = Vec::new();
    let payment_intent_id_value = if session.mode == "payment" {
        Some(ids::payment_intent_id())
    } else {
        None
    };
    let subscription_id_value = if session.mode == "subscription" {
        Some(ids::subscription_id())
    } else {
        None
    };

    {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;

        // Claim the session first so concurrent completions cannot both proceed.
        let claimed = sqlx::query(
            "UPDATE checkout_sessions SET status = 'complete' WHERE id = ?1 AND status = 'open'",
        )
        .bind(&session.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;
        if claimed.rows_affected() == 0 {
            return Err(ApiError::invalid_request(v1(), "Session is not open"));
        }

        // The claim gates redemption, so a concurrent completion increments once.
        for discount in session.discounts() {
            if let Some(coupon_id) = get_str(&discount, &["coupon", "id"]) {
                sqlx::query(
                    "UPDATE coupons SET times_redeemed = times_redeemed + 1 WHERE id = ?1 AND account_id = ?2",
                )
                .bind(&coupon_id)
                .bind(&account_id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            }
        }

        if let Some(pi_id) = &payment_intent_id_value {
            sqlx::query(
                "INSERT INTO payment_intents (id, account_id, amount, currency, status, customer, created, checkout_session, metadata)
                 VALUES (?1, ?2, ?3, ?4, 'succeeded', ?5, ?6, ?7, '{}')",
            )
            .bind(pi_id)
            .bind(&account_id)
            .bind(session.amount_total)
            .bind(&session.currency)
            .bind(&session.customer)
            .bind(time::now_epoch())
            .bind(&session.id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
            let pi: PaymentIntentRow =
                sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1")
                    .bind(pi_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            event_ids.push(
                events::emit(
                    &mut tx,
                    &account_id,
                    is_connected,
                    "payment_intent.succeeded",
                    pi.to_json(),
                )
                .await
                .map_err(ApiError::internal)?,
            );
        }

        if let Some(sub_id) = &subscription_id_value {
            let now = time::now_epoch();
            let period_end = now + 30 * 86_400;
            let mut items = Vec::new();
            for item in session.line_items() {
                let price_id = get_str(&item, &["price", "id"]);
                let quantity = get_i64(&item, &["quantity"])
                    .filter(|q| *q >= 1)
                    .unwrap_or(1);
                if let Some(price_id) = price_id {
                    let price: Option<PriceRow> =
                        sqlx::query_as("SELECT * FROM prices WHERE id = ?1 AND account_id = ?2")
                            .bind(&price_id)
                            .bind(&account_id)
                            .fetch_optional(&mut *tx)
                            .await
                            .map_err(ApiError::internal)?;
                    if let Some(price) = price {
                        items.push(build_subscription_item(
                            &price, quantity, sub_id, now, period_end,
                        ));
                    }
                }
            }
            sqlx::query(
                "INSERT INTO subscriptions (id, account_id, customer, customer_account, status, created, current_period_start,
                    current_period_end, cancel_at_period_end, items, metadata)
                 VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5, ?6, 0, ?7, '{}')",
            )
            .bind(sub_id)
            .bind(&account_id)
            .bind(&session.customer)
            .bind(&session.customer_account)
            .bind(now)
            .bind(period_end)
            .bind(serde_json::to_string(&items).unwrap())
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
            let subscription: SubscriptionRow =
                sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
                    .bind(sub_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            event_ids.push(
                events::emit(
                    &mut tx,
                    &account_id,
                    is_connected,
                    "customer.subscription.created",
                    subscription.to_json(),
                )
                .await
                .map_err(ApiError::internal)?,
            );
        }

        sqlx::query(
            "UPDATE checkout_sessions SET payment_intent = ?1, subscription = ?2 WHERE id = ?3",
        )
        .bind(&payment_intent_id_value)
        .bind(&subscription_id_value)
        .bind(&session.id)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;

        let completed: CheckoutSessionRow =
            sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1")
                .bind(&session.id)
                .fetch_one(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        event_ids.push(
            events::emit(
                &mut tx,
                &account_id,
                is_connected,
                "checkout.session.completed",
                completed.to_json(),
            )
            .await
            .map_err(ApiError::internal)?,
        );

        tx.commit().await.map_err(ApiError::internal)?;
    }

    for event_id in event_ids {
        // The state change is already committed; a failed enqueue must not turn it into a 500.
        let _ = st.dispatcher.dispatch_event(&event_id).await;
    }

    load_session(st, &account_id, &session.id).await
}

// ---------------------------------------------------------------------------
// Coupons
// ---------------------------------------------------------------------------

fn parse_percent_off(params: &Value) -> Result<Option<f64>, ApiError> {
    let Some(raw) = get_str(params, &["percent_off"]) else {
        return Ok(None);
    };
    let percentage = raw.trim().parse::<f64>().ok().ok_or_else(|| {
        ApiError::invalid_param(v1(), "percent_off", "Invalid percent_off")
    })?;
    if !(percentage > 0.0 && percentage <= 100.0) {
        return Err(ApiError::invalid_param(
            v1(),
            "percent_off",
            "Invalid percent_off: must be greater than 0 and at most 100",
        ));
    }
    Ok(Some(percentage))
}

fn parse_amount_off(params: &Value) -> Result<Option<i64>, ApiError> {
    let Some(raw) = get_str(params, &["amount_off"]) else {
        return Ok(None);
    };
    raw.trim()
        .parse::<i64>()
        .ok()
        .filter(|amount| *amount > 0)
        .map(Some)
        .ok_or_else(|| {
            ApiError::invalid_param(
                v1(),
                "amount_off",
                "Invalid amount_off: must be a positive integer in minor units",
            )
        })
}

fn parse_applies_to_products(params: &Value) -> Vec<String> {
    get_array(params, &["applies_to", "products"])
        .map(|products| {
            products
                .iter()
                .filter_map(|product| product.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub async fn create_coupon(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);

    let percent_off = parse_percent_off(&params)?;
    let amount_off = parse_amount_off(&params)?;
    match (percent_off.is_some(), amount_off.is_some()) {
        (true, true) => {
            return Err(ApiError::invalid_request(
                v1(),
                "A coupon accepts exactly one of percent_off or amount_off",
            ));
        }
        (false, false) => {
            return Err(ApiError::invalid_request(
                v1(),
                "A coupon requires one of percent_off or amount_off",
            ));
        }
        _ => {}
    }

    let currency = parse_currency(&params)?;
    if amount_off.is_some() && currency.is_none() {
        return Err(ApiError::missing_param(v1(), "currency"));
    }

    let duration = get_str(&params, &["duration"]).unwrap_or_else(|| "once".to_string());
    if !["once", "forever", "repeating"].contains(&duration.as_str()) {
        return Err(ApiError::invalid_param(
            v1(),
            "duration",
            "Invalid duration: must be once, forever, or repeating",
        ));
    }
    let duration_in_months = get_i64(&params, &["duration_in_months"]);
    if duration == "repeating" && duration_in_months.is_none_or(|months| months <= 0) {
        return Err(ApiError::invalid_param(
            v1(),
            "duration_in_months",
            "A repeating coupon requires a positive duration_in_months",
        ));
    }

    let id = match get_str(&params, &["id"]).filter(|id| !id.is_empty()) {
        Some(id) => id,
        None => ids::coupon_id(),
    };
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM coupons WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    if existing.is_some() {
        return Err(ApiError::invalid_param(
            v1(),
            "id",
            format!("A coupon with id {id} already exists"),
        ));
    }

    let applies_to = json!({ "products": parse_applies_to_products(&params) });
    let now = time::now_epoch();
    sqlx::query(
        "INSERT INTO coupons (id, account_id, percent_off, amount_off, currency, duration, duration_in_months,
            max_redemptions, times_redeemed, redeem_by, name, applies_to, metadata, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(percent_off)
    .bind(amount_off)
    .bind(&currency)
    .bind(&duration)
    .bind(duration_in_months)
    .bind(get_i64(&params, &["max_redemptions"]))
    .bind(get_i64(&params, &["redeem_by"]))
    .bind(get_str(&params, &["name"]))
    .bind(applies_to.to_string())
    .bind(metadata_from(&params).to_string())
    .bind(now)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let coupon: CouponRow = sqlx::query_as("SELECT * FROM coupons WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(coupon.to_json()))
}

pub async fn get_coupon(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let coupon: Option<CouponRow> =
        sqlx::query_as("SELECT * FROM coupons WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let coupon = coupon.ok_or_else(|| ApiError::not_found(v1(), format!("No such coupon: {id}")))?;
    Ok(ok(coupon.to_json()))
}

pub async fn list_coupons(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let coupons: Vec<CouponRow> = sqlx::query_as(
        "SELECT * FROM coupons WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let (page, has_more) = paginate(
        coupons,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |coupon| coupon.id.clone(),
    );
    let data = page.iter().map(CouponRow::to_json).collect();
    Ok(ok(list_envelope("/v1/coupons", data, has_more)))
}

pub async fn delete_coupon(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let coupon: Option<CouponRow> =
        sqlx::query_as("SELECT * FROM coupons WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let coupon = coupon.ok_or_else(|| ApiError::not_found(v1(), format!("No such coupon: {id}")))?;
    sqlx::query("DELETE FROM coupons WHERE id = ?1")
        .bind(&id)
        .execute(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    // Stripe returns a deleted tombstone, not the full coupon.
    Ok(ok(json!({
        "id": coupon.id,
        "object": "coupon",
        "deleted": true,
    })))
}

// ---------------------------------------------------------------------------
// Tax rates
// ---------------------------------------------------------------------------

pub async fn create_tax_rate(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let display_name = get_str(&params, &["display_name"])
        .ok_or_else(|| ApiError::missing_param(v1(), "display_name"))?;
    let inclusive =
        get_bool(&params, &["inclusive"]).ok_or_else(|| ApiError::missing_param(v1(), "inclusive"))?;
    let percentage = get_str(&params, &["percentage"])
        .ok_or_else(|| ApiError::missing_param(v1(), "percentage"))?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|percentage| (0.0..=100.0).contains(percentage))
        .ok_or_else(|| {
            ApiError::invalid_param(
                v1(),
                "percentage",
                "Invalid percentage: must be between 0 and 100",
            )
        })?;
    let active = get_bool(&params, &["active"]).unwrap_or(true);
    let id = ids::tax_rate_id();
    let now = time::now_epoch();
    sqlx::query(
        "INSERT INTO tax_rates (id, account_id, active, display_name, percentage, inclusive, country, state,
            jurisdiction, description, tax_type, metadata, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(active as i64)
    .bind(&display_name)
    .bind(percentage)
    .bind(inclusive as i64)
    .bind(get_str(&params, &["country"]))
    .bind(get_str(&params, &["state"]))
    .bind(get_str(&params, &["jurisdiction"]))
    .bind(get_str(&params, &["description"]))
    .bind(get_str(&params, &["tax_type"]))
    .bind(metadata_from(&params).to_string())
    .bind(now)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let tax_rate: TaxRateRow = sqlx::query_as("SELECT * FROM tax_rates WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(tax_rate.to_json()))
}

pub async fn get_tax_rate(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let tax_rate: Option<TaxRateRow> =
        sqlx::query_as("SELECT * FROM tax_rates WHERE id = ?1 AND account_id = ?2")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let tax_rate =
        tax_rate.ok_or_else(|| ApiError::not_found(v1(), format!("No such tax rate: {id}")))?;
    Ok(ok(tax_rate.to_json()))
}

pub async fn list_tax_rates(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let tax_rates: Vec<TaxRateRow> = sqlx::query_as(
        "SELECT * FROM tax_rates WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let active = get_bool(&params, &["active"]);
    let inclusive = get_bool(&params, &["inclusive"]);
    let tax_rates: Vec<TaxRateRow> = tax_rates
        .into_iter()
        .filter(|rate| active.is_none_or(|value| (rate.active != 0) == value))
        .filter(|rate| inclusive.is_none_or(|value| (rate.inclusive != 0) == value))
        .collect();
    let (page, has_more) = paginate(
        tax_rates,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |rate| rate.id.clone(),
    );
    let data = page.iter().map(TaxRateRow::to_json).collect();
    Ok(ok(list_envelope("/v1/tax_rates", data, has_more)))
}

// ---------------------------------------------------------------------------
// Billing portal
// ---------------------------------------------------------------------------

pub async fn create_billing_portal_session(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let customer =
        get_str(&params, &["customer"]).ok_or_else(|| ApiError::missing_param(v1(), "customer"))?;
    let return_url = get_str(&params, &["return_url"])
        .ok_or_else(|| ApiError::missing_param(v1(), "return_url"))?;

    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(&customer)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    if existing.is_none() {
        return Err(ApiError::not_found(
            v1(),
            format!("No such customer: {customer}"),
        ));
    }

    let id = ids::billing_portal_session_id();
    let now = time::now_epoch();
    let url = format!("{}/mock/billing-portal/{}", st.config.base_url(), id);
    sqlx::query(
        "INSERT INTO billing_portal_sessions (id, account_id, customer, return_url, url, created)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&id)
    .bind(&ctx.account_id)
    .bind(&customer)
    .bind(&return_url)
    .bind(&url)
    .bind(now)
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let session: BillingPortalSessionRow =
        sqlx::query_as("SELECT * FROM billing_portal_sessions WHERE id = ?1")
            .bind(&id)
            .fetch_one(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    Ok(ok(session.to_json()))
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

pub async fn get_event(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let event: Option<EventRow> =
        sqlx::query_as("SELECT * FROM events WHERE id = ?1 AND account_id = ?2 AND version = 'v1'")
            .bind(&id)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    let event = event.ok_or_else(|| ApiError::not_found(v1(), format!("No such event: {id}")))?;
    let pending = events::pending_webhooks(&st.pool, &event.id).await;
    Ok(ok(event.to_json(pending)))
}

pub async fn list_events(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let events: Vec<EventRow> = sqlx::query_as("SELECT * FROM events WHERE account_id = ?1 AND version = 'v1' ORDER BY created DESC, rowid DESC")
        .bind(&ctx.account_id)
        .fetch_all(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    let event_type = get_str(&params, &["type"]);
    let events: Vec<EventRow> = match event_type {
        Some(t) => events.into_iter().filter(|e| e.event_type == t).collect(),
        None => events,
    };
    let (page, has_more) = paginate(
        events,
        get_i64(&params, &["limit"]).or(Some(100)),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |e| e.id.clone(),
    );
    let mut data = Vec::new();
    for e in &page {
        data.push(e.to_json(events::pending_webhooks(&st.pool, &e.id).await));
    }
    Ok(ok(list_envelope("/v1/events", data, has_more)))
}

/// Build a v1 error response (used by the router fallback).
pub fn not_found_response() -> Response {
    ApiError::not_found(v1(), "Unrecognized request URL").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proration_uses_the_remaining_period_fraction() {
        // A full period remaining charges the full quantity delta.
        assert_eq!(proration_amount(1000, 2, 0, 1000, 0), 2000);
        // Half the period remaining charges half.
        assert_eq!(proration_amount(1000, 2, 0, 1000, 500), 1000);
        // A decreased quantity produces a negative amount.
        assert_eq!(proration_amount(1000, -1, 0, 1000, 250), -750);
        // A completed period yields zero, including a clamped negative now.
        assert_eq!(proration_amount(1000, 1, 0, 1000, 1000), 0);
        assert_eq!(proration_amount(1000, 1, 0, 1000, 5000), 0);
        // A zero-length period cannot be prorated.
        assert_eq!(proration_amount(1000, 1, 10, 10, 10), 0);
    }

    #[test]
    fn proration_rounds_half_away_from_zero() {
        // 101 * 1 * 1/3 = 33.67 -> 34
        assert_eq!(proration_amount(101, 1, 0, 3, 2), 34);
        // -101 * 1 * 1/3 = -33.67 -> -34
        assert_eq!(proration_amount(101, -1, 0, 3, 2), -34);
        // Exactly half rounds away from zero.
        assert_eq!(proration_amount(1, 1, 0, 2, 1), 1);
        assert_eq!(proration_amount(1, -1, 0, 2, 1), -1);
    }
    fn coupon(percent_off: Option<f64>, amount_off: Option<i64>) -> CouponRow {
        CouponRow {
            id: "coupon_1".into(),
            account_id: "acct_1".into(),
            percent_off,
            amount_off,
            currency: amount_off.map(|_| "usd".to_string()),
            duration: "once".into(),
            duration_in_months: None,
            max_redemptions: None,
            times_redeemed: 0,
            redeem_by: None,
            name: None,
            applies_to: Some("{}".into()),
            metadata: Some("{}".into()),
            created: 0,
        }
    }

    #[test]
    fn coupon_discounts_apply_sequentially_and_stop_at_zero() {
        let coupons = vec![coupon(Some(25.0), None), coupon(None, Some(1000))];
        assert_eq!(coupon_discounts(&coupons, 2000), vec![500, 1000]);

        let capped = vec![coupon(None, Some(5000))];
        assert_eq!(coupon_discounts(&capped, 2000), vec![2000]);

        let over = vec![
            coupon(None, Some(1500)),
            coupon(Some(50.0), None),
        ];
        assert_eq!(coupon_discounts(&over, 2000), vec![1500, 250]);
    }

    #[test]
    fn discount_allocation_is_proportional_with_a_deterministic_remainder() {
        let allocated = allocate_discount(&[1000, 1000], 999);
        assert_eq!(allocated.iter().sum::<i64>(), 999);
        assert_eq!(allocated, vec![500, 499]);

        // The remainder goes to the largest line, ties by order.
        assert_eq!(allocate_discount(&[3, 1], 2), vec![2, 0]);
        assert_eq!(allocate_discount(&[1, 3], 2), vec![0, 2]);
        assert_eq!(allocate_discount(&[0, 100], 50), vec![0, 50]);
        assert_eq!(allocate_discount(&[], 50), Vec::<i64>::new());
        assert_eq!(allocate_discount(&[100], 0), vec![0]);
    }

    #[test]
    fn tax_is_applied_to_the_discounted_amount_in_both_modes() {
        assert_eq!(tax_amount(1000, 20.0, false), 200);
        assert_eq!(tax_amount(1000, 10.0, false), 100);
        // Inclusive 20% of 1200 is 200; of a 1000 amount it is 166.67 -> 167.
        assert_eq!(tax_amount(1000, 20.0, true), 167);
        assert_eq!(tax_amount(1200, 20.0, true), 200);
        assert_eq!(tax_amount(750, 10.0, false), 75);
    }

}
