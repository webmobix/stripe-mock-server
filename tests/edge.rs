mod common;

use common::*;
use serde_json::{Value, json};

async fn connected_account(server: &TestServer, key: &str) -> String {
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

#[tokio::test]
async fn unknown_stripe_account_header_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let response = server
        .client
        .get(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", "acct_does_not_exist")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "authentication_error");
}

#[tokio::test]
async fn v2_account_list_paginates() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    for _ in 0..3 {
        connected_account(&server, &key).await;
    }
    let page: Value = server
        .client
        .get(server.url("/v2/core/accounts?limit=2"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["object"], "list");
    assert_eq!(page["data"].as_array().unwrap().len(), 2);
    assert_eq!(page["has_more"], true);
    assert!(page["next_page_url"].is_string());
}

#[tokio::test]
async fn v1_list_envelopes_paginate_newest_first() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let mut ids = Vec::new();
    for i in 0..3 {
        let product: Value = server
            .client
            .post(server.url("/v1/products"))
            .header("Authorization", format!("Bearer {key}"))
            .form(&[("name", format!("P{i}").as_str())])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        ids.push(product["id"].as_str().unwrap().to_string());
    }
    let page: Value = server
        .client
        .get(server.url("/v1/products?limit=2"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let data = page["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(page["has_more"], true);
    // newest first
    assert_eq!(data[0]["id"], ids[2]);
    assert_eq!(data[1]["id"], ids[1]);

    let next: Value = server
        .client
        .get(server.url(&format!("/v1/products?limit=2&starting_after={}", ids[1])))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let next_data = next["data"].as_array().unwrap();
    assert_eq!(next_data.len(), 1);
    assert_eq!(next_data[0]["id"], ids[0]);
    assert_eq!(next["has_more"], false);
}
