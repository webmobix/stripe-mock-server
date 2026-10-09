mod common;

use std::time::Duration;

use common::*;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

async fn create_connected_account(server: &TestServer, key: &str) -> String {
    let created: Value = server
        .client
        .post(server.url("/v2/core/accounts"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({ "configuration": { "merchant": {} } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    created["id"].as_str().unwrap().to_string()
}

async fn onboard(server: &TestServer, key: &str, account: &str) -> String {
    let link: Value = server
        .client
        .post(server.url("/v1/account_links"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("account", account),
            ("type", "account_onboarding"),
            ("refresh_url", "https://example.com/refresh"),
            ("return_url", "https://example.com/return"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = link["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    server
        .client
        .post(server.url(&format!("/mock/connect/{token}")))
        .form(&[("kyc", "pass")])
        .send()
        .await
        .unwrap();
    // Return the latest account.updated event id for this account.
    let events: Value = server
        .client
        .get(server.url("/v1/events?type=account.updated"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", account)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    events["data"][0]["id"].as_str().unwrap().to_string()
}

async fn wait_delivery(server: &TestServer, event_id: &str) -> Value {
    server
        .client
        .post(server.url("/internal/deliveries/wait"))
        .json(&json!({ "event_id": event_id, "timeout_ms": 8000 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn destination_secret(server: &TestServer) -> String {
    let destinations: Value = server
        .client
        .get(server.url("/internal/destinations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    destinations["data"][0]["webhook_endpoint"]["signing_secret"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn internal_main_and_destinations_expose_secrets() {
    let server = spawn().await;
    let main: Value = server
        .client
        .get(server.url("/internal/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(main["secret_key"].as_str().unwrap().starts_with("sk_test_"));
    assert!(
        main["publishable_key"]
            .as_str()
            .unwrap()
            .starts_with("pk_test_")
    );
    assert_eq!(main["is_main"], true);

    let secret = destination_secret(&server).await;
    assert!(secret.starts_with("whsec_"));
}

#[tokio::test]
async fn event_is_created_synchronously_and_delivered_async() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let account = create_connected_account(&server, &key).await;

    let event_id = onboard(&server, &key, &account).await;

    // Response returned before the webhook (the engine waits 200ms).
    assert_eq!(received_count(&receiver).await, 0);

    let waited = wait_delivery(&server, &event_id).await;
    assert_eq!(waited["settled"], true);
    assert_eq!(waited["deliveries"][0]["status"], "succeeded");

    let state = receiver.state.lock().await;
    assert_eq!(state.received.len(), 1);
    let delivery = &state.received[0];
    assert_eq!(delivery.body["type"], "account.updated");
    assert_eq!(delivery.body["account"], account);
    assert!(delivery.delivery_id.starts_with("dl_"));

    // Verify the HMAC signature with the destination secret.
    let secret = destination_secret(&server).await;
    let (timestamp, signature) = delivery
        .signature
        .split_once(',')
        .map(|(t, v)| {
            (
                t.trim_start_matches("t=").to_string(),
                v.trim_start_matches("v1=").to_string(),
            )
        })
        .unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{timestamp}.{}", delivery.raw).as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());
    assert_eq!(expected, signature);
}

#[tokio::test]
async fn deliveries_are_ordered_and_spaced() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let account1 = create_connected_account(&server, &key).await;
    let account2 = create_connected_account(&server, &key).await;

    let event1 = onboard(&server, &key, &account1).await;
    let event2 = onboard(&server, &key, &account2).await;
    let _ = event1;
    let _ = event2;

    server
        .client
        .post(server.url("/internal/deliveries/flush"))
        .json(&json!({ "timeout_ms": 8000 }))
        .send()
        .await
        .unwrap();

    let state = receiver.state.lock().await;
    assert_eq!(state.received.len(), 2);
    assert_eq!(state.received[0].body["account"], account1);
    assert_eq!(state.received[1].body["account"], account2);
    let gap = state.received[1].at.duration_since(state.received[0].at);
    assert!(gap >= Duration::from_millis(150), "gap was {gap:?}");
}

#[tokio::test]
async fn retries_then_exhausts_and_can_be_manually_retried() {
    let receiver = spawn_receiver().await;
    {
        let mut state = receiver.state.lock().await;
        state.fail_first = 10;
    }
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let account = create_connected_account(&server, &key).await;
    let event_id = onboard(&server, &key, &account).await;

    let waited = wait_delivery(&server, &event_id).await;
    assert_eq!(waited["settled"], true);
    let delivery = &waited["deliveries"][0];
    assert_eq!(delivery["status"], "failed_exhausted");
    assert_eq!(delivery["attempts"], 3);
    let delivery_id = delivery["id"].as_str().unwrap().to_string();

    let attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM delivery_attempts WHERE delivery_id = ?1")
            .bind(&delivery_id)
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert_eq!(attempts, 3);

    // Now let the receiver succeed and retry manually.
    {
        let mut state = receiver.state.lock().await;
        state.fail_first = 0;
    }
    server
        .client
        .post(server.url(&format!("/internal/deliveries/{delivery_id}/retry")))
        .send()
        .await
        .unwrap();
    let waited = wait_delivery(&server, &event_id).await;
    assert_eq!(waited["deliveries"][0]["status"], "succeeded");
    assert_eq!(received_count(&receiver).await, 1);
}

#[tokio::test]
async fn reset_clears_state_and_reseeds_main_account() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    create_connected_account(&server, &key).await;
    server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Widget")])
        .send()
        .await
        .unwrap();

    let reset = server
        .client
        .post(server.url("/internal/reset"))
        .send()
        .await
        .unwrap();
    assert!(reset.status().is_success());

    let accounts: Value = server
        .client
        .get(server.url("/internal/accounts"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let data = accounts["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["is_main"], true);

    // Reset reseeds the main account, so fetch its fresh key.
    let key = main_secret(&server).await;
    let products: Value = server
        .client
        .get(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(products["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn wait_times_out_when_delivery_does_not_settle_in_time() {
    let receiver = spawn_receiver().await;
    {
        let mut state = receiver.state.lock().await;
        // Fail so the delivery stays pending through its retries.
        state.fail_first = 10;
    }
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let account = create_connected_account(&server, &key).await;
    let event_id = onboard(&server, &key, &account).await;

    // A tiny timeout elapses while the delivery is still pending.
    let waited: Value = server
        .client
        .post(server.url("/internal/deliveries/wait"))
        .json(&json!({ "event_id": event_id, "timeout_ms": 50 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(waited["settled"], false);
}

#[tokio::test]
async fn internal_endpoints_need_no_stripe_auth() {
    let server = spawn().await;
    let response = server
        .client
        .get(server.url("/internal/main"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}
