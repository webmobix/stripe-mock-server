pub mod api;
pub mod auth;
pub mod config;
pub mod db;
pub mod dispatch;
pub mod error;
pub mod events;
pub mod form;
pub mod ids;
pub mod internal;
pub mod mock;
pub mod models;
pub mod seed;
pub mod time;
pub mod util;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use sqlx::SqlitePool;

use config::Config;
use dispatch::Dispatcher;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<Config>,
    pub dispatcher: Arc<Dispatcher>,
}

impl AppState {
    pub async fn new(pool: SqlitePool, config: Config) -> Result<Self, sqlx::Error> {
        let dispatcher = Dispatcher::new(pool.clone());
        Ok(Self {
            pool,
            config: Arc::new(config),
            dispatcher,
        })
    }
}

async fn health() -> Response {
    Json(serde_json::json!({ "status": "ok" })).into_response()
}

async fn fallback() -> Response {
    api::v1::not_found_response()
}

/// Log every inbound request so the SDK drop-in guarantee can be observed.
async fn log_request(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    tracing::info!("{} {}", request.method(), request.uri().path());
    next.run(request).await
}

/// Reject browser-originated cross-site requests to `/internal/*`. Plain
/// cross-origin form POSTs need no preflight, so without this any web page the
/// developer visits could reset the mock or read keys. Non-browser clients send
/// no `Origin` header and are unaffected.
async fn guard_internal(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if request.uri().path().starts_with("/internal/") {
        let origin = request
            .headers()
            .get(axum::http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if let Some(origin) = origin {
            let host = request
                .headers()
                .get(axum::http::header::HOST)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let origin_host = origin
                .split_once("://")
                .map(|(_, rest)| rest)
                .unwrap_or(&origin);
            if origin_host != host {
                tracing::warn!("rejected cross-origin request to {}", request.uri().path());
                return (
                    axum::http::StatusCode::FORBIDDEN,
                    "cross-origin requests to /internal are not allowed",
                )
                    .into_response();
            }
        }
    }
    next.run(request).await
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        // v1 API
        .route(
            "/v1/accounts",
            post(api::v1::create_account).get(api::v1::list_accounts),
        )
        .route("/v1/accounts/{id}", get(api::v1::get_account))
        .route("/v1/account_links", post(api::v1::create_account_link))
        .route(
            "/v1/products",
            post(api::v1::create_product).get(api::v1::list_products),
        )
        .route("/v1/products/{id}", get(api::v1::get_product))
        .route(
            "/v1/prices",
            post(api::v1::create_price).get(api::v1::list_prices),
        )
        .route("/v1/prices/{id}", get(api::v1::get_price))
        .route("/v1/customers", post(api::v1::create_customer))
        .route("/v1/customers/{id}", get(api::v1::get_customer))
        .route(
            "/v1/subscriptions",
            post(api::v1::create_subscription).get(api::v1::list_subscriptions),
        )
        .route(
            "/v1/subscriptions/{id}",
            get(api::v1::get_subscription).delete(api::v1::cancel_subscription),
        )
        .route(
            "/v1/checkout/sessions",
            post(api::v1::create_checkout_session),
        )
        .route(
            "/v1/checkout/sessions/{id}",
            get(api::v1::get_checkout_session),
        )
        .route(
            "/v1/checkout/sessions/{id}/line_items",
            get(api::v1::get_checkout_line_items),
        )
        .route(
            "/v1/checkout/sessions/{id}/expire",
            post(api::v1::expire_checkout_session),
        )
        .route("/v1/events", get(api::v1::list_events))
        .route("/v1/events/{id}", get(api::v1::get_event))
        // v2 core API
        .route(
            "/v2/core/accounts",
            post(api::v2::create_account).get(api::v2::list_accounts),
        )
        .route(
            "/v2/core/accounts/{id}",
            get(api::v2::get_account).post(api::v2::update_account),
        )
        .route("/v2/core/account_links", post(api::v2::create_account_link))
        .route(
            "/v2/core/event_destinations",
            post(api::v2::create_event_destination).get(api::v2::list_event_destinations),
        )
        .route(
            "/v2/core/event_destinations/{id}",
            get(api::v2::get_event_destination)
                .post(api::v2::update_event_destination)
                .delete(api::v2::delete_event_destination),
        )
        .route(
            "/v2/core/event_destinations/{id}/enable",
            post(api::v2::enable_event_destination),
        )
        .route(
            "/v2/core/event_destinations/{id}/disable",
            post(api::v2::disable_event_destination),
        )
        .route(
            "/v2/core/event_destinations/{id}/ping",
            post(api::v2::ping_event_destination),
        )
        .route("/v2/core/events", get(api::v2::list_events))
        .route("/v2/core/events/{id}", get(api::v2::get_event))
        // Mock hosted pages
        .route(
            "/mock/connect/{token}",
            get(mock::onboarding_page).post(mock::onboarding_submit),
        )
        .route(
            "/mock/checkout/{id}",
            get(mock::checkout_page).post(mock::checkout_submit),
        )
        .route("/mock/checkout/{id}/cancel", get(mock::checkout_cancel))
        // Internal control plane
        .route("/internal/main", get(internal::main_account))
        .route(
            "/internal/accounts",
            get(internal::list_accounts).post(internal::create_account),
        )
        .route(
            "/internal/accounts/{id}/secrets",
            get(internal::account_secrets),
        )
        .route("/internal/destinations", get(internal::list_destinations))
        .route("/internal/deliveries", get(internal::list_deliveries))
        .route(
            "/internal/deliveries/{id}/retry",
            post(internal::retry_delivery),
        )
        .route("/internal/deliveries/wait", post(internal::wait_delivery))
        .route(
            "/internal/deliveries/flush",
            post(internal::flush_deliveries),
        )
        .route("/internal/reset", post(internal::reset))
        .layer(axum::middleware::from_fn(guard_internal))
        .layer(axum::middleware::from_fn(log_request))
        .fallback(fallback)
        .with_state(state)
}

/// Load configuration, connect and migrate the database, seed, start workers,
/// and bind the listener. Returns the state and a bound listener.
pub async fn boot(
    config: Config,
) -> Result<(AppState, tokio::net::TcpListener), Box<dyn std::error::Error>> {
    let addr = format!("{}:{}", config.server.bind, config.server.port);
    if !matches!(
        config.server.bind.as_str(),
        "127.0.0.1" | "localhost" | "::1"
    ) {
        tracing::warn!(
            "binding to {}: the unauthenticated /internal API (including secret keys) is reachable from the network",
            config.server.bind
        );
    }
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    boot_with_listener(config, listener).await
}

/// Like [`boot`], but serves on a caller-provided, already-bound listener.
///
/// Bind on port `0` to let the OS pick a free port, then call this with a
/// matching `public_base_url`. Tests use it to avoid the race between probing
/// for a free port and binding it later.
pub async fn boot_with_listener(
    config: Config,
    listener: tokio::net::TcpListener,
) -> Result<(AppState, tokio::net::TcpListener), Box<dyn std::error::Error>> {
    let pool = db::connect(&config.database.path).await?;
    db::migrate(&pool).await?;
    if config.database.reset_on_start {
        db::reset_state(&pool).await?;
    }
    seed::seed(&pool, &config).await?;
    let state = AppState::new(pool, config).await?;
    state.dispatcher.start().await?;
    Ok((state, listener))
}
