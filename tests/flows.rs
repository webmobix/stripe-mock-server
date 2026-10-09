mod common;

use common::*;
use serde_json::{Value, json};

async fn create_connected_account(server: &TestServer, key: &str) -> String {
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
    assert_eq!(created["object"], "v2.core.account");
    created["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn onboarding_pass_and_fail() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let account = create_connected_account(&server, &key).await;

    // v1 link
    let link: Value = server
        .client
        .post(server.url("/v1/account_links"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("account", account.as_str()),
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
    let url = link["url"].as_str().unwrap().to_string();
    let token = url.rsplit('/').next().unwrap().to_string();

    // The page is server-rendered with CDN assets and no framework bundle.
    let page = server.client.get(&url).send().await.unwrap();
    let html = page.text().await.unwrap();
    assert!(html.contains("cdn.tailwindcss.com"));
    assert!(html.contains("daisyui"));
    assert!(html.contains("KYC pass"));
    assert!(!html.contains("Phone authentication"));
    assert!(!html.contains("react"));

    // Before onboarding the requested capability reads as pending in the v2 view.
    let pending: Value = server
        .client
        .get(server.url(&format!(
            "/v2/core/accounts/{account}?include[]=configuration"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        pending["configuration"]["merchant"]["capabilities"]["card_payments"]["status"],
        "pending"
    );

    // Passing KYC redirects to return_url and completes onboarding.
    let response = server
        .client
        .post(server.url(&format!("/mock/connect/{token}")))
        .form(&[("kyc", "pass"), ("business_name", "Acme")])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_redirection());
    assert_eq!(response.headers()["location"], "https://example.com/return");

    let account_v1: Value = server
        .client
        .get(server.url(&format!("/v1/accounts/{account}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(account_v1["details_submitted"], true);
    assert_eq!(account_v1["charges_enabled"], true);
    assert_eq!(account_v1["capabilities"]["card_payments"], "active");

    // The canonical v2 configuration reports the capability status as active.
    let account_v2: Value = server
        .client
        .get(server.url(&format!(
            "/v2/core/accounts/{account}?include[]=configuration"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        account_v2["configuration"]["merchant"]["capabilities"]["card_payments"]["status"],
        "active"
    );

    // account.updated event exists for the connected account.
    let events: Value = server
        .client
        .get(server.url("/v1/events?type=account.updated"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        events["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["account"] == account.as_str())
    );

    // Reusing a stale link redirects to refresh_url.
    let response = server
        .client
        .post(server.url(&format!("/mock/connect/{token}")))
        .form(&[("kyc", "pass")])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_redirection());
    assert_eq!(
        response.headers()["location"],
        "https://example.com/refresh"
    );

    // Failing KYC on a fresh account keeps details_submitted false.
    let account2 = create_connected_account(&server, &key).await;
    let link2: Value = server
        .client
        .post(server.url("/v1/account_links"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("account", account2.as_str()),
            ("type", "account_onboarding"),
            ("refresh_url", "https://example.com/refresh2"),
            ("return_url", "https://example.com/return2"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token2 = link2["url"]
        .as_str()
        .unwrap()
        .rsplit('/')
        .next()
        .unwrap()
        .to_string();
    server
        .client
        .post(server.url(&format!("/mock/connect/{token2}")))
        .form(&[("kyc", "fail")])
        .send()
        .await
        .unwrap();
    let account2_v1: Value = server
        .client
        .get(server.url(&format!("/v1/accounts/{account2}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(account2_v1["details_submitted"], false);
    assert!(
        account2_v1["requirements"]["currently_due"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "business_profile.url")
    );
}

#[tokio::test]
async fn catalog_crud_and_isolation() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Widget"), ("description", "A widget")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    assert!(product_id.starts_with("prod_"));
    assert_eq!(product["active"], true);
    assert!(product["created"].is_i64());

    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product_id.as_str()),
            ("currency", "usd"),
            ("unit_amount", "1000"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price_id = price["id"].as_str().unwrap().to_string();
    assert!(price_id.starts_with("price_"));
    assert_eq!(price["type"], "one_time");
    assert_eq!(price["unit_amount"], 1000);

    // Creating a price updates the product default_price.
    let refreshed: Value = server
        .client
        .get(server.url(&format!("/v1/products/{product_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refreshed["default_price"], price_id);

    let recurring: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product_id.as_str()),
            ("currency", "usd"),
            ("unit_amount", "2000"),
            ("recurring[interval]", "month"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recurring["type"], "recurring");
    assert_eq!(recurring["recurring"]["interval"], "month");

    // Connected-account catalog isolation.
    let account = create_connected_account(&server, &key).await;
    let isolated: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .form(&[("name", "Connected Widget")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let isolated_id = isolated["id"].as_str().unwrap().to_string();
    let missing = server
        .client
        .get(server.url(&format!("/v1/products/{isolated_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn billing_subscription_lifecycle() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "buyer@example.com"), ("name", "Buyer")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let customer_id = customer["id"].as_str().unwrap().to_string();
    assert!(customer_id.starts_with("cus_"));
    assert_eq!(customer["email"], "buyer@example.com");

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product_id.as_str()),
            ("currency", "usd"),
            ("unit_amount", "1500"),
            ("recurring[interval]", "month"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price_id = price["id"].as_str().unwrap().to_string();

    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer_id.as_str()),
            ("items[0][price]", price_id.as_str()),
            ("items[0][quantity]", "1"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription_id = subscription["id"].as_str().unwrap().to_string();
    assert!(subscription_id.starts_with("sub_"));
    assert_eq!(subscription["status"], "active");
    assert_eq!(subscription["items"]["data"][0]["price"]["id"], price_id);

    let listed: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions?customer={customer_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == subscription_id)
    );

    // customer_account reference.
    let account = create_connected_account(&server, &key).await;
    let account_sub: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer_account", account.as_str()),
            ("items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(account_sub["customer_account"], account);

    // Cancel emits customer.subscription.deleted and flips status.
    let canceled: Value = server
        .client
        .delete(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(canceled["status"], "canceled");

    let events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.subscription.deleted"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!events["data"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn checkout_payment_flow_and_expiry() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "T-Shirt")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product_id.as_str()),
            ("unit_amount", "2500"),
            ("currency", "usd"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price_id = price["id"].as_str().unwrap().to_string();

    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("line_items[0][price]", price_id.as_str()),
            ("line_items[0][quantity]", "2"),
            (
                "success_url",
                "https://example.com/success?session={CHECKOUT_SESSION_ID}",
            ),
            ("cancel_url", "https://example.com/cancel"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = session["id"].as_str().unwrap().to_string();
    assert!(session_id.starts_with("cs_test_"));
    assert_eq!(session["status"], "open");
    assert_eq!(session["amount_total"], 5000);
    assert!(session["url"].as_str().unwrap().contains("/mock/checkout/"));

    let line_items: Value = server
        .client
        .get(server.url(&format!("/v1/checkout/sessions/{session_id}/line_items")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(line_items["object"], "list");
    assert_eq!(line_items["data"].as_array().unwrap().len(), 1);

    // Mock page renders.
    let page = server
        .client
        .get(server.url(&format!("/mock/checkout/{session_id}")))
        .send()
        .await
        .unwrap();
    assert!(page.status().is_success());
    let html = page.text().await.unwrap();
    assert!(html.contains("Successful card"));

    // Completing with a successful card redirects to success_url with substitution.
    let response = server
        .client
        .post(server.url(&format!("/mock/checkout/{session_id}")))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_redirection());
    assert_eq!(
        response.headers()["location"],
        format!("https://example.com/success?session={session_id}")
    );

    let completed: Value = server
        .client
        .get(server.url(&format!("/v1/checkout/sessions/{session_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(completed["status"], "complete");
    assert!(
        completed["payment_intent"]
            .as_str()
            .unwrap()
            .starts_with("pi_")
    );

    for event_type in ["checkout.session.completed", "payment_intent.succeeded"] {
        let events: Value = server
            .client
            .get(server.url(&format!("/v1/events?type={event_type}")))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            !events["data"].as_array().unwrap().is_empty(),
            "missing {event_type}"
        );
    }

    // Expiry path.
    let session2: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("line_items[0][price]", price_id.as_str()),
            ("success_url", "https://example.com/success"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session2_id = session2["id"].as_str().unwrap().to_string();
    let expired: Value = server
        .client
        .post(server.url(&format!("/v1/checkout/sessions/{session2_id}/expire")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(expired["status"], "expired");
}

#[tokio::test]
async fn price_list_filter_and_checkout_modes() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    async fn make_product(server: &TestServer, key: &str, name: &str) -> String {
        let product: Value = server
            .client
            .post(server.url("/v1/products"))
            .header("Authorization", format!("Bearer {key}"))
            .form(&[("name", name)])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        product["id"].as_str().unwrap().to_string()
    }
    async fn make_price(
        server: &TestServer,
        key: &str,
        product: &str,
        amount: i64,
        recurring: bool,
    ) -> String {
        let mut form = vec![
            ("product".to_string(), product.to_string()),
            ("unit_amount".to_string(), amount.to_string()),
            ("currency".to_string(), "usd".to_string()),
        ];
        if recurring {
            form.push(("recurring[interval]".to_string(), "month".to_string()));
        }
        let price: Value = server
            .client
            .post(server.url("/v1/prices"))
            .header("Authorization", format!("Bearer {key}"))
            .form(&form)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        price["id"].as_str().unwrap().to_string()
    }

    let product_a = make_product(&server, &key, "A").await;
    let product_b = make_product(&server, &key, "B").await;
    let price_a = make_price(&server, &key, &product_a, 1000, false).await;
    let _price_b = make_price(&server, &key, &product_b, 2000, false).await;

    let filtered: Value = server
        .client
        .get(server.url(&format!("/v1/prices?product={product_a}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let data = filtered["data"].as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["id"], price_a);

    // Subscription-mode checkout completes into a subscription.
    let recurring_price = make_price(&server, &key, &product_a, 1500, true).await;
    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "subscription"),
            ("line_items[0][price]", recurring_price.as_str()),
            ("success_url", "https://example.com/success"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = session["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/mock/checkout/{session_id}")))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    let completed: Value = server
        .client
        .get(server.url(&format!("/v1/checkout/sessions/{session_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(completed["mode"], "subscription");
    assert_eq!(completed["status"], "complete");
    assert!(
        completed["subscription"]
            .as_str()
            .unwrap()
            .starts_with("sub_")
    );

    // Setup-mode session completes with no payment intent.
    let setup_session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "setup"),
            ("success_url", "https://example.com/success"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let setup_id = setup_session["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/mock/checkout/{setup_id}")))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    let setup_done: Value = server
        .client
        .get(server.url(&format!("/v1/checkout/sessions/{setup_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(setup_done["status"], "complete");
    assert!(setup_done["payment_intent"].is_null());
}
