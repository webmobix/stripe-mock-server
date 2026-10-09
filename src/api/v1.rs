use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use crate::AppState;
use crate::api::{get_array, get_bool, get_i64, get_str, ok, paginate};
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
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::invalid_param(
            v1(),
            "currency",
            "Invalid currency: must be a three-letter ISO code",
        ));
    }
    let currency = currency.to_ascii_lowercase();
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
    let id = ids::price_id();
    let now = time::now_epoch();
    sqlx::query(
        "INSERT INTO prices (id, account_id, product_id, currency, unit_amount, price_type, recurring_interval,
            recurring_interval_count, nickname, active, created, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10, ?11)",
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
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    sqlx::query("UPDATE products SET default_price = ?1 WHERE id = ?2 AND default_price IS NULL")
        .bind(&id)
        .bind(&product_id)
        .execute(&st.pool)
        .await
        .map_err(ApiError::internal)?;

    let price: PriceRow = sqlx::query_as("SELECT * FROM prices WHERE id = ?1")
        .bind(&id)
        .fetch_one(&st.pool)
        .await
        .map_err(ApiError::internal)?;
    Ok(ok(price.to_json()))
}

pub async fn get_price(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
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
    Ok(ok(price.to_json()))
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
    let (page, has_more) = paginate(
        prices,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |p| p.id.clone(),
    );
    let data = page.iter().map(|p| p.to_json()).collect();
    Ok(ok(list_envelope("/v1/prices", data, has_more)))
}

// ---------------------------------------------------------------------------
// Billing
// ---------------------------------------------------------------------------

pub async fn create_customer(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let id = ids::customer_id();
    let now = time::now_epoch();
    sqlx::query("INSERT INTO customers (id, account_id, email, name, created, metadata) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(get_str(&params, &["email"]))
        .bind(get_str(&params, &["name"]))
        .bind(now)
        .bind(metadata_from(&params).to_string())
        .execute(&st.pool)
        .await
        .map_err(ApiError::internal)?;
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

async fn load_price(st: &AppState, account_id: &str, price_id: &str) -> Result<PriceRow, ApiError> {
    let price: Option<PriceRow> =
        sqlx::query_as("SELECT * FROM prices WHERE id = ?1 AND account_id = ?2")
            .bind(price_id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    price.ok_or_else(|| {
        ApiError::invalid_param(
            v1(),
            "items[0][price]",
            format!("No such price: {price_id}"),
        )
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

fn build_subscription_item(price: &PriceRow, quantity: i64) -> Value {
    json!({
        "id": ids::id("si_"),
        "object": "subscription_item",
        "created": time::now_epoch(),
        "price": price.to_json(),
        "quantity": quantity,
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
    let mut built_items = Vec::new();
    for item in &items {
        let price_id = get_str(item, &["price"])
            .ok_or_else(|| ApiError::missing_param(v1(), "items[][price]"))?;
        let quantity = parse_quantity(item)?;
        let price = load_price(&st, &ctx.account_id, &price_id).await?;
        built_items.push(build_subscription_item(&price, quantity));
    }

    let id = ids::subscription_id();
    let now = time::now_epoch();
    let period_end = now + 30 * 86_400;

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
    Ok(ok(subscription.to_json()))
}

pub async fn get_subscription(
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
    Ok(ok(subscription.to_json()))
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
    let data = page.iter().map(|s| s.to_json()).collect();
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
    let raw_items = get_array(&params, &["line_items"])
        .cloned()
        .unwrap_or_default();
    let mut line_items = Vec::new();
    let mut amount_total = 0i64;
    let mut currency = "usd".to_string();
    for item in &raw_items {
        let price_id = get_str(item, &["price"])
            .ok_or_else(|| ApiError::missing_param(v1(), "line_items[][price]"))?;
        let quantity = parse_quantity(item)?;
        let price = load_price(&st, &ctx.account_id, &price_id).await?;
        amount_total = price
            .unit_amount
            .unwrap_or(0)
            .checked_mul(quantity)
            .and_then(|line| amount_total.checked_add(line))
            .ok_or_else(|| ApiError::invalid_param(v1(), "line_items", "Amount too large"))?;
        currency = price.currency.clone();
        line_items.push(build_line_item(&price, quantity));
    }

    let id = ids::checkout_session_id();
    let now = time::now_epoch();
    let expires_at = now + 86_400;
    let url = format!("{}/mock/checkout/{}", st.config.base_url(), id);
    let success_url = get_str(&params, &["success_url"]);
    let cancel_url = get_str(&params, &["cancel_url"]);
    let customer = get_str(&params, &["customer"]);
    let customer_account = get_str(&params, &["customer_account"]);

    sqlx::query(
        "INSERT INTO checkout_sessions (id, account_id, mode, status, created, expires_at, success_url, cancel_url,
            customer, customer_account, line_items, payment_intent, subscription, amount_total, currency, url, metadata)
         VALUES (?1, ?2, ?3, 'open', ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11, ?12, ?13, ?14)",
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
    .bind(serde_json::to_string(&line_items).unwrap())
    .bind(amount_total)
    .bind(&currency)
    .bind(&url)
    .bind(metadata_from(&params).to_string())
    .execute(&st.pool)
    .await
    .map_err(ApiError::internal)?;

    let session: CheckoutSessionRow =
        sqlx::query_as("SELECT * FROM checkout_sessions WHERE id = ?1")
            .bind(&id)
            .fetch_one(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    Ok(ok(session.to_json()))
}

pub async fn get_checkout_session(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let session = load_session(&st, &ctx.account_id, &id).await?;
    Ok(ok(session.to_json()))
}

pub async fn get_checkout_line_items(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let session = load_session(&st, &ctx.account_id, &id).await?;
    Ok(ok(list_envelope(
        &format!("/v1/checkout/sessions/{id}/line_items"),
        session.line_items(),
        false,
    )))
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
                        items.push(build_subscription_item(&price, quantity));
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
            .bind(now + 30 * 86_400)
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
