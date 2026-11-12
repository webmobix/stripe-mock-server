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

    // Shared developer-tooling chrome and panel framing are gone on all pages.
    assert!(!html.contains("TEST MODE"));
    assert!(!html.contains("Local simulation tool. Never use in production."));
    assert!(!html.contains("Simulation panel"));
    assert!(!html.contains("developer-mode simulator"));
    // The onboarding controls remain available.
    assert!(html.contains("KYC fail"));
    assert!(html.contains("Payout IBAN"));

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
    assert!(html.contains("Successful"));
    assert!(html.contains("Declined"));
    assert!(html.contains("Simulate payment"));
    assert!(html.contains("id=\"payment-success\""));
    assert!(html.contains("id=\"payment-decline\""));
    assert!(html.contains("id=\"payment-cancel\""));
    assert!(!html.contains("Successful card"));
    assert!(!html.contains("Declined card"));

    // The cancel control navigates to the session's cancel_url.
    let cancel = server
        .client
        .get(server.url(&format!("/mock/checkout/{session_id}/cancel")))
        .send()
        .await
        .unwrap();
    assert!(cancel.status().is_redirection());
    assert_eq!(cancel.headers()["location"], "https://example.com/cancel");

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

#[tokio::test]
async fn static_checkout_session_is_always_available() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    // Seeded at boot with the fixed id and demo content, no session creation needed.
    let (status, mode, amount_total, line_items): (String, String, i64, String) =
        sqlx::query_as(
            "SELECT status, mode, amount_total, line_items FROM checkout_sessions WHERE id = 'cs_test_static'",
        )
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(status, "open");
    assert_eq!(mode, "payment");
    assert_eq!(amount_total, 2000);
    let line_items: Value = serde_json::from_str(&line_items).unwrap();
    assert_eq!(line_items.as_array().unwrap().len(), 1);
    assert_eq!(line_items[0]["description"], "Static demo item");

    // The page renders without creating a session first.
    let page = server
        .client
        .get(server.url("/mock/checkout/cs_test_static"))
        .send()
        .await
        .unwrap();
    assert!(page.status().is_success());
    let html = page.text().await.unwrap();
    assert!(html.contains("20.00"));
    assert!(html.contains("Successful"));
    assert!(html.contains("Declined"));
    assert!(!html.contains("Successful card"));
    assert!(!html.contains("Declined card"));
    assert!(html.contains("id=\"payment-success\""));
    assert!(html.contains("id=\"payment-decline\""));
    assert!(html.contains("id=\"payment-cancel\""));
    // Removed tooling chrome and panel framing are absent from the static page.
    assert!(!html.contains("TEST MODE"));
    assert!(!html.contains("Local simulation tool. Never use in production."));
    assert!(!html.contains("Simulation panel"));
    assert!(!html.contains("developer-mode simulator"));

    // Completing reuses the normal completion flow.
    let response = server
        .client
        .post(server.url("/mock/checkout/cs_test_static"))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());

    let session: Value = server
        .client
        .get(server.url("/v1/checkout/sessions/cs_test_static"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["status"], "complete");
    assert_eq!(session["payment_status"], "paid");

    let pi_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payment_intents WHERE checkout_session = 'cs_test_static' AND status = 'succeeded'",
    )
    .fetch_one(&server.pool)
    .await
    .unwrap();
    assert!(pi_count >= 1);
    let completed_events: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE type = 'checkout.session.completed'")
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert!(completed_events >= 1);
    let pi_events: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE type = 'payment_intent.succeeded'")
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert!(pi_events >= 1);

    // Opening the page again re-opens a fresh demo session.
    let page = server
        .client
        .get(server.url("/mock/checkout/cs_test_static"))
        .send()
        .await
        .unwrap();
    assert!(page.status().is_success());
    assert!(page.text().await.unwrap().contains("Successful"));
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = 'cs_test_static'")
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert_eq!(status, "open");

    // Reset re-seeds it as open.
    let reset = server
        .client
        .post(server.url("/internal/reset"))
        .send()
        .await
        .unwrap();
    assert!(reset.status().is_success());
    let status: String =
        sqlx::query_scalar("SELECT status FROM checkout_sessions WHERE id = 'cs_test_static'")
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert_eq!(status, "open");
}

