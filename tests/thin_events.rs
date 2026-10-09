mod common;

use common::*;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;

async fn create_destination(
    server: &TestServer,
    key: &str,
    url: &str,
    enabled_events: &[&str],
) -> Value {
    let body = json!({
        "name": "thin-receiver",
        "description": "thin event destination",
        "enabled_events": enabled_events,
        "event_payload": "thin",
        "type": "webhook_endpoint",
        "webhook_endpoint": { "url": url },
        "include": ["webhook_endpoint.signing_secret", "webhook_endpoint.url"],
    });
    server
        .client
        .post(server.url("/v2/core/event_destinations"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn create_destination_with_payload(
    server: &TestServer,
    key: &str,
    name: &str,
    url: &str,
    event_payload: &str,
) -> Value {
    let body = json!({
        "name": name,
        "enabled_events": ["*"],
        "event_payload": event_payload,
        "type": "webhook_endpoint",
        "webhook_endpoint": { "url": url },
        "include": ["webhook_endpoint.signing_secret", "webhook_endpoint.url"],
    });
    server
        .client
        .post(server.url("/v2/core/event_destinations"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn create_v2_account(server: &TestServer, key: &str) -> String {
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

async fn latest_event_of_type(server: &TestServer, key: &str, event_type: &str) -> Value {
    let list: Value = server
        .client
        .get(server.url("/v2/core/events?limit=100"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    list["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == event_type)
        .cloned()
        .unwrap_or(Value::Null)
}

async fn latest_v2_event(server: &TestServer, key: &str) -> Value {
    latest_event_of_type(server, key, "v2.core.account.created").await
}

async fn onboard_account(server: &TestServer, key: &str, account: &str) {
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

fn assert_signature(secret: &str, delivery: &common::ReceivedDelivery) {
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
async fn event_destination_crud_and_secret_include() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let created = create_destination(
        &server,
        &key,
        "http://127.0.0.1:9/hook",
        &["v2.core.account.created"],
    )
    .await;
    assert_eq!(created["object"], "v2.core.event_destination");
    assert!(created["id"].as_str().unwrap().starts_with("ed_test_"));
    assert_eq!(created["status"], "enabled");
    assert_eq!(created["event_payload"], "thin");
    assert_eq!(created["type"], "webhook_endpoint");
    assert_eq!(created["enabled_events"][0], "v2.core.account.created");
    assert!(
        created["webhook_endpoint"]["signing_secret"]
            .as_str()
            .unwrap()
            .starts_with("whsec_")
    );
    assert_eq!(
        created["webhook_endpoint"]["url"],
        "http://127.0.0.1:9/hook"
    );
    let id = created["id"].as_str().unwrap().to_string();

    // Without the signing-secret include the secret is redacted.
    let fetched: Value = server
        .client
        .get(server.url(&format!(
            "/v2/core/event_destinations/{id}?include[]=webhook_endpoint.url"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(fetched["webhook_endpoint"]["signing_secret"].is_null());
    assert_eq!(
        fetched["webhook_endpoint"]["url"],
        "http://127.0.0.1:9/hook"
    );

    // Disable, enable, then update.
    let disabled: Value = server
        .client
        .post(server.url(&format!("/v2/core/event_destinations/{id}/disable")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(disabled["status"], "disabled");
    assert_eq!(disabled["status_details"]["disabled"]["reason"], "user");

    let enabled: Value = server
        .client
        .post(server.url(&format!("/v2/core/event_destinations/{id}/enable")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(enabled["status"], "enabled");

    let updated: Value = server
        .client
        .post(server.url(&format!("/v2/core/event_destinations/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({ "name": "renamed", "enabled_events": ["v2.core.account.updated"] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["name"], "renamed");
    assert_eq!(updated["enabled_events"][0], "v2.core.account.updated");

    // Internal endpoint exposes the secret for local verification.
    let internal: Value = server
        .client
        .get(server.url("/internal/destinations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let internal_destination = internal["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id.as_str())
        .unwrap();
    assert!(
        internal_destination["webhook_endpoint"]["signing_secret"]
            .as_str()
            .unwrap()
            .starts_with("whsec_")
    );

    let deleted: Value = server
        .client
        .delete(server.url(&format!("/v2/core/event_destinations/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);

    let list: Value = server
        .client
        .get(server.url("/v2/core/event_destinations"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        list["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["id"] != id.as_str())
    );
}

#[tokio::test]
async fn one_signing_secret_per_endpoint_url() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let url = "http://127.0.0.1:4099/shared-hook";
    let other = "http://127.0.0.1:4099/other-hook";

    let snapshot = create_destination_with_payload(&server, &key, "snap", url, "snapshot").await;
    let thin = create_destination_with_payload(&server, &key, "thin", url, "thin").await;
    let elsewhere = create_destination_with_payload(&server, &key, "far", other, "thin").await;

    let snapshot_secret = snapshot["webhook_endpoint"]["signing_secret"]
        .as_str()
        .unwrap();
    let thin_secret = thin["webhook_endpoint"]["signing_secret"].as_str().unwrap();
    let elsewhere_secret = elsewhere["webhook_endpoint"]["signing_secret"]
        .as_str()
        .unwrap();

    // Two destinations (snapshot + thin) on the same URL share one secret...
    assert_eq!(snapshot_secret, thin_secret);
    // ...and different URLs get different secrets.
    assert_ne!(snapshot_secret, elsewhere_secret);
}

#[tokio::test]
async fn unsupported_destination_type_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let response = server
        .client
        .post(server.url("/v2/core/event_destinations"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({
            "name": "bad",
            "enabled_events": ["v2.core.account.created"],
            "event_payload": "thin",
            "type": "amazon_eventbridge",
            "amazon_eventbridge": { "aws_account_id": "012345678910", "aws_region": "us-east-1" },
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
}

#[tokio::test]
async fn v2_account_creation_delivers_thin_event() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;

    let destination =
        create_destination(&server, &key, &receiver.url, &["v2.core.account.created"]).await;
    let secret = destination["webhook_endpoint"]["signing_secret"]
        .as_str()
        .unwrap()
        .to_string();
    let account = create_v2_account(&server, &key).await;

    let event = latest_v2_event(&server, &key).await;
    let event_id = event["id"].as_str().unwrap().to_string();
    assert!(event_id.starts_with("evt_test_"));
    assert_eq!(event["type"], "v2.core.account.created");
    assert_eq!(event["related_object"]["id"], account);
    assert_eq!(event["related_object"]["type"], "v2.core.account");
    assert_eq!(
        event["related_object"]["url"],
        format!("/v2/core/accounts/{account}")
    );
    // Fetched events expose the envelope fields.
    assert!(event.get("data").is_some());
    assert!(event.get("changes").is_some());
    assert!(event.get("reason").is_some());
    assert!(event.get("context").is_some());

    // The synchronous event exists before any delivery settles.
    assert_eq!(received_count(&receiver).await, 0);

    let waited = wait_delivery(&server, &event_id).await;
    assert_eq!(waited["settled"], true);
    assert_eq!(waited["deliveries"][0]["status"], "succeeded");

    let state = receiver.state.lock().await;
    assert_eq!(state.received.len(), 1);
    let delivery = &state.received[0];
    assert_eq!(delivery.body["object"], "v2.core.event");
    assert_eq!(delivery.body["type"], "v2.core.account.created");
    assert_eq!(delivery.body["related_object"]["id"], account);
    // The thin payload omits the fetched-only envelope fields.
    assert!(delivery.body.get("data").is_none());
    assert!(delivery.body.get("changes").is_none());
    assert!(delivery.body.get("reason").is_none());
    assert_signature(&secret, delivery);
}

#[tokio::test]
async fn non_matching_and_disabled_destinations_are_skipped() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;

    // Matching by wildcard, plus one that does not match and one that is disabled.
    create_destination(&server, &key, &receiver.url, &["v2.core.account.*"]).await;
    let mismatch =
        create_destination(&server, &key, &receiver.url, &["v2.core.account.updated"]).await;
    let disabled = create_destination(&server, &key, &receiver.url, &["*"]).await;
    let disabled_id = disabled["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!(
            "/v2/core/event_destinations/{disabled_id}/disable"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    create_v2_account(&server, &key).await;
    let event = latest_v2_event(&server, &key).await;
    let event_id = event["id"].as_str().unwrap().to_string();

    let waited = wait_delivery(&server, &event_id).await;
    assert_eq!(waited["settled"], true);
    // Exactly one delivery, for the wildcard destination.
    assert_eq!(waited["deliveries"].as_array().unwrap().len(), 1);
    let target = waited["deliveries"][0]["destination"].as_str().unwrap();
    assert_ne!(target, mismatch["id"].as_str().unwrap());
    assert_ne!(target, disabled_id);
    assert_eq!(received_count(&receiver).await, 1);
}

#[tokio::test]
async fn account_update_emits_thin_updated_event() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;
    create_destination(&server, &key, &receiver.url, &["v2.core.account.*"]).await;
    let account = create_v2_account(&server, &key).await;

    // Drain the account.created delivery first.
    let created = latest_v2_event(&server, &key).await;
    wait_delivery(&server, created["id"].as_str().unwrap()).await;
    let before = received_count(&receiver).await;

    // Updating an already-created account must emit a thin change event.
    let updated: Value = server
        .client
        .post(server.url(&format!("/v2/core/accounts/{account}")))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({ "metadata": { "note": "hello" } }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["metadata"]["note"], "hello");

    let event = latest_event_of_type(&server, &key, "v2.core.account.updated").await;
    assert!(event["id"].as_str().unwrap().starts_with("evt_test_"));
    assert_eq!(event["related_object"]["id"], account);
    wait_delivery(&server, event["id"].as_str().unwrap()).await;
    assert!(received_count(&receiver).await > before);

    let state = receiver.state.lock().await;
    assert!(
        state
            .received
            .iter()
            .any(|d| d.body["type"] == "v2.core.account.updated")
    );
}

#[tokio::test]
async fn onboarding_existing_account_emits_thin_capability_events() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;
    create_destination(&server, &key, &receiver.url, &["*"]).await;
    let account = create_v2_account(&server, &key).await;
    let created = latest_v2_event(&server, &key).await;
    wait_delivery(&server, created["id"].as_str().unwrap()).await;

    onboard_account(&server, &key, &account).await;

    let capability = latest_event_of_type(
        &server,
        &key,
        "v2.core.account[configuration.merchant].capability_status_updated",
    )
    .await;
    assert_eq!(capability["data"]["updated_capability"], "card_payments");
    assert_eq!(capability["related_object"]["id"], account);
    wait_delivery(&server, capability["id"].as_str().unwrap()).await;

    let updated = latest_event_of_type(&server, &key, "v2.core.account.updated").await;
    wait_delivery(&server, updated["id"].as_str().unwrap()).await;

    let state = receiver.state.lock().await;
    assert!(
        state.received.iter().any(|d| d.body["type"]
            == "v2.core.account[configuration.merchant].capability_status_updated")
    );
}

#[tokio::test]
async fn config_declared_destination_receives_thin_events() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_thin_destination(receiver.url.clone()).await;
    let key = main_secret(&server).await;

    let account = create_v2_account(&server, &key).await;
    let event = latest_v2_event(&server, &key).await;
    wait_delivery(&server, event["id"].as_str().unwrap()).await;

    assert_eq!(received_count(&receiver).await, 1);
    let state = receiver.state.lock().await;
    assert_eq!(state.received[0].body["object"], "v2.core.event");
    assert_eq!(state.received[0].body["type"], "v2.core.account.created");
    assert_eq!(state.received[0].body["related_object"]["id"], account);
}

#[tokio::test]
async fn v2_account_update_emits_v1_and_v2_events_with_stripe_scopes() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;

    async fn destination(
        server: &TestServer,
        key: &str,
        url: &str,
        name: &str,
        payload: &str,
        from: &[&str],
    ) {
        let response = server
            .client
            .post(server.url("/v2/core/event_destinations"))
            .header("Authorization", format!("Bearer {key}"))
            .json(&json!({
                "name": name,
                "type": "webhook_endpoint",
                "event_payload": payload,
                "events_from": from,
                "enabled_events": ["*"],
                "webhook_endpoint": { "url": url },
            }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
    destination(
        &server,
        &key,
        &receiver.url,
        "self-thin",
        "thin",
        &["@self"],
    )
    .await;
    destination(
        &server,
        &key,
        &receiver.url,
        "accounts-thin",
        "thin",
        &["@accounts"],
    )
    .await;
    destination(
        &server,
        &key,
        &receiver.url,
        "accounts-snapshot",
        "snapshot",
        &["@accounts"],
    )
    .await;

    let account = create_v2_account(&server, &key).await;
    let created = latest_v2_event(&server, &key).await;
    wait_delivery(&server, created["id"].as_str().unwrap()).await;

    // Update the account, then let every delivery settle.
    server
        .client
        .post(server.url(&format!("/v2/core/accounts/{account}")))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({ "metadata": { "note": "hello" } }))
        .send()
        .await
        .unwrap();
    server
        .client
        .post(server.url("/internal/deliveries/flush"))
        .json(&json!({ "timeout_ms": 8000 }))
        .send()
        .await
        .unwrap();

    let state = receiver.state.lock().await;
    let count = |event_type: &str| {
        state
            .received
            .iter()
            .filter(|d| d.body["type"] == event_type)
            .count()
    };
    let snapshot_account_updated = state
        .received
        .iter()
        .filter(|d| d.body["type"] == "account.updated" && d.body["object"] == "event")
        .count();

    // v2 Account update emits all three forms, each to its Stripe scope only:
    // `v2.core.account.updated` -> Your account (@self),
    // `v1.account.updated` and `account.updated` -> Connected accounts (@accounts).
    assert_eq!(count("v2.core.account.updated"), 1);
    assert_eq!(count("v1.account.updated"), 1);
    assert_eq!(snapshot_account_updated, 1);
    assert_eq!(count("v2.core.account.created"), 1);
}

#[tokio::test]
async fn v1_events_do_not_include_v2_thin_events() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    create_v2_account(&server, &key).await;

    let v1: Value = server
        .client
        .get(server.url("/v1/events"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        v1["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["type"] != "v2.core.account.created")
    );
}

#[tokio::test]
async fn snapshot_destination_does_not_receive_v2_thin_events() {
    // The configured snapshot destination points at the receiver; creating a v2
    // account must not deliver a v2 thin event to a snapshot destination.
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;

    create_v2_account(&server, &key).await;

    server
        .client
        .post(server.url("/internal/deliveries/flush"))
        .json(&json!({ "timeout_ms": 1500 }))
        .send()
        .await
        .unwrap();
    assert_eq!(received_count(&receiver).await, 0);

    let deliveries: Value = server
        .client
        .get(server.url("/internal/deliveries"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // A snapshot destination never matches a thin event, so there are no deliveries.
    assert!(deliveries["data"].as_array().unwrap().is_empty());
}

async fn make_destination(
    server: &TestServer,
    key: &str,
    url: &str,
    payload: &str,
    from: &[&str],
) -> String {
    let created: Value = server
        .client
        .post(server.url("/v2/core/event_destinations"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({
            "name": format!("{payload}-dest"),
            "type": "webhook_endpoint",
            "event_payload": payload,
            "events_from": from,
            "enabled_events": ["*"],
            "webhook_endpoint": { "url": url },
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    created["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn ping_only_reaches_thin_destinations() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;
    let snapshot = make_destination(&server, &key, &receiver.url, "snapshot", &["@self"]).await;

    let ping = server
        .client
        .post(server.url(&format!("/v2/core/event_destinations/{snapshot}/ping")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert!(ping.status().is_success());
    server
        .client
        .post(server.url("/internal/deliveries/flush"))
        .json(&json!({ "timeout_ms": 8000 }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        received_count(&receiver).await,
        0,
        "snapshot destination must not get a thin ping"
    );
}

#[tokio::test]
async fn platform_lists_and_retrieves_connected_scope_thin_events() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let account = create_v2_account(&server, &key).await;
    server
        .client
        .post(server.url(&format!("/v2/core/accounts/{account}")))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({ "metadata": { "note": "x" } }))
        .send()
        .await
        .unwrap();

    let event = latest_event_of_type(&server, &key, "v1.account.updated").await;
    assert!(
        event.is_object(),
        "platform should list v1.account.updated for a connected account"
    );
    let id = event["id"].as_str().unwrap();
    let fetched = server
        .client
        .get(server.url(&format!("/v2/core/events/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert!(fetched.status().is_success());
}

#[tokio::test]
async fn snapshot_pending_webhooks_counts_deliveries() {
    let receiver = spawn_receiver().await;
    let server = spawn().await;
    let key = main_secret(&server).await;
    make_destination(&server, &key, &receiver.url, "snapshot", &["@self"]).await;
    make_destination(&server, &key, &receiver.url, "snapshot", &["@self"]).await;

    let created: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "a@example.com")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(created["id"].is_string());
    server
        .client
        .post(server.url("/internal/deliveries/flush"))
        .json(&json!({ "timeout_ms": 8000 }))
        .send()
        .await
        .unwrap();
    let state = receiver.state.lock().await;
    for d in state
        .received
        .iter()
        .filter(|d| d.body["object"] == "event")
    {
        assert!(d.body["pending_webhooks"].as_i64().unwrap() >= 1);
    }
}

#[tokio::test]
async fn retrieve_honors_indexed_and_dotted_include() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let account = create_v2_account(&server, &key).await;
    server
        .client
        .post(server.url(&format!("/v2/core/accounts/{account}")))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({ "configuration": { "merchant": { "capabilities": { "card_payments": { "requested": true } } } } }))
        .send()
        .await
        .unwrap();
    onboard_account(&server, &key, &account).await;

    // stripe-node serializes include as `include[0]=configuration.merchant`.
    for query in [
        "include%5B0%5D=configuration.merchant",
        "include[]=configuration.merchant",
        "include=configuration",
    ] {
        let got: Value = server
            .client
            .get(server.url(&format!("/v2/core/accounts/{account}?{query}")))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            got["configuration"]["merchant"]["capabilities"]["card_payments"]["status"], "active",
            "query {query}: {got}"
        );
    }
    let bare: Value = server
        .client
        .get(server.url(&format!("/v2/core/accounts/{account}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(bare["configuration"].is_null());
}
