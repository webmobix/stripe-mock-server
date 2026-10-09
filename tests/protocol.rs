mod common;

use common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn health_probe_answers() {
    let server = spawn().await;
    let response = server
        .client
        .get(server.url("/health"))
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
}

#[tokio::test]
async fn missing_key_is_rejected_with_v1_envelope() {
    let server = spawn().await;
    let response = server
        .client
        .get(server.url("/v1/products"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "authentication_error");
}

#[tokio::test]
async fn unknown_key_is_rejected() {
    let server = spawn().await;
    let response = server
        .client
        .get(server.url("/v1/products"))
        .header("Authorization", "Bearer sk_test_does_not_exist")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn missing_required_param_on_v1_reports_param() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let response = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("description", "no name")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "name");
}

#[tokio::test]
async fn invalid_v2_request_uses_v2_envelope() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let response = server
        .client
        .post(server.url("/v2/core/accounts"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert!(body["error"]["code"].is_string());
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn v2_account_include_and_v1_projection() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let created: Value = server
        .client
        .post(server.url("/v2/core/accounts"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({
            "identity": { "business_details": { "name": "Acme" } },
            "configuration": { "merchant": { "capabilities": { "card_payments": { "requested": true } } } },
            "defaults": { "currency": "usd" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let id = created["id"].as_str().unwrap();
    assert!(id.starts_with("acct_"));
    assert_eq!(created["object"], "v2.core.account");
    assert!(
        created["applied_configurations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "merchant")
    );
    assert!(created["created"].as_str().unwrap().contains('T'));

    // Default-null without include.
    let plain: Value = server
        .client
        .get(server.url(&format!("/v2/core/accounts/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(plain["identity"].is_null());
    assert!(plain["requirements"].is_null());
    assert!(plain["configuration"].is_null());

    // Populated with include.
    let included: Value = server
        .client
        .get(server.url(&format!(
            "/v2/core/accounts/{id}?include[]=identity&include[]=requirements"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(included["identity"]["business_details"]["name"], "Acme");
    assert!(included["requirements"].is_object());

    // v1 projection uses epoch seconds.
    let v1: Value = server
        .client
        .get(server.url(&format!("/v1/accounts/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v1["object"], "account");
    assert!(v1["created"].is_i64());
    assert!(v1["charges_enabled"].is_boolean());
    assert_eq!(v1["details_submitted"], false);
}

#[tokio::test]
async fn account_links_present_per_version() {
    let server = spawn().await;
    let key = main_secret(&server).await;

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
    let id = created["id"].as_str().unwrap().to_string();

    let v1: Value = server
        .client
        .post(server.url("/v1/account_links"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("account", id.as_str()),
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
    assert_eq!(v1["object"], "account_link");
    assert!(v1["created"].is_i64());
    assert!(v1["url"].as_str().unwrap().starts_with(&server.base_url));

    let v2: Value = server
        .client
        .post(server.url("/v2/core/account_links"))
        .header("Authorization", format!("Bearer {key}"))
        .json(&json!({
            "account": id,
            "use_case": { "account_onboarding": { "refresh_url": "https://example.com/refresh", "return_url": "https://example.com/return" } }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v2["object"], "v2.core.account_link");
    assert!(v2["created"].as_str().unwrap().contains('T'));
}