#[tokio::test]
async fn expansion_embeds_referenced_products() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Expanded Plan")])
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
            ("unit_amount", "1000"),
            ("recurring[interval]", "month"),
            ("lookup_key", "normal_monthly_flat"),
            ("expand[]", "product"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price_id = price["id"].as_str().unwrap().to_string();
    assert_eq!(price["product"]["id"], product_id);
    assert_eq!(price["product"]["object"], "product");

    let retrieved: Value = server
        .client
        .get(server.url(&format!("/v1/prices/{price_id}?expand[]=product")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retrieved["product"]["id"], product_id);

    let listed: Value = server
        .client
        .get(server.url("/v1/prices?expand[]=product"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["data"][0]["product"]["id"], product_id);

    let listed_by_envelope: Value = server
        .client
        .get(server.url(
            "/v1/prices?lookup_keys[]=normal_monthly_flat&expand[]=data.product",
        ))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed_by_envelope["data"].as_array().unwrap().len(), 1);
    assert_eq!(
        listed_by_envelope["data"][0]["product"]["id"],
        product_id
    );
    assert_eq!(
        listed_by_envelope["data"][0]["product"]["object"],
        "product"
    );

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "expand@example.com")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let customer_id = customer["id"].as_str().unwrap().to_string();

    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer_id.as_str()),
            ("items[0][price]", price_id.as_str()),
            ("expand[]", "items.data.price.product"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription_id = subscription["id"].as_str().unwrap().to_string();
    assert_eq!(
        subscription["items"]["data"][0]["price"]["product"]["id"],
        product_id
    );

    let retrieved: Value = server
        .client
        .get(server.url(&format!(
            "/v1/subscriptions/{subscription_id}?expand[]=items.data.price.product"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        retrieved["items"]["data"][0]["price"]["product"]["object"],
        "product"
    );

    let listed: Value = server
        .client
        .get(server.url(
            "/v1/subscriptions?expand[]=data.items.data.price.product",
        ))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed_subscription = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == subscription_id.as_str())
        .unwrap();
    assert_eq!(
        listed_subscription["items"]["data"][0]["price"]["product"]["id"],
        product_id
    );
    assert_eq!(
        listed_subscription["items"]["data"][0]["price"]["product"]["object"],
        "product"
    );

    let listed_item_relative: Value = server
        .client
        .get(server.url(
            "/v1/subscriptions?expand[]=items.data.price.product",
        ))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed_item_relative_subscription = listed_item_relative["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == subscription_id.as_str())
        .unwrap();
    assert_eq!(
        listed_item_relative_subscription["items"]["data"][0]["price"]["product"]["object"],
        "product"
    );

    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("line_items[0][price]", price_id.as_str()),
            ("expand[]", "line_items.data.price.product"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let session_id = session["id"].as_str().unwrap().to_string();
    assert_eq!(
        session["line_items"]["data"][0]["price"]["product"]["id"],
        product_id
    );

    let retrieved: Value = server
        .client
        .get(server.url(&format!(
            "/v1/checkout/sessions/{session_id}?expand[]=line_items.data.price.product"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        retrieved["line_items"]["data"][0]["price"]["product"]["object"],
        "product"
    );

    let lines: Value = server
        .client
        .get(server.url(&format!(
            "/v1/checkout/sessions/{session_id}/line_items?expand[]=data.price.product"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(lines["data"][0]["price"]["product"]["id"], product_id);
}

#[tokio::test]
async fn price_lookup_keys_are_unique_and_filterable() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Lookup Plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    let create_price = |lookup: Option<String>, transfer: bool| {
        let server_url = server.url("/v1/prices");
        let client = server.client.clone();
        let key = key.clone();
        let product_id = product_id.clone();
        async move {
            let mut form = vec![
                ("product", product_id.clone()),
                ("currency", "usd".to_string()),
                ("unit_amount", "1000".to_string()),
            ];
            if let Some(lookup) = lookup {
                form.push(("lookup_key", lookup));
            }
            if transfer {
                form.push(("transfer_lookup_key", "true".to_string()));
            }
            client
                .post(server_url)
                .header("Authorization", format!("Bearer {key}"))
                .form(&form)
                .send()
                .await
                .unwrap()
        }
    };

    let first: Value = create_price(Some("pro_monthly".to_string()), false)
        .await
        .json()
        .await
        .unwrap();
    let first_id = first["id"].as_str().unwrap().to_string();
    assert_eq!(first["lookup_key"], "pro_monthly");

    let plain: Value = create_price(None, false).await.json().await.unwrap();
    assert_eq!(plain["lookup_key"], Value::Null);

    // A duplicate key is rejected and creates no price.
    let duplicate = create_price(Some("pro_monthly".to_string()), false).await;
    assert_eq!(duplicate.status(), 400);
    let duplicate_body: Value = duplicate.json().await.unwrap();
    assert_eq!(duplicate_body["error"]["param"], "lookup_key");
    let all: Value = server
        .client
        .get(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all["data"].as_array().unwrap().len(), 2);

    // Transferring clears the previous holder's key.
    let transferred: Value = create_price(Some("pro_monthly".to_string()), true)
        .await
        .json()
        .await
        .unwrap();
    let transferred_id = transferred["id"].as_str().unwrap().to_string();
    assert_eq!(transferred["lookup_key"], "pro_monthly");
    let previous: Value = server
        .client
        .get(server.url(&format!("/v1/prices/{first_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(previous["lookup_key"], Value::Null);

    // Filtering by repeated lookup keys returns only matching prices.
    let team: Value = create_price(Some("team_monthly".to_string()), false)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(team["lookup_key"], "team_monthly");
    let filtered: Value = server
        .client
        .get(server.url("/v1/prices?lookup_keys[]=pro_monthly&lookup_keys[]=team_monthly"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = filtered["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["id"].as_str())
        .collect();
    assert!(ids.contains(&transferred_id.as_str()));
    assert!(ids.contains(&team["id"].as_str().unwrap()));
    assert_eq!(ids.len(), 2);

    let no_match: Value = server
        .client
        .get(server.url("/v1/prices?lookup_keys[]=missing_key"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(no_match["data"].as_array().unwrap().is_empty());

    // Connected accounts may reuse a key; neither account sees the other.
    let account = create_connected_account(&server, &key).await;
    let connected_product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .form(&[("name", "Connected Plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let connected_product_id = connected_product["id"].as_str().unwrap().to_string();
    let connected_price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .form(&[
            ("product", connected_product_id.as_str()),
            ("currency", "usd"),
            ("unit_amount", "1500"),
            ("lookup_key", "pro_monthly"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(connected_price["lookup_key"], "pro_monthly");
    let visible: Value = server
        .client
        .get(server.url("/v1/prices?lookup_keys[]=pro_monthly"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(visible["data"].as_array().unwrap().len(), 1);
    assert_eq!(visible["data"][0]["id"], transferred_id);
}

#[tokio::test]
async fn coupons_crud_validation_and_isolation() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let post_coupon = |form: Vec<(String, String)>, account: Option<String>| {
        let client = server.client.clone();
        let url = server.url("/v1/coupons");
        let key = key.clone();
        async move {
            let mut request = client
                .post(url)
                .header("Authorization", format!("Bearer {key}"))
                .form(&form);
            if let Some(account) = account {
                request = request.header("Stripe-Account", account);
            }
            request.send().await.unwrap()
        }
    };

    // Percentage coupon.
    let percent: Value = post_coupon(
        vec![
            ("percent_off".into(), "25".into()),
            ("name".into(), "Quarter off".into()),
            ("duration".into(), "forever".into()),
        ],
        None,
    )
    .await
    .json()
    .await
    .unwrap();
    let percent_id = percent["id"].as_str().unwrap().to_string();
    assert!(percent_id.starts_with("coupon_"));
    assert_eq!(percent["object"], "coupon");
    assert_eq!(percent["percent_off"], 25);
    assert_eq!(percent["valid"], true);
    assert_eq!(percent["times_redeemed"], 0);

    // Fixed-amount coupon.
    let amount: Value = post_coupon(
        vec![
            ("amount_off".into(), "500".into()),
            ("currency".into(), "usd".into()),
            ("duration".into(), "once".into()),
        ],
        None,
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(amount["amount_off"], 500);
    assert_eq!(amount["currency"], "usd");

    // Both or neither discount form.
    for form in [
        vec![
            ("percent_off".into(), "10".into()),
            ("amount_off".into(), "500".into()),
            ("currency".into(), "usd".into()),
        ],
        vec![("name".into(), "No discount".into())],
    ] {
        let response = post_coupon(form, None).await;
        assert_eq!(response.status(), 400);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
    }

    // Percentage bounds.
    for value in ["0", "101"] {
        let response = post_coupon(
            vec![("percent_off".into(), value.into())],
            None,
        )
        .await;
        assert_eq!(response.status(), 400, "percent_off {value} was accepted");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["param"], "percent_off");
    }

    // Amount without a currency.
    let response = post_coupon(vec![("amount_off".into(), "500".into())], None).await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "currency");

    // Repeating duration requires a month count.
    let response = post_coupon(
        vec![
            ("percent_off".into(), "10".into()),
            ("duration".into(), "repeating".into()),
        ],
        None,
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "duration_in_months");

    let repeating: Value = post_coupon(
        vec![
            ("percent_off".into(), "10".into()),
            ("duration".into(), "repeating".into()),
            ("duration_in_months".into(), "3".into()),
        ],
        None,
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(repeating["duration"], "repeating");
    assert_eq!(repeating["duration_in_months"], 3);

    // Unsupported duration.
    let response = post_coupon(
        vec![
            ("percent_off".into(), "10".into()),
            ("duration".into(), "weekly".into()),
        ],
        None,
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "duration");

    // Client-supplied id, product restriction, and duplicate rejection.
    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Discounted Plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let restricted: Value = post_coupon(
        vec![
            ("id".into(), "friends_only".into()),
            ("percent_off".into(), "10".into()),
            ("applies_to[products][0]".into(), product_id.clone()),
        ],
        None,
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(restricted["id"], "friends_only");
    assert_eq!(restricted["applies_to"]["products"][0], product_id);

    let duplicate = post_coupon(
        vec![
            ("id".into(), "friends_only".into()),
            ("percent_off".into(), "50".into()),
        ],
        None,
    )
    .await;
    assert_eq!(duplicate.status(), 400);
    let unchanged: Value = server
        .client
        .get(server.url("/v1/coupons/friends_only"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unchanged["percent_off"], 10);

    // Expired and fully redeemed coupons are invalid.
    sqlx::query("UPDATE coupons SET redeem_by = 1 WHERE id = ?1")
        .bind(&percent_id)
        .execute(&server.pool)
        .await
        .unwrap();
    let expired: Value = server
        .client
        .get(server.url(&format!("/v1/coupons/{percent_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(expired["valid"], false);

    let redeemed: Value = post_coupon(
        vec![
            ("id".into(), "one_use".into()),
            ("percent_off".into(), "10".into()),
            ("max_redemptions".into(), "1".into()),
        ],
        None,
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(redeemed["valid"], true);
    sqlx::query("UPDATE coupons SET times_redeemed = 1 WHERE id = 'one_use'")
        .execute(&server.pool)
        .await
        .unwrap();
    let spent: Value = server
        .client
        .get(server.url("/v1/coupons/one_use"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(spent["valid"], false);

    // Retrieve, pagination, and delete.
    let first_page: Value = server
        .client
        .get(server.url("/v1/coupons?limit=2"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(first_page["data"].as_array().unwrap().len(), 2);
    assert_eq!(first_page["has_more"], true);

    let deleted: Value = server
        .client
        .delete(server.url("/v1/coupons/friends_only"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);
    let gone = server
        .client
        .get(server.url("/v1/coupons/friends_only"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 404);
    let listed: Value = server
        .client
        .get(server.url("/v1/coupons"))
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
            .all(|coupon| coupon["id"] != "friends_only")
    );

    // Connected-account isolation.
    let account = create_connected_account(&server, &key).await;
    let isolated: Value = post_coupon(
        vec![
            ("id".into(), "connected_only".into()),
            ("percent_off".into(), "20".into()),
        ],
        Some(account.clone()),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(isolated["id"], "connected_only");
    let hidden = server
        .client
        .get(server.url("/v1/coupons/connected_only"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(hidden.status(), 404);
    let visible: Value = server
        .client
        .get(server.url("/v1/coupons/connected_only"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(visible["id"], "connected_only");
}

#[tokio::test]
async fn tax_rates_create_list_filters_and_isolation() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let post_rate = |form: Vec<(String, String)>| {
        let client = server.client.clone();
        let url = server.url("/v1/tax_rates");
        let key = key.clone();
        async move {
            client
                .post(url)
                .header("Authorization", format!("Bearer {key}"))
                .form(&form)
                .send()
                .await
                .unwrap()
        }
    };

    let exclusive: Value = post_rate(vec![
        ("display_name".into(), "VAT".into()),
        ("inclusive".into(), "false".into()),
        ("percentage".into(), "20".into()),
        ("country".into(), "DE".into()),
    ])
    .await
    .json()
    .await
    .unwrap();
    let exclusive_id = exclusive["id"].as_str().unwrap().to_string();
    assert!(exclusive_id.starts_with("txr_"));
    assert_eq!(exclusive["object"], "tax_rate");
    assert_eq!(exclusive["active"], true);
    assert_eq!(exclusive["inclusive"], false);
    assert_eq!(exclusive["percentage"], 20);

    let inclusive: Value = post_rate(vec![
        ("display_name".into(), "Included VAT".into()),
        ("inclusive".into(), "true".into()),
        ("percentage".into(), "10".into()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(inclusive["inclusive"], true);

    let inactive: Value = post_rate(vec![
        ("display_name".into(), "Old tax".into()),
        ("inclusive".into(), "false".into()),
        ("percentage".into(), "5".into()),
        ("active".into(), "false".into()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(inactive["active"], false);

    // Required fields and percentage bounds.
    for (form, param) in [
        (
            vec![
                ("inclusive".into(), "false".into()),
                ("percentage".into(), "20".into()),
            ],
            "display_name",
        ),
        (
            vec![
                ("display_name".into(), "VAT".into()),
                ("percentage".into(), "20".into()),
            ],
            "inclusive",
        ),
        (
            vec![
                ("display_name".into(), "VAT".into()),
                ("inclusive".into(), "false".into()),
                ("percentage".into(), "-1".into()),
            ],
            "percentage",
        ),
        (
            vec![
                ("display_name".into(), "VAT".into()),
                ("inclusive".into(), "false".into()),
                ("percentage".into(), "101".into()),
            ],
            "percentage",
        ),
    ] {
        let response = post_rate(form).await;
        assert_eq!(response.status(), 400);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["param"], param);
    }

    // Retrieve.
    let retrieved: Value = server
        .client
        .get(server.url(&format!("/v1/tax_rates/{exclusive_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retrieved["id"], exclusive_id);

    // List filters.
    let active: Value = server
        .client
        .get(server.url("/v1/tax_rates?active=true"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(active["data"].as_array().unwrap().len(), 2);
    assert!(
        active["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|rate| rate["active"] == true)
    );

    let exclusive_only: Value = server
        .client
        .get(server.url("/v1/tax_rates?inclusive=false"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        exclusive_only["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|rate| rate["inclusive"] == false)
    );

    // Unknown id.
    let missing = server
        .client
        .get(server.url("/v1/tax_rates/txr_missing"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 404);

    // Connected-account isolation.
    let account = create_connected_account(&server, &key).await;
    let connected: Value = server
        .client
        .post(server.url("/v1/tax_rates"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .form(&[
            ("display_name", "Connected VAT"),
            ("inclusive", "false"),
            ("percentage", "7"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let connected_id = connected["id"].as_str().unwrap().to_string();
    let hidden = server
        .client
        .get(server.url(&format!("/v1/tax_rates/{connected_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(hidden.status(), 404);
    let visible: Value = server
        .client
        .get(server.url(&format!("/v1/tax_rates/{connected_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .header("Stripe-Account", &account)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(visible["id"], connected_id);
}

#[tokio::test]
async fn subscription_updates_canceled_retrieval_and_prorations() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "seats@example.com")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let customer_id = customer["id"].as_str().unwrap().to_string();

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Seat Plan")])
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
            ("unit_amount", "1000"),
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
    let item_id = subscription["items"]["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let period_start = subscription["current_period_start"].as_i64().unwrap();
    let period_end = subscription["current_period_end"].as_i64().unwrap();
    assert_eq!(
        subscription["items"]["data"][0]["current_period_start"],
        period_start
    );
    assert_eq!(
        subscription["items"]["data"][0]["current_period_end"],
        period_end
    );
    assert_eq!(
        subscription["items"]["data"][0]["subscription"],
        subscription_id
    );

    // Increase the quantity by item id.
    let increased: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("items[0][id]", item_id.as_str()),
            ("items[0][quantity]", "5"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(increased["status"], "active");
    assert_eq!(increased["items"]["data"][0]["quantity"], 5);
    let proration = &increased["prorations"][0];
    assert!(proration["amount"].as_i64().unwrap() > 0);
    assert_eq!(proration["quantity_delta"], 4);
    assert_eq!(proration["item"], item_id);
    assert_eq!(proration["price"], price_id);
    assert_eq!(proration["currency"], "usd");
    assert_eq!(proration["period_start"], period_start);
    assert_eq!(proration["period_end"], period_end);

    // Decrease by price on a one-item subscription.
    let decreased: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("items[0][price]", price_id.as_str()),
            ("items[0][quantity]", "2"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(decreased["items"]["data"][0]["quantity"], 2);
    assert!(decreased["prorations"][0]["amount"].as_i64().unwrap() < 0);
    assert_eq!(decreased["prorations"][0]["quantity_delta"], -3);

    // Disabled prorations produce an empty array.
    let disabled: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("items[0][id]", item_id.as_str()),
            ("items[0][quantity]", "3"),
            ("proration_behavior", "none"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(disabled["prorations"], json!([]));

    // always_invoice returns figures but creates no invoice.
    let invoices_before: Value = server
        .client
        .get(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let always: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("items[0][id]", item_id.as_str()),
            ("items[0][quantity]", "4"),
            ("proration_behavior", "always_invoice"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(always["prorations"].as_array().unwrap().len(), 1);
    let invoices_after: Value = server
        .client
        .get(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        invoices_before["data"].as_array().unwrap().len(),
        invoices_after["data"].as_array().unwrap().len()
    );

    // Unsupported behavior, unknown item, and invalid quantity change nothing.
    for (form, param) in [
        (
            vec![
                ("items[0][id]".to_string(), item_id.clone()),
                ("items[0][quantity]".to_string(), "9".to_string()),
                ("proration_behavior".to_string(), "sometimes".to_string()),
            ],
            "proration_behavior",
        ),
        (
            vec![
                ("items[0][id]".to_string(), "si_missing".to_string()),
                ("items[0][quantity]".to_string(), "9".to_string()),
            ],
            "items[0][id]",
        ),
        (
            vec![
                ("items[0][id]".to_string(), item_id.clone()),
                ("items[0][quantity]".to_string(), "0".to_string()),
            ],
            "quantity",
        ),
    ] {
        let response = server
            .client
            .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
            .header("Authorization", format!("Bearer {key}"))
            .form(&form)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["param"], param);
    }
    let unchanged: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unchanged["items"]["data"][0]["quantity"], 4);

    // cancel_at_period_end and metadata without changing status.
    let canceling: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("cancel_at_period_end", "true"), ("metadata[note]", "keep")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(canceling["cancel_at_period_end"], true);
    assert_eq!(canceling["status"], "active");
    assert_eq!(canceling["metadata"]["note"], "keep");

    // The update emitted an event with the updated subscription.
    let events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.subscription.updated"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!events["data"].as_array().unwrap().is_empty());
    assert_eq!(events["data"][0]["data"]["object"]["id"], subscription_id);
    assert_eq!(events["data"][0]["data"]["object"]["items"]["data"][0]["quantity"], 4);

    // A canceled subscription stays retrievable and in the list.
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
    let retrieved: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retrieved["status"], "canceled");
    let listed: Value = server
        .client
        .get(server.url("/v1/subscriptions"))
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
}

#[tokio::test]
async fn checkout_customer_email_creation_and_events() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Checkout Plan")])
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
            ("unit_amount", "1000"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price_id = price["id"].as_str().unwrap().to_string();

    let customers_before: Value = server
        .client
        .get(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let before_count = customers_before["data"].as_array().unwrap().len();

    // Email only: a customer is created and linked.
    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("customer_email", "guest@example.com"),
            ("line_items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let customer_id = session["customer"].as_str().unwrap().to_string();
    assert!(customer_id.starts_with("cus_"));
    assert_eq!(session["customer_email"], "guest@example.com");
    assert_eq!(session["customer_details"]["email"], "guest@example.com");

    let customer: Value = server
        .client
        .get(server.url(&format!("/v1/customers/{customer_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customer["email"], "guest@example.com");

    let customers_after: Value = server
        .client
        .get(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        customers_after["data"].as_array().unwrap().len(),
        before_count + 1
    );

    let events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.created"))
        .header("Authorization", format!("Bearer {key}"))
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
            .any(|event| event["data"]["object"]["id"] == customer_id)
    );

    // Retrieved session still exposes the email.
    let retrieved: Value = server
        .client
        .get(server.url(&format!(
            "/v1/checkout/sessions/{}",
            session["id"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retrieved["customer_email"], "guest@example.com");
    assert_eq!(retrieved["customer_details"]["email"], "guest@example.com");

    // An existing customer is kept and no additional customer is created.
    let existing: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("customer", customer_id.as_str()),
            ("customer_email", "override@example.com"),
            ("line_items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(existing["customer"], customer_id);
    assert_eq!(existing["customer_email"], "override@example.com");
    let customers_after_existing: Value = server
        .client
        .get(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        customers_after_existing["data"].as_array().unwrap().len(),
        before_count + 1
    );

    // Without customer_email the linked customer's email is used.
    let fallback: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("customer", customer_id.as_str()),
            ("line_items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(fallback["customer_email"], "guest@example.com");
    assert_eq!(fallback["customer_details"]["email"], "guest@example.com");
}

#[tokio::test]
async fn checkout_discounts_taxes_and_totals() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Priced Plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();
    let other_product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Other Plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let other_product_id = other_product["id"].as_str().unwrap().to_string();

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

    let post = |path: &'static str, form: Vec<(String, String)>| {
        let client = server.client.clone();
        let url = server.url(path);
        let key = key.clone();
        async move {
            client
                .post(url)
                .header("Authorization", format!("Bearer {key}"))
                .form(&form)
                .send()
                .await
                .unwrap()
        }
    };
    let create_session = |form: Vec<(String, String)>| post("/v1/checkout/sessions", form);

    let quarter: Value = post(
        "/v1/coupons",
        vec![
            ("percent_off".into(), "25".into()),
            ("duration".into(), "forever".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let quarter_id = quarter["id"].as_str().unwrap().to_string();

    let fixed: Value = post(
        "/v1/coupons",
        vec![
            ("amount_off".into(), "500".into()),
            ("currency".into(), "usd".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let fixed_id = fixed["id"].as_str().unwrap().to_string();

    let too_big: Value = post(
        "/v1/coupons",
        vec![
            ("id".into(), "too_big".into()),
            ("amount_off".into(), "5000".into()),
            ("currency".into(), "usd".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(too_big["amount_off"], 5000);

    let eur: Value = post(
        "/v1/coupons",
        vec![
            ("id".into(), "eur_coupon".into()),
            ("amount_off".into(), "500".into()),
            ("currency".into(), "eur".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(eur["currency"], "eur");

    let expired: Value = post(
        "/v1/coupons",
        vec![
            ("id".into(), "expired_coupon".into()),
            ("percent_off".into(), "10".into()),
            ("redeem_by".into(), "1".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(expired["valid"], false);

    let restricted: Value = post(
        "/v1/coupons",
        vec![
            ("id".into(), "other_product_only".into()),
            ("percent_off".into(), "10".into()),
            ("applies_to[products][0]".into(), other_product_id.clone()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(restricted["applies_to"]["products"][0], other_product_id);

    let exclusive: Value = post(
        "/v1/tax_rates",
        vec![
            ("display_name".into(), "Exclusive 10".into()),
            ("inclusive".into(), "false".into()),
            ("percentage".into(), "10".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let exclusive_id = exclusive["id"].as_str().unwrap().to_string();

    let exclusive_twenty: Value = post(
        "/v1/tax_rates",
        vec![
            ("display_name".into(), "Exclusive 20".into()),
            ("inclusive".into(), "false".into()),
            ("percentage".into(), "20".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let exclusive_twenty_id = exclusive_twenty["id"].as_str().unwrap().to_string();

    let inclusive: Value = post(
        "/v1/tax_rates",
        vec![
            ("display_name".into(), "Inclusive 20".into()),
            ("inclusive".into(), "true".into()),
            ("percentage".into(), "20".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let inclusive_id = inclusive["id"].as_str().unwrap().to_string();

    let inactive: Value = post(
        "/v1/tax_rates",
        vec![
            ("display_name".into(), "Inactive".into()),
            ("inclusive".into(), "false".into()),
            ("percentage".into(), "5".into()),
            ("active".into(), "false".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let inactive_id = inactive["id"].as_str().unwrap().to_string();

    // Discounted and taxed total.
    let session: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][quantity]".into(), "2".into()),
        ("discounts[0][coupon]".into(), quarter_id.clone()),
        ("tax_rates[]".into(), exclusive_id.clone()),
    ])
    .await
    .json()
    .await
    .unwrap();
    let _session_id = session["id"].as_str().unwrap().to_string();
    assert_eq!(session["amount_subtotal"], 2000);
    assert_eq!(session["amount_discount"], 500);
    assert_eq!(session["amount_tax"], 150);
    assert_eq!(session["amount_total"], 1650);
    assert_eq!(
        session["total_details"]["breakdown"]["discounts"][0]["coupon"],
        quarter_id
    );
    assert_eq!(
        session["total_details"]["breakdown"]["discounts"][0]["amount"],
        500
    );
    assert_eq!(
        session["total_details"]["breakdown"]["taxes"][0]["amount"],
        150
    );
    assert_eq!(
        session["total_details"]["breakdown"]["taxes"][0]["tax_rate"],
        exclusive_id
    );
    assert_eq!(
        session["total_details"]["breakdown"]["taxes"][0]["taxable_amount"],
        1500
    );
    assert_eq!(
        session["line_items"]["data"][0]["tax_amounts"][0]["amount"],
        150
    );

    // No discount or tax.
    let plain: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][quantity]".into(), "2".into()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(plain["amount_subtotal"], 2000);
    assert_eq!(plain["amount_total"], 2000);
    assert_eq!(plain["amount_discount"], 0);
    assert_eq!(plain["amount_tax"], 0);
    assert_eq!(plain["total_details"]["breakdown"]["discounts"], json!([]));

    // Fixed-amount discount.
    let fixed_session: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][quantity]".into(), "2".into()),
        ("discounts[0][coupon]".into(), fixed_id.clone()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(fixed_session["amount_discount"], 500);

    // A discount larger than the subtotal is capped.
    let capped: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][quantity]".into(), "2".into()),
        ("discounts[0][coupon]".into(), "too_big".into()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(capped["amount_discount"], 2000);
    assert_eq!(capped["amount_total"], 0);

    // Inclusive tax is reported but not added again.
    let inclusive_session: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][quantity]".into(), "2".into()),
        ("tax_rates[]".into(), inclusive_id.clone()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(inclusive_session["amount_tax"], 333);
    assert_eq!(inclusive_session["amount_total"], 2000);

    // A line-level tax rate overrides the session rate.
    let override_session: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][tax_rates][]".into(), exclusive_twenty_id.clone()),
        ("line_items[1][price]".into(), price_id.clone()),
        ("tax_rates[]".into(), exclusive_id.clone()),
    ])
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(override_session["amount_tax"], 300);
    assert_eq!(override_session["amount_total"], 2300);

    // Unknown or invalid coupons, currency mismatches, and product restrictions.
    for coupon in [
        "coupon_missing",
        "expired_coupon",
        "eur_coupon",
        "other_product_only",
    ] {
        let response = create_session(vec![
            ("mode".into(), "payment".into()),
            ("line_items[0][price]".into(), price_id.clone()),
            ("discounts[0][coupon]".into(), coupon.to_string()),
        ])
        .await;
        assert_eq!(response.status(), 400, "coupon {coupon} was accepted");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
    }

    // A fully redeemed coupon is rejected too.
    let spent: Value = post(
        "/v1/coupons",
        vec![
            ("id".into(), "spent_coupon".into()),
            ("percent_off".into(), "10".into()),
            ("max_redemptions".into(), "1".into()),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(spent["valid"], true);
    sqlx::query("UPDATE coupons SET times_redeemed = 1 WHERE id = 'spent_coupon'")
        .execute(&server.pool)
        .await
        .unwrap();
    let response = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("discounts[0][coupon]".into(), "spent_coupon".into()),
    ])
    .await;
    assert_eq!(response.status(), 400);

    // Unknown and inactive tax rates are rejected.
    for tax_rate in ["txr_missing", inactive_id.as_str()] {
        let response = create_session(vec![
            ("mode".into(), "payment".into()),
            ("line_items[0][price]".into(), price_id.clone()),
            ("tax_rates[]".into(), tax_rate.to_string()),
        ])
        .await;
        assert_eq!(response.status(), 400, "tax rate {tax_rate} was accepted");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
    }

    // A completed session redeems the coupon once and its payment intent uses
    // the discounted-taxed total.
    let completed: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("line_items[0][quantity]".into(), "2".into()),
        ("discounts[0][coupon]".into(), quarter_id.clone()),
        ("tax_rates[]".into(), exclusive_id.clone()),
    ])
    .await
    .json()
    .await
    .unwrap();
    let completed_id = completed["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/mock/checkout/{completed_id}")))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    let redeemed: Value = server
        .client
        .get(server.url(&format!("/v1/coupons/{quarter_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(redeemed["times_redeemed"], 1);

    // Completing an already complete session does not increment again.
    server
        .client
        .post(server.url(&format!("/mock/checkout/{completed_id}")))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    let redeemed_again: Value = server
        .client
        .get(server.url(&format!("/v1/coupons/{quarter_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(redeemed_again["times_redeemed"], 1);

    let completed_session: Value = server
        .client
        .get(server.url(&format!("/v1/checkout/sessions/{completed_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(completed_session["amount_subtotal"], 2000);
    assert_eq!(completed_session["amount_total"], 1650);
    let intent_id = completed_session["payment_intent"]
        .as_str()
        .unwrap()
        .to_string();
    let intent: Value = server
        .client
        .get(server.url(&format!("/v1/payment_intents/{intent_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(intent["amount"], 1650);

    // An expired session does not redeem the coupon.
    let expiring: Value = create_session(vec![
        ("mode".into(), "payment".into()),
        ("line_items[0][price]".into(), price_id.clone()),
        ("discounts[0][coupon]".into(), fixed_id.clone()),
    ])
    .await
    .json()
    .await
    .unwrap();
    let expiring_id = expiring["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/checkout/sessions/{expiring_id}/expire")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    let unchanged: Value = server
        .client
        .get(server.url(&format!("/v1/coupons/{fixed_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unchanged["times_redeemed"], 0);
}

#[tokio::test]
async fn billing_portal_sessions_and_page() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "portal@example.com")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let customer_id = customer["id"].as_str().unwrap().to_string();

    let empty_customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "empty@example.com")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let empty_customer_id = empty_customer["id"].as_str().unwrap().to_string();

    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Portal Plan")])
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
            ("items[0][quantity]", "3"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription_id = subscription["id"].as_str().unwrap().to_string();
    let item_period_end = subscription["items"]["data"][0]["current_period_end"]
        .as_i64()
        .unwrap();

    let events_before: Value = server
        .client
        .get(server.url("/v1/events"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let before_count = events_before["data"].as_array().unwrap().len();

    let portal: Value = server
        .client
        .post(server.url("/v1/billing_portal/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer_id.as_str()),
            ("return_url", "https://example.com/account"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let portal_id = portal["id"].as_str().unwrap().to_string();
    assert!(portal_id.starts_with("bps_"));
    assert_eq!(portal["object"], "billing_portal.session");
    assert_eq!(portal["customer"], customer_id);
    assert_eq!(portal["return_url"], "https://example.com/account");
    let page_url = portal["url"].as_str().unwrap().to_string();
    assert!(page_url.ends_with(&format!("/mock/billing-portal/{portal_id}")));

    let events_after: Value = server
        .client
        .get(server.url("/v1/events"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        events_after["data"].as_array().unwrap().len(),
        before_count,
        "a portal session emits no event"
    );

    // The page renders the customer, subscription, and return link.
    let page = server.client.get(&page_url).send().await.unwrap();
    assert_eq!(page.status(), 200);
    let html = page.text().await.unwrap();
    assert!(html.contains("Billing portal"));
    assert!(html.contains(&customer_id));
    assert!(html.contains("portal@example.com"));
    assert!(html.contains(&subscription_id));
    assert!(html.contains("3 ×"));
    assert!(html.contains(&item_period_end.to_string()));
    assert!(html.contains("https://example.com/account"));
    assert!(html.contains("Return to the application"));

    // A customer without subscriptions renders an empty list.
    let empty_portal: Value = server
        .client
        .post(server.url("/v1/billing_portal/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", empty_customer_id.as_str()),
            ("return_url", "https://example.com/empty"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let empty_html = server
        .client
        .get(empty_portal["url"].as_str().unwrap())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(empty_html.contains("no subscriptions"));

    // Errors: unknown customer and missing return_url create no session.
    let unknown = server
        .client
        .post(server.url("/v1/billing_portal/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", "cus_missing"),
            ("return_url", "https://example.com"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(unknown.status(), 404);

    let missing = server
        .client
        .post(server.url("/v1/billing_portal/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("customer", customer_id.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 400);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(body["error"]["param"], "return_url");

    // An unknown session id renders the not-found page.
    let not_found = server
        .client
        .get(server.url("/mock/billing-portal/bps_missing"))
        .send()
        .await
        .unwrap();
    assert_eq!(not_found.status(), 404);
    assert!(not_found.text().await.unwrap().contains("Not found"));
}
