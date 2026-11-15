//! Saved payment methods. Checkout attaches the simulated card to the customer
//! and makes it the invoice default, so applications can render the billing
//! page's card from the API.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use chrono::Datelike;
use serde_json::json;
use sqlx::{Sqlite, Transaction};

use crate::AppState;
use crate::api::{ok, paginate};
use crate::auth::{self, AuthContext};
use crate::error::{ApiError, ApiVersion};
use crate::ids;
use crate::models::{PaymentMethodRow, card_payment_method_card};
use crate::time;
use crate::util::list_envelope;

fn v1() -> ApiVersion {
    ApiVersion::V1
}

async fn auth(st: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    auth::authenticate(&st.pool, headers, v1()).await
}

async fn load_payment_method(
    st: &AppState,
    account_id: &str,
    id: &str,
) -> Result<PaymentMethodRow, ApiError> {
    let payment_method: Option<PaymentMethodRow> =
        sqlx::query_as("SELECT * FROM payment_methods WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    payment_method.ok_or_else(|| ApiError::not_found(v1(), format!("No such payment method: {id}")))
}

pub async fn get_payment_method(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let payment_method = load_payment_method(&st, &ctx.account_id, &id).await?;
    Ok(ok(payment_method.to_json()))
}

pub async fn list_customer_payment_methods(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(customer): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(&customer)
            .bind(&ctx.account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    if exists.is_none() {
        return Err(ApiError::not_found(
            v1(),
            format!("No such customer: {customer}"),
        ));
    }
    let methods: Vec<PaymentMethodRow> = sqlx::query_as(
        "SELECT * FROM payment_methods WHERE account_id = ?1 AND customer = ?2
         ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .bind(&customer)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let (page, has_more) = paginate(methods, None, None, None, |method| method.id.clone());
    let data = page.iter().map(PaymentMethodRow::to_json).collect();
    Ok(ok(list_envelope(
        &format!("/v1/customers/{customer}/payment_methods"),
        data,
        has_more,
    )))
}

pub async fn get_customer_payment_method(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path((customer, id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let payment_method = load_payment_method(&st, &ctx.account_id, &id).await?;
    if payment_method.customer.as_deref() != Some(customer.as_str()) {
        return Err(ApiError::not_found(
            v1(),
            format!("No such payment method: {id}"),
        ));
    }
    Ok(ok(payment_method.to_json()))
}

/// Create the simulated card a completed checkout saves on the customer and
/// make it their default payment method. Idempotent for a customer that already
/// has a saved card.
pub async fn attach_default_card(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    customer_id: &str,
) -> Result<String, sqlx::Error> {
    let existing: Option<String> = sqlx::query_scalar(
        "SELECT id FROM payment_methods WHERE account_id = ?1 AND customer = ?2
         ORDER BY created, rowid LIMIT 1",
    )
    .bind(account_id)
    .bind(customer_id)
    .fetch_optional(&mut **tx)
    .await?;
    let payment_method_id = match existing {
        Some(id) => id,
        None => {
            let (email, name): (Option<String>, Option<String>) = sqlx::query_as(
                "SELECT email, name FROM customers WHERE id = ?1 AND account_id = ?2",
            )
            .bind(customer_id)
            .bind(account_id)
            .fetch_one(&mut **tx)
            .await?;
            let id = ids::payment_method_id();
            let now = time::now_epoch();
            let exp_year = (chrono::Utc::now().year() + 3) as i64;
            sqlx::query(
                "INSERT INTO payment_methods (id, account_id, customer, pm_type, card,
                    billing_details, metadata, created)
                 VALUES (?1, ?2, ?3, 'card', ?4, ?5, '{}', ?6)",
            )
            .bind(&id)
            .bind(account_id)
            .bind(customer_id)
            .bind(card_payment_method_card("visa", "4242", 12, exp_year).to_string())
            .bind(
                json!({
                    "address": null,
                    "email": email,
                    "name": name,
                    "phone": null,
                })
                .to_string(),
            )
            .bind(now)
            .execute(&mut **tx)
            .await?;
            id
        }
    };
    sqlx::query("UPDATE customers SET invoice_settings = ?1 WHERE id = ?2 AND account_id = ?3")
        .bind(json!({ "default_payment_method": payment_method_id }).to_string())
        .bind(customer_id)
        .bind(account_id)
        .execute(&mut **tx)
        .await?;
    Ok(payment_method_id)
}
