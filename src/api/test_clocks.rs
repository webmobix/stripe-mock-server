//! Stripe-compatible test clocks.
//!
//! A test clock freezes the time of the customers attached to it. Billing
//! objects created for those customers inherit the clock, and advancing the
//! clock drives every attached subscription through its overdue periods.

use axum::extract::{Path, RawQuery, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::json;

use crate::AppState;
use crate::api::billing::{
    AdvanceRun, advance_period, advance_subscription_periods, subscription_interval,
};
use crate::api::expand::{expand_list_response, expand_response};
use crate::api::{collect_form_list, collect_list_param, get_i64, get_str, ok, paginate};
use crate::auth::{self, AuthContext};
use crate::error::{ApiError, ApiVersion};
use crate::events;
use crate::form;
use crate::ids;
use crate::models::{CustomerRow, SubscriptionRow, TestClockRow};
use crate::time;
use crate::util::list_envelope;

/// Stripe's maximum `name` length.
const NAME_MAX_CHARS: usize = 300;
/// A clock is deleted by Stripe's cleanup job 30 days after creation.
const DELETES_AFTER_SECONDS: i64 = 30 * 86_400;

fn v1() -> ApiVersion {
    ApiVersion::V1
}

async fn auth(st: &AppState, headers: &HeaderMap) -> Result<AuthContext, ApiError> {
    auth::authenticate(&st.pool, headers, v1()).await
}

/// Load a clock owned by the account or answer `not_found`.
pub(crate) async fn load_clock(
    st: &AppState,
    account_id: &str,
    id: &str,
) -> Result<TestClockRow, ApiError> {
    let clock: Option<TestClockRow> =
        sqlx::query_as("SELECT * FROM test_clocks WHERE id = ?1 AND account_id = ?2")
            .bind(id)
            .bind(account_id)
            .fetch_optional(&st.pool)
            .await
            .map_err(ApiError::internal)?;
    clock.ok_or_else(|| ApiError::not_found(v1(), format!("No such test clock: {id}")))
}

/// Validate a customer that a clock is being attached to.
///
/// The customer must belong to the account, must not already carry a clock,
/// and must have no live subscriptions: a clock cannot retroactively drive
/// subscriptions that were created before the attachment.
async fn validate_clock_customer(
    conn: &mut sqlx::SqliteConnection,
    account_id: &str,
    customer_id: &str,
) -> Result<CustomerRow, ApiError> {
    let customer: Option<CustomerRow> =
        sqlx::query_as("SELECT * FROM customers WHERE id = ?1 AND account_id = ?2")
            .bind(customer_id)
            .bind(account_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(ApiError::internal)?;
    let customer = customer.ok_or_else(|| {
        ApiError::invalid_param(v1(), "customer", format!("No such customer: {customer_id}"))
    })?;
    if customer.test_clock.is_some() {
        return Err(ApiError::invalid_param(
            v1(),
            "customer",
            "A customer can only be attached to one test clock",
        ));
    }
    let live_subscriptions: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM subscriptions
         WHERE customer = ?1 AND account_id = ?2 AND status != 'canceled'",
    )
    .bind(customer_id)
    .bind(account_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(ApiError::internal)?;
    if live_subscriptions > 0 {
        return Err(ApiError::invalid_param(
            v1(),
            "customer",
            "A customer with active subscriptions cannot be attached to a test clock",
        ));
    }
    Ok(customer)
}

pub async fn create_test_clock(
    State(st): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let frozen_time = get_i64(&params, &["frozen_time"])
        .ok_or_else(|| ApiError::missing_param(v1(), "frozen_time"))?;
    let name = get_str(&params, &["name"]);
    if name
        .as_deref()
        .is_some_and(|name| name.chars().count() > NAME_MAX_CHARS)
    {
        return Err(ApiError::invalid_param(
            v1(),
            "name",
            format!("name must be at most {NAME_MAX_CHARS} characters"),
        ));
    }
    let customer_id = get_str(&params, &["customer"]);

    let id = ids::test_clock_id();
    let now = time::now_epoch();
    let event_id = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;

        // Validating the customer before the clock exists keeps a rejected
        // attach from leaving an orphan clock.
        if let Some(customer_id) = &customer_id {
            validate_clock_customer(&mut tx, &ctx.account_id, customer_id).await?;
        }

        sqlx::query(
            "INSERT INTO test_clocks (id, account_id, name, frozen_time, status, created, deletes_after)
             VALUES (?1, ?2, ?3, ?4, 'ready', ?5, ?6)",
        )
        .bind(&id)
        .bind(&ctx.account_id)
        .bind(&name)
        .bind(frozen_time)
        .bind(now)
        .bind(now + DELETES_AFTER_SECONDS)
        .execute(&mut *tx)
        .await
        .map_err(ApiError::internal)?;

        if let Some(customer_id) = &customer_id {
            // Attaching freezes the customer: its `created` moves onto the
            // clock's timeline like a customer created with `test_clock`.
            sqlx::query("UPDATE customers SET test_clock = ?1, created = ?2 WHERE id = ?3")
                .bind(&id)
                .bind(frozen_time)
                .bind(customer_id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        }

        let clock: TestClockRow = sqlx::query_as("SELECT * FROM test_clocks WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let event_id = events::emit(
            &mut tx,
            &ctx.account_id,
            ctx.is_connected,
            "test_helpers.test_clock.created",
            clock.to_json(),
        )
        .await
        .map_err(ApiError::internal)?;
        tx.commit().await.map_err(ApiError::internal)?;
        event_id
    };
    // The state change is already committed; a failed enqueue must not turn it into a 500.
    let _ = st.dispatcher.dispatch_event(&event_id).await;

    let clock = load_clock(&st, &ctx.account_id, &id).await?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        clock.to_json(),
        &collect_form_list(&params, "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn get_test_clock(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let clock = load_clock(&st, &ctx.account_id, &id).await?;
    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        clock.to_json(),
        &collect_list_param(query.as_deref().unwrap_or(""), "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn list_test_clocks(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(query.as_deref().unwrap_or(""));
    let clocks: Vec<TestClockRow> = sqlx::query_as(
        "SELECT * FROM test_clocks WHERE account_id = ?1 ORDER BY created DESC, rowid DESC",
    )
    .bind(&ctx.account_id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let (page, has_more) = paginate(
        clocks,
        get_i64(&params, &["limit"]),
        get_str(&params, &["starting_after"]),
        get_str(&params, &["ending_before"]),
        |clock| clock.id.clone(),
    );
    let data = page.iter().map(TestClockRow::to_json).collect();
    let envelope = list_envelope("/v1/test_helpers/test_clocks", data, has_more);
    let value = expand_list_response(
        &st.pool,
        &ctx.account_id,
        envelope,
        &collect_list_param(query.as_deref().unwrap_or(""), "expand"),
    )
    .await;
    Ok(ok(value))
}

pub async fn delete_test_clock(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let clock = load_clock(&st, &ctx.account_id, &id).await?;

    let event_ids = {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        let mut event_ids = Vec::new();

        // Cancel the clock's subscriptions before the customers disappear.
        let subscriptions: Vec<SubscriptionRow> =
            sqlx::query_as("SELECT * FROM subscriptions WHERE account_id = ?1 AND test_clock = ?2")
                .bind(&ctx.account_id)
                .bind(&id)
                .fetch_all(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        for subscription in &subscriptions {
            if subscription.status == "canceled" {
                continue;
            }
            sqlx::query("UPDATE subscriptions SET status = 'canceled' WHERE id = ?1")
                .bind(&subscription.id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            let canceled: SubscriptionRow =
                sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1")
                    .bind(&subscription.id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(ApiError::internal)?;
            event_ids.push(
                events::emit(
                    &mut tx,
                    &ctx.account_id,
                    ctx.is_connected,
                    "customer.subscription.deleted",
                    canceled.to_json(),
                )
                .await
                .map_err(ApiError::internal)?,
            );
        }

        let customers: Vec<CustomerRow> =
            sqlx::query_as("SELECT * FROM customers WHERE account_id = ?1 AND test_clock = ?2")
                .bind(&ctx.account_id)
                .bind(&id)
                .fetch_all(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
        for customer in &customers {
            // Saved cards go with the customer.
            sqlx::query("DELETE FROM payment_methods WHERE customer = ?1 AND account_id = ?2")
                .bind(&customer.id)
                .bind(&ctx.account_id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            sqlx::query("DELETE FROM customers WHERE id = ?1")
                .bind(&customer.id)
                .execute(&mut *tx)
                .await
                .map_err(ApiError::internal)?;
            event_ids.push(
                events::emit(
                    &mut tx,
                    &ctx.account_id,
                    ctx.is_connected,
                    "customer.deleted",
                    customer.to_json(),
                )
                .await
                .map_err(ApiError::internal)?,
            );
        }

        sqlx::query("DELETE FROM test_clocks WHERE id = ?1")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        event_ids.push(
            events::emit(
                &mut tx,
                &ctx.account_id,
                ctx.is_connected,
                "test_helpers.test_clock.deleted",
                clock.to_json(),
            )
            .await
            .map_err(ApiError::internal)?,
        );
        tx.commit().await.map_err(ApiError::internal)?;
        event_ids
    };
    for event_id in event_ids {
        let _ = st.dispatcher.dispatch_event(&event_id).await;
    }

    Ok(ok(json!({
        "id": clock.id,
        "object": "test_helpers.test_clock",
        "deleted": true,
    })))
}

/// `POST /v1/test_helpers/test_clocks/{id}/advance`.
///
/// All work runs in one transaction. The response reports `advancing` while
/// the committed clock is already `ready`, so clients that assert the immediate
/// response and clients that poll for `ready` both succeed.
pub async fn advance_test_clock(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: String,
) -> Result<Response, ApiError> {
    let ctx = auth(&st, &headers).await?;
    let params = form::parse_form(&body);
    let target = get_i64(&params, &["frozen_time"])
        .ok_or_else(|| ApiError::missing_param(v1(), "frozen_time"))?;
    let clock = load_clock(&st, &ctx.account_id, &id).await?;
    if clock.status != "ready" {
        return Err(ApiError::invalid_request(
            v1(),
            "The test clock is not ready",
        ));
    }
    if target <= clock.frozen_time {
        return Err(ApiError::invalid_param(
            v1(),
            "frozen_time",
            format!(
                "frozen_time must be later than the current frozen time {}",
                clock.frozen_time
            ),
        ));
    }

    let subscriptions: Vec<SubscriptionRow> = sqlx::query_as(
        "SELECT * FROM subscriptions WHERE account_id = ?1 AND test_clock = ?2
         ORDER BY current_period_end, id",
    )
    .bind(&ctx.account_id)
    .bind(&id)
    .fetch_all(&st.pool)
    .await
    .map_err(ApiError::internal)?;
    let live: Vec<&SubscriptionRow> = subscriptions
        .iter()
        .filter(|subscription| subscription.status != "canceled")
        .collect();
    // Two intervals of the shortest subscription bound a single advance; an
    // empty clock gets two years.
    let max_span = if live.is_empty() {
        2 * 365 * 86_400
    } else {
        live.iter()
            .map(|subscription| {
                let (interval, count) = subscription_interval(subscription);
                advance_period(0, &interval, count)
            })
            .min()
            .unwrap_or(30 * 86_400)
            * 2
    };
    if target - clock.frozen_time > max_span {
        return Err(ApiError::invalid_param(
            v1(),
            "frozen_time",
            format!(
                "frozen_time is too far ahead: a single advance may cover at most {max_span} seconds"
            ),
        ));
    }

    let mut event_ids = Vec::new();
    {
        let mut tx = st.pool.begin().await.map_err(ApiError::internal)?;
        for subscription in &live {
            advance_subscription_periods(
                &mut tx,
                subscription,
                &ctx.account_id,
                ctx.is_connected,
                AdvanceRun::Through(target),
                true,
                clock.frozen_time,
                &st.config.base_url(),
                &mut event_ids,
            )
            .await?;
        }
        sqlx::query("UPDATE test_clocks SET frozen_time = ?1, status = 'ready' WHERE id = ?2")
            .bind(target)
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        let updated: TestClockRow = sqlx::query_as("SELECT * FROM test_clocks WHERE id = ?1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(ApiError::internal)?;
        event_ids.push(
            events::emit_at(
                &mut tx,
                &ctx.account_id,
                ctx.is_connected,
                "test_helpers.test_clock.advancing",
                clock.render("advancing", json!({ "target_frozen_time": target })),
                clock.frozen_time,
            )
            .await
            .map_err(ApiError::internal)?,
        );
        event_ids.push(
            events::emit_at(
                &mut tx,
                &ctx.account_id,
                ctx.is_connected,
                "test_helpers.test_clock.ready",
                updated.to_json(),
                target,
            )
            .await
            .map_err(ApiError::internal)?,
        );
        tx.commit().await.map_err(ApiError::internal)?;
    }
    for event_id in event_ids {
        let _ = st.dispatcher.dispatch_event(&event_id).await;
    }

    let value = expand_response(
        &st.pool,
        &ctx.account_id,
        clock.render("advancing", json!({ "target_frozen_time": target })),
        &collect_form_list(&params, "expand"),
    )
    .await;
    Ok(ok(value))
}
