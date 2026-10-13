mod common;

use common::*;
use serde_json::{Value, json};

/// POST a seed request to the internal control plane.
async fn seed(server: &TestServer, body: Value) -> (reqwest::StatusCode, Value) {
    let response = server
        .client
        .post(server.url("/internal/accounts"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let value = response.json().await.unwrap();
    (status, value)
}

async fn account_count(server: &TestServer) -> usize {
    let body: Value = server
        .client
        .get(server.url("/internal/accounts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    body["data"].as_array().unwrap().len()
}

#[tokio::test]
async fn seed_returns_id_and_keys_without_auth() {
    let server = spawn().await;

    let (status, body) = seed(&server, json!({ "name": "Acme" })).await;
    assert!(status.is_success());
    assert_eq!(body["object"], "account");
    assert_eq!(body["is_main"], false);
    assert!(
        body["id"].as_str().unwrap().starts_with("acct_"),
        "unexpected id: {}",
        body["id"]
    );
    assert!(
        body["secret_key"].as_str().unwrap().starts_with("sk_test_"),
        "unexpected secret key: {}",
        body["secret_key"]
    );
    assert!(
        body["publishable_key"]
            .as_str()
            .unwrap()
            .starts_with("pk_test_"),
        "unexpected publishable key: {}",
        body["publishable_key"]
    );
}

#[tokio::test]
async fn active_and_pending_states_and_distinct_ids() {
    let server = spawn().await;

    let (_, active) = seed(
        &server,
        json!({ "name": "Active Co", "state": "active" }),
    )
    .await;
    assert_eq!(active["details_submitted"], true);
    assert_eq!(active["charges_enabled"], true);
    assert_eq!(active["payouts_enabled"], true);

    let (_, pending) = seed(&server, json!({ "name": "Pending Co" })).await;
    assert_eq!(pending["details_submitted"], false);
    assert_eq!(pending["charges_enabled"], false);
    assert_eq!(pending["payouts_enabled"], false);

    let active_id = active["id"].as_str().unwrap();
    let pending_id = pending["id"].as_str().unwrap();
    assert_ne!(active_id, pending_id);

    // Capability statuses surface through the v1 projection, authenticated with
    // each seeded account's own secret key.
    let active_v1 = get_account(&server, active_id, active["secret_key"].as_str().unwrap()).await;
    assert_eq!(active_v1["capabilities"]["card_payments"], "active");
    assert_eq!(active_v1["capabilities"]["transfers"], "active");

    let pending_v1 =
        get_account(&server, pending_id, pending["secret_key"].as_str().unwrap()).await;
    assert_eq!(pending_v1["capabilities"]["card_payments"], "pending");
    assert_eq!(pending_v1["capabilities"]["transfers"], "pending");
}

#[tokio::test]
async fn unknown_state_is_rejected_and_creates_nothing() {
    let server = spawn().await;
    let before = account_count(&server).await;

    let (status, body) = seed(&server, json!({ "name": "Acme", "state": "frozen" })).await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "state");

    assert_eq!(account_count(&server).await, before);
}

async fn get_account(server: &TestServer, id: &str, key: &str) -> Value {
    server
        .client
        .get(server.url(&format!("/v1/accounts/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn seeded_keys_are_usable() {
    let server = spawn().await;
    let (_, body) = seed(
        &server,
        json!({ "name": "Acme", "state": "active", "country": "GB" }),
    )
    .await;
    let id = body["id"].as_str().unwrap();
    let secret = body["secret_key"].as_str().unwrap().to_string();
    let publishable = body["publishable_key"].as_str().unwrap().to_string();

    // The internal secrets endpoint returns the same keys.
    let secrets: Value = server
        .client
        .get(server.url(&format!("/internal/accounts/{id}/secrets")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(secrets["secret_key"], secret);
    assert_eq!(secrets["publishable_key"], publishable);

    // The Stripe-compatible projection reflects the seeded state, and the seeded
    // secret key authenticates the request.
    let account = get_account(&server, id, &secret).await;
    assert_eq!(account["id"], id);
    assert_eq!(account["business_type"], "company");
    assert_eq!(account["country"], "GB");
    assert_eq!(account["details_submitted"], true);
    assert_eq!(account["charges_enabled"], true);
    assert_eq!(account["payouts_enabled"], true);
}

#[tokio::test]
async fn seed_then_use_after_reset() {
    let server = spawn().await;
    // Create some state, then reset the mock.
    let _ = seed(&server, json!({ "name": "Before" })).await;
    let reset: Value = server
        .client
        .post(server.url("/internal/reset"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reset["status"], "ok");

    // Seeding still works after a reset and the fresh keys are usable.
    let (status, body) = seed(&server, json!({ "name": "After", "state": "active" })).await;
    assert!(status.is_success());
    let id = body["id"].as_str().unwrap();
    let secret = body["secret_key"].as_str().unwrap();
    let account = get_account(&server, id, secret).await;
    assert_eq!(account["id"], id);
    assert_eq!(account["details_submitted"], true);
}
