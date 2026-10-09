#![allow(dead_code)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use stripe_mock::config::Config;
use stripe_mock::{boot, build_router};
use tokio::sync::Mutex;

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub struct TestServer {
    pub base_url: String,
    pub client: reqwest::Client,
    pub pool: sqlx::SqlitePool,
    pub db_path: PathBuf,
    pub destination_url: String,
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

async fn wait_for_health(client: &reqwest::Client, base_url: &str) {
    for _ in 0..100 {
        if let Ok(response) = client.get(format!("{base_url}/health")).send().await {
            if response.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("server did not become healthy");
}

/// Boot a fresh server backed by a unique temp database.
pub async fn spawn() -> TestServer {
    spawn_with_receiver(None).await
}

/// Boot a fresh server, optionally pointing a snapshot event destination at a receiver.
pub async fn spawn_with_receiver(receiver_url: Option<String>) -> TestServer {
    let port = free_port();
    let db_path = std::env::temp_dir().join(format!(
        "stripe-mock-it-{}.db",
        stripe_mock::ids::random_string(10)
    ));
    let target_url =
        receiver_url.unwrap_or_else(|| format!("http://127.0.0.1:{port}/unused-webhook"));
    let config_text = format!(
        r#"
[server]
bind = "127.0.0.1"
port = {port}
public_base_url = "http://127.0.0.1:{port}"

[database]
path = "{}"

[main_account]
business_type = "company"
country = "US"
email = "owner@example.com"
name = "Mock Platform"

[[event_destinations]]
name = "app"
event_payload = "snapshot"
events_from = ["@self", "@accounts"]
enabled_events = ["*"]
enabled = true
webhook_endpoint = {{ url = "{target_url}" }}
"#,
        db_path.to_string_lossy(),
    );
    let config = Config::from_str(&config_text).unwrap();
    let (state, listener) = boot(config).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    wait_for_health(&client, &base_url).await;

    TestServer {
        base_url,
        client,
        pool: state.pool.clone(),
        db_path,
        destination_url: target_url,
    }
}

/// Boot a fresh server with a single config-declared thin event destination
/// pointing at the receiver.
pub async fn spawn_with_thin_destination(receiver_url: String) -> TestServer {
    let port = free_port();
    let db_path = std::env::temp_dir().join(format!(
        "stripe-mock-it-{}.db",
        stripe_mock::ids::random_string(10)
    ));
    let config_text = format!(
        r#"
[server]
bind = "127.0.0.1"
port = {port}
public_base_url = "http://127.0.0.1:{port}"

[database]
path = "{}"

[main_account]
business_type = "company"
country = "US"
email = "owner@example.com"
name = "Mock Platform"

[[event_destinations]]
name = "thin-app"
event_payload = "thin"
enabled_events = ["*"]
enabled = true
webhook_endpoint = {{ url = "{receiver_url}" }}
"#,
        db_path.to_string_lossy(),
    );
    let config = Config::from_str(&config_text).unwrap();
    let (state, listener) = boot(config).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{addr}");
    let app = build_router(state.clone());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    wait_for_health(&client, &base_url).await;

    TestServer {
        base_url,
        client,
        pool: state.pool.clone(),
        db_path,
        destination_url: format!("http://127.0.0.1:{port}/unused-webhook"),
    }
}

pub async fn main_secret(server: &TestServer) -> String {
    let body: Value = server
        .client
        .get(server.url("/internal/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["secret_key"].as_str().unwrap().to_string()
}

pub async fn main_account_id(server: &TestServer) -> String {
    let body: Value = server
        .client
        .get(server.url("/internal/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["id"].as_str().unwrap().to_string()
}

// ---------------------------------------------------------------------------
// Webhook receiver
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct Received {
    pub deliveries: Vec<ReceivedDelivery>,
}

pub struct ReceivedDelivery {
    pub signature: String,
    pub delivery_id: String,
    pub raw: String,
    pub body: Value,
    pub at: std::time::Instant,
}

pub struct ReceiverState {
    pub received: Vec<ReceivedDelivery>,
    pub fail_first: usize,
    pub attempts: usize,
}

pub struct WebhookReceiver {
    pub url: String,
    pub state: Arc<Mutex<ReceiverState>>,
}

pub async fn spawn_receiver() -> WebhookReceiver {
    let state = Arc::new(Mutex::new(ReceiverState {
        received: Vec::new(),
        fail_first: 0,
        attempts: 0,
    }));

    async fn hook(
        State(state): State<Arc<Mutex<ReceiverState>>>,
        headers: HeaderMap,
        body: Bytes,
    ) -> axum::response::Response {
        let mut state = state.lock().await;
        state.attempts += 1;
        if state.fail_first > 0 {
            state.fail_first -= 1;
            return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response();
        }
        let signature = headers
            .get("stripe-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let delivery_id = headers
            .get("x-stripe-mock-delivery-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let parsed: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let raw = String::from_utf8_lossy(&body).to_string();
        state.received.push(ReceivedDelivery {
            signature,
            delivery_id,
            raw,
            body: parsed,
            at: std::time::Instant::now(),
        });
        axum::response::IntoResponse::into_response(Json(json!({ "ok": true })))
    }

    let app = Router::new()
        .route("/hook", post(hook))
        .with_state(state.clone());
    let port = free_port();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    WebhookReceiver {
        url: format!("http://127.0.0.1:{port}/hook"),
        state,
    }
}

pub async fn received_count(receiver: &WebhookReceiver) -> usize {
    receiver.state.lock().await.received.len()
}

/// Reset a shared counter (used to keep the compiler happy when a test only needs the type).
pub async fn bump(counter: &AtomicUsize) -> usize {
    counter.fetch_add(1, Ordering::SeqCst)
}
