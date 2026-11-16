mod common;

use common::*;
use serde_json::{Value, json};

async fn create_customer(server: &TestServer, key: &str, email: &str) -> String {
    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", email), ("name", "Buyer")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    customer["id"].as_str().unwrap().to_string()
}

async fn create_product(server: &TestServer, key: &str, name: &str) -> String {
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

async fn create_meter(server: &TestServer, key: &str) -> String {
    let meter: Value = server
        .client
        .post(server.url("/v1/billing/meters"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("display_name", "API calls"),
            ("event_name", "api_call"),
            ("default_aggregation[formula]", "sum"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meter["object"], "billing.meter");
    assert_eq!(meter["status"], "active");
    assert_eq!(
        meter["customer_mapping"]["event_payload_key"],
        "stripe_customer_id"
    );
    meter["id"].as_str().unwrap().to_string()
}

async fn report_usage(
    server: &TestServer,
    key: &str,
    customer: &str,
    identifier: &str,
    value: i64,
) -> reqwest::Response {
    server
        .client
        .post(server.url("/v1/billing/meter_events"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("event_name", "api_call"),
            ("identifier", identifier),
            ("payload[stripe_customer_id]", customer),
            ("payload[value]", &value.to_string()),
        ])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn meters_reject_duplicate_identifiers_and_aggregate_summaries() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let meter = create_meter(&server, &key).await;
    let customer = create_customer(&server, &key, "usage@example.com").await;

    let first = report_usage(&server, &key, &customer, "usage-1", 10).await;
    assert!(first.status().is_success());
    let event: Value = first.json().await.unwrap();
    assert_eq!(event["object"], "billing.meter_event");
    assert_eq!(event["identifier"], "usage-1");
    assert_eq!(event["payload"]["value"], "10");
    assert_eq!(event["payload"]["stripe_customer_id"], customer);

    let second = report_usage(&server, &key, &customer, "usage-2", 5).await;
    assert!(second.status().is_success());

    let duplicate = report_usage(&server, &key, &customer, "usage-1", 10).await;
    assert_eq!(duplicate.status(), 400);
    let body: Value = duplicate.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "duplicate_meter_event");

    let now = stripe_mock::time::now_epoch();
    let start = (now / 3600) * 3600 - 3600;
    let end = start + 7200;
    let summaries: Value = server
        .client
        .get(server.url(&format!(
            "/v1/billing/meters/{meter}/event_summaries?customer={customer}&start_time={start}&end_time={end}&value_grouping_window=hour"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summaries["object"], "list");
    let data = summaries["data"].as_array().unwrap();
    let total: i64 = data
        .iter()
        .map(|summary| summary["aggregated_value"].as_i64().unwrap())
        .sum();
    assert_eq!(total, 15);
    assert_eq!(data[0]["object"], "billing.meter_event_summary");
    assert_eq!(data[0]["meter"], meter);
    assert!(data[0]["id"].as_str().unwrap().starts_with("mtrusg_"));
}

async fn create_metered_price(server: &TestServer, key: &str, meter: &str) -> String {
    let product = create_product(server, key, "API Plan").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("recurring[usage_type]", "metered"),
            ("recurring[meter]", meter),
            ("unit_amount", "2"),
            ("nickname", "API calls"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(price["billing_scheme"], "per_unit");
    assert_eq!(price["recurring"]["usage_type"], "metered");
    assert_eq!(price["recurring"]["meter"], meter);
    price["id"].as_str().unwrap().to_string()
}

async fn create_tiered_price(server: &TestServer, key: &str) -> String {
    let product = create_product(server, key, "Seat Plan").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("billing_scheme", "tiered"),
            ("tiers_mode", "graduated"),
            ("tiers[0][up_to]", "10"),
            ("tiers[0][unit_amount]", "100"),
            ("tiers[1][up_to]", "inf"),
            ("tiers[1][unit_amount]", "50"),
            ("nickname", "Seats"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(price["billing_scheme"], "tiered");
    assert_eq!(price["tiers_mode"], "graduated");
    assert_eq!(price["tiers"].as_array().unwrap().len(), 2);
    assert_eq!(price["tiers"][1]["up_to"], Value::Null);
    price["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn metered_and_tiered_prices_reject_quantities_and_price_invoices() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let meter = create_meter(&server, &key).await;
    let customer = create_customer(&server, &key, "plans@example.com").await;
    let metered = create_metered_price(&server, &key, &meter).await;
    let tiered = create_tiered_price(&server, &key).await;

    // A quantity on a metered line is rejected everywhere a line can name one.
    let response = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", metered.as_str()),
            ("items[0][quantity]", "2"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "items[0][quantity]");

    let checkout = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "subscription"),
            ("line_items[0][price]", metered.as_str()),
            ("line_items[0][quantity]", "2"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(checkout.status(), 400);

    // A metered subscription starts with a paid first invoice and no quantity.
    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", metered.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(subscription["items"]["data"][0]["quantity"], Value::Null);
    let subscription_id = subscription["id"].as_str().unwrap().to_string();
    let invoices: Value = server
        .client
        .get(server.url(&format!("/v1/invoices?subscription={subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoices["data"][0]["status"], "paid");
    assert_eq!(invoices["data"][0]["billing_reason"], "subscription_create");

    // Usage reported during the period is billed on the renewal invoice.
    let usage = report_usage(&server, &key, &customer, "cycle-1", 7).await;
    assert!(usage.status().is_success());
    let renewal: Value = server
        .client
        .post(server.url(&format!(
            "/internal/subscriptions/{subscription_id}/advance"
        )))
        .json(&json!({ "outcome": "paid" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renewal["subscription"]["status"], "active");
    assert_eq!(renewal["invoice"]["status"], "paid");
    assert_eq!(renewal["invoice"]["billing_reason"], "subscription_cycle");
    assert_eq!(renewal["invoice"]["total"], 14);
    assert_eq!(renewal["invoice"]["lines"]["data"][0]["quantity"], 7);
    let renewal_line = &renewal["invoice"]["lines"]["data"][0];
    assert_eq!(renewal_line["parent"]["type"], "subscription_item_details");
    assert_eq!(
        renewal_line["parent"]["subscription_item_details"]["subscription"],
        subscription_id
    );
    assert_eq!(
        renewal_line["parent"]["subscription_item_details"]["subscription_item"],
        subscription["items"]["data"][0]["id"]
    );
    assert_eq!(renewal_line["pricing"]["type"], "price_details");
    assert_eq!(renewal_line["pricing"]["price_details"]["price"], metered);
    assert_eq!(renewal_line["pricing"]["unit_amount_decimal"], "2");

    // Graduated tiers: 10 units at 100 plus 15 at 50.
    let tiered_subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", tiered.as_str()),
            ("items[0][quantity]", "25"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tiered_id = tiered_subscription["id"].as_str().unwrap();
    let tiered_invoices: Value = server
        .client
        .get(server.url(&format!("/v1/invoices?subscription={tiered_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tiered_invoices["data"][0]["total"], 1750);
    // A tiered price has no single unit amount, so pricing reports null rather
    // than the line total.
    let tiered_line = &tiered_invoices["data"][0]["lines"]["data"][0];
    assert_eq!(tiered_line["pricing"]["price_details"]["price"], tiered);
    assert_eq!(tiered_line["pricing"]["unit_amount_decimal"], Value::Null);
}

#[tokio::test]
async fn embedded_checkout_returns_a_client_secret_and_saves_a_card() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "embedded@example.com").await;
    let tiered = create_tiered_price(&server, &key).await;

    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "subscription"),
            ("ui_mode", "embedded_page"),
            (
                "return_url",
                "https://example.com/return?session={CHECKOUT_SESSION_ID}",
            ),
            ("customer", customer.as_str()),
            ("line_items[0][price]", tiered.as_str()),
            ("line_items[0][quantity]", "3"),
            ("subscription_data[metadata][plan]", "team"),
            ("payment_method_collection", "if_required"),
            ("automatic_tax[enabled]", "true"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["ui_mode"], "embedded_page");
    assert_eq!(session["url"], Value::Null);
    assert_eq!(session["payment_method_collection"], "if_required");
    assert_eq!(session["automatic_tax"]["enabled"], true);
    assert_eq!(session["automatic_tax"]["status"], Value::Null);
    let client_secret = session["client_secret"].as_str().unwrap();
    assert!(client_secret.starts_with(session["id"].as_str().unwrap()));
    let session_id = session["id"].as_str().unwrap().to_string();

    let completion = server
        .client
        .post(server.url(&format!("/mock/checkout/{session_id}")))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    assert_eq!(completion.status(), 303);
    let location = completion
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        location,
        format!("https://example.com/return?session={session_id}")
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
    assert_eq!(completed["payment_method_collection"], "if_required");
    assert_eq!(completed["automatic_tax"]["enabled"], true);
    assert_eq!(completed["automatic_tax"]["status"], Value::Null);
    let subscription_id = completed["subscription"].as_str().unwrap();
    let subscription: Value = server
        .client
        .get(server.url(&format!(
            "/v1/subscriptions/{subscription_id}?expand[]=latest_invoice&expand[]=default_payment_method"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(subscription["metadata"]["plan"], "team");
    let card_id = subscription["default_payment_method"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(card_id.starts_with("pm_"));
    assert_eq!(
        subscription["default_payment_method"]["card"]["last4"],
        "4242"
    );
    let latest_invoice = subscription["latest_invoice"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(subscription["latest_invoice"]["object"], "invoice");

    let created_events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.subscription.created"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let created = &created_events["data"][0]["data"]["object"];
    assert_eq!(created["id"], subscription_id);
    assert_eq!(created["default_payment_method"], card_id);
    assert_eq!(created["latest_invoice"], latest_invoice);

    let stored_customer: Value = server
        .client
        .get(server.url(&format!(
            "/v1/customers/{customer}?expand[]=invoice_settings.default_payment_method"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let card = &stored_customer["invoice_settings"]["default_payment_method"]["card"];
    assert_eq!(card["brand"], "visa");
    assert_eq!(card["last4"], "4242");

    let preview: Value = server
        .client
        .post(server.url("/v1/invoices/create_preview"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("subscription", subscription_id),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["object"], "invoice");
    assert_eq!(preview["billing_reason"], "upcoming");
    assert_eq!(preview["status"], "draft");
    assert!(preview["id"].as_str().unwrap().starts_with("upcoming_in_"));
    assert!(preview["total"].as_i64().unwrap() > 0);

    // The older GET form accepts the same fields as query parameters.
    let upcoming: Value = server
        .client
        .get(server.url(&format!(
            "/v1/invoices/upcoming?customer={customer}&subscription={subscription_id}"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(upcoming["billing_reason"], "upcoming");
}

#[tokio::test]
async fn checkout_echoes_defaults_and_rejects_an_unsupported_collection() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "echo@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
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

    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("customer", customer.as_str()),
            ("line_items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["payment_method_collection"], "always");
    assert_eq!(session["automatic_tax"]["enabled"], false);
    assert_eq!(session["automatic_tax"]["status"], Value::Null);

    let rejected: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "payment"),
            ("customer", customer.as_str()),
            ("line_items[0][price]", price_id.as_str()),
            ("payment_method_collection", "never"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rejected["error"]["type"], "invalid_request_error");
    assert_eq!(rejected["error"]["param"], "payment_method_collection");
}

#[tokio::test]
async fn failed_renewal_can_be_recovered_without_raising_another_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "renew@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("unit_amount", "1000"),
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
            ("customer", customer.as_str()),
            ("items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = subscription["id"].as_str().unwrap().to_string();

    let failed: Value = server
        .client
        .post(server.url(&format!("/internal/subscriptions/{id}/advance")))
        .json(&json!({ "outcome": "failed" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(failed["subscription"]["status"], "past_due");
    assert_eq!(failed["invoice"]["status"], "open");
    assert_eq!(failed["invoice"]["amount_remaining"], 1000);
    assert_eq!(failed["invoice"]["attempt_count"], 1);
    let invoice_id = failed["invoice"]["id"].as_str().unwrap().to_string();

    // The updated event names the invoice the advance just raised.
    let updated_events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.subscription.updated"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let updated = &updated_events["data"][0]["data"]["object"];
    assert_eq!(updated["latest_invoice"], invoice_id);

    let recovered: Value = server
        .client
        .post(server.url(&format!("/internal/subscriptions/{id}/advance")))
        .json(&json!({ "outcome": "paid" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recovered["subscription"]["status"], "active");
    assert_eq!(recovered["invoice"]["id"], invoice_id);
    assert_eq!(recovered["invoice"]["status"], "paid");
    assert_eq!(recovered["invoice"]["attempt_count"], 2);

    let invoices: Value = server
        .client
        .get(server.url(&format!("/v1/invoices?subscription={id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoices["data"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn subscription_creation_reports_its_first_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "latest@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("unit_amount", "1000"),
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
            ("customer", customer.as_str()),
            ("items[0][price]", price_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription_id = subscription["id"].as_str().unwrap().to_string();
    let latest_invoice = subscription["latest_invoice"].as_str().unwrap().to_string();

    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{latest_invoice}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["billing_reason"], "subscription_create");
    assert_eq!(invoice["subscription"], subscription_id);

    let events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.subscription.created"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let created = &events["data"][0]["data"]["object"];
    assert_eq!(created["id"], subscription_id);
    assert_eq!(created["latest_invoice"], latest_invoice);

    let expanded: Value = server
        .client
        .get(server.url(&format!(
            "/v1/subscriptions/{subscription_id}?expand[]=latest_invoice"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(expanded["latest_invoice"]["object"], "invoice");
    assert_eq!(expanded["latest_invoice"]["id"], latest_invoice);
    assert!(
        expanded["latest_invoice"]["hosted_invoice_url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/mock/invoices/{latest_invoice}"))
    );
}

#[tokio::test]
async fn cancel_at_period_end_tracks_cancel_at_and_cancellation_stamps() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "cancel@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("unit_amount", "1000"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = subscription["id"].as_str().unwrap().to_string();
    let period_end = subscription["current_period_end"].as_i64().unwrap();

    let scheduled: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("cancel_at_period_end", "true")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(scheduled["cancel_at_period_end"], true);
    assert_eq!(scheduled["cancel_at"].as_i64(), Some(period_end));

    let cleared: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("cancel_at_period_end", "false")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cleared["cancel_at_period_end"], false);
    assert_eq!(cleared["cancel_at"], Value::Null);

    let deleted: Value = server
        .client
        .delete(server.url(&format!("/v1/subscriptions/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["status"], "canceled");
    assert!(deleted["canceled_at"].as_i64().is_some());
    assert!(deleted["ended_at"].as_i64().is_some());
}

#[tokio::test]
async fn advance_cancels_a_subscription_at_the_period_end() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "advance-cancel@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("unit_amount", "1000"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = subscription["id"].as_str().unwrap().to_string();
    let period_end = subscription["current_period_end"].as_i64().unwrap();

    let scheduled: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("cancel_at_period_end", "true")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(scheduled["cancel_at"], period_end);

    let advanced: Value = server
        .client
        .post(server.url(&format!("/internal/subscriptions/{id}/advance")))
        .json(&json!({ "outcome": "paid" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(advanced["subscription"]["status"], "canceled");
    assert_eq!(advanced["subscription"]["canceled_at"], period_end);
    assert_eq!(advanced["subscription"]["ended_at"], period_end);
    assert_eq!(advanced["invoice"], Value::Null);

    // No renewal invoice exists for the canceled period.
    let invoices: Value = server
        .client
        .get(server.url(&format!("/v1/invoices?subscription={id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let invoices_data = invoices["data"].as_array().unwrap();
    assert_eq!(invoices_data.len(), 1);
    assert_eq!(invoices_data[0]["billing_reason"], "subscription_create");

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
    let deleted = &events["data"][0]["data"]["object"];
    assert_eq!(deleted["id"], id);
    assert_eq!(deleted["status"], "canceled");
}

#[tokio::test]
async fn billing_cycle_anchor_day_is_echoed_and_validated() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "anchor@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("unit_amount", "1000"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", price["id"].as_str().unwrap()),
            ("billing_cycle_anchor_config[day_of_month]", "15"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        subscription["billing_cycle_anchor_config"]["day_of_month"],
        15
    );
    let id = subscription["id"].as_str().unwrap().to_string();
    let retrieved: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(retrieved["billing_cycle_anchor_config"]["day_of_month"], 15);

    let rejected: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("items[0][price]", price["id"].as_str().unwrap()),
            ("billing_cycle_anchor_config[day_of_month]", "32"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rejected["error"]["type"], "invalid_request_error");
    assert_eq!(
        rejected["error"]["param"],
        "billing_cycle_anchor_config[day_of_month]"
    );

    let list: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions?customer={customer}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        list["data"].as_array().unwrap().len(),
        1,
        "the invalid anchor created no subscription"
    );
}

#[tokio::test]
async fn subscription_invoices_and_previews_report_parent_and_taxes() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "parent@example.com").await;
    let product = create_product(&server, &key, "Monthly").await;
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product.as_str()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("unit_amount", "4900"),
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
            ("customer", customer.as_str()),
            ("items[0][price]", price_id.as_str()),
            ("metadata[plan]", "team"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let subscription_id = subscription["id"].as_str().unwrap().to_string();

    let invoice: Value = server
        .client
        .get(server.url(&format!(
            "/v1/invoices/{}",
            subscription["latest_invoice"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["parent"]["type"], "subscription_details");
    assert_eq!(
        invoice["parent"]["subscription_details"]["subscription"],
        subscription_id
    );
    assert_eq!(
        invoice["parent"]["subscription_details"]["metadata"]["plan"],
        "team"
    );
    assert_eq!(invoice["total_taxes"], json!([]));
    // The legacy fields remain alongside the newer shape.
    assert_eq!(invoice["subscription"], subscription_id);
    let line = &invoice["lines"]["data"][0];
    assert_eq!(
        line["subscription_item"],
        subscription["items"]["data"][0]["id"]
    );
    assert_eq!(line["pricing"]["unit_amount_decimal"], "4900");

    let preview: Value = server
        .client
        .post(server.url("/v1/invoices/create_preview"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("subscription", subscription_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["parent"]["type"], "subscription_details");
    assert_eq!(
        preview["parent"]["subscription_details"]["subscription"],
        subscription_id
    );
    assert_eq!(
        preview["parent"]["subscription_details"]["metadata"]["plan"],
        "team"
    );
    assert_eq!(preview["total_taxes"], json!([]));
    let preview_line = &preview["lines"]["data"][0];
    assert_eq!(
        preview_line["parent"]["subscription_item_details"]["subscription"],
        subscription_id
    );
    assert_eq!(preview_line["pricing"]["price_details"]["price"], price_id);
}

#[tokio::test]
async fn internal_meter_events_list_and_filter_accepted_events() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let meter = create_meter(&server, &key).await;
    let customer = create_customer(&server, &key, "inspect@example.com").await;
    assert!(!meter.is_empty());

    let documents: Value = server
        .client
        .post(server.url("/v1/billing/meters"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("display_name", "Documents"),
            ("event_name", "documents.used"),
            ("default_aggregation[formula]", "sum"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(documents["object"], "billing.meter");

    assert!(
        report_usage(&server, &key, &customer, "inspect-1", 10)
            .await
            .status()
            .is_success()
    );
    assert!(
        report_usage(&server, &key, &customer, "inspect-2", 5)
            .await
            .status()
            .is_success()
    );
    let document_event = server
        .client
        .post(server.url("/v1/billing/meter_events"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("event_name", "documents.used"),
            ("identifier", "documents-1"),
            ("payload[stripe_customer_id]", customer.as_str()),
            ("payload[value]", "3"),
        ])
        .send()
        .await
        .unwrap();
    assert!(document_event.status().is_success());

    // The unfiltered list carries every accepted event; no Authorization header
    // is needed on the control plane.
    let all: Value = server
        .client
        .get(server.url("/internal/meter_events"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all["object"], "list");
    assert_eq!(all["data"].as_array().unwrap().len(), 3);
    let api_call = all["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["identifier"] == "inspect-1")
        .unwrap();
    assert_eq!(api_call["event_name"], "api_call");
    assert_eq!(api_call["customer"], customer);
    assert_eq!(api_call["value"], "10");
    assert!(api_call["timestamp"].as_i64().is_some());

    let filtered: Value = server
        .client
        .get(server.url(&format!(
            "/internal/meter_events?event_name=api_call&customer={customer}"
        )))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let filtered_data = filtered["data"].as_array().unwrap();
    assert_eq!(filtered_data.len(), 2);
    assert!(
        filtered_data
            .iter()
            .all(|event| event["event_name"] == "api_call")
    );

    let documents_only: Value = server
        .client
        .get(server.url("/internal/meter_events?event_name=documents.used"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let documents_data = documents_only["data"].as_array().unwrap();
    assert_eq!(documents_data.len(), 1);
    assert_eq!(documents_data[0]["identifier"], "documents-1");
    assert_eq!(documents_data[0]["value"], "3");
}

#[tokio::test]
async fn configured_keys_and_signing_secrets_survive_reset() {
    let server =
        spawn_with_main_keys("sk_test_stateleo", "pk_test_stateleo", "whsec_stateleo").await;
    let before: Value = server
        .client
        .get(server.url("/internal/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["secret_key"], "sk_test_stateleo");
    assert_eq!(before["publishable_key"], "pk_test_stateleo");
    let destinations: Value = server
        .client
        .get(server.url("/internal/destinations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        destinations["data"][0]["webhook_endpoint"]["signing_secret"],
        "whsec_stateleo"
    );

    let reset = server
        .client
        .post(server.url("/internal/reset"))
        .send()
        .await
        .unwrap();
    assert!(reset.status().is_success());

    let after: Value = server
        .client
        .get(server.url("/internal/main"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["secret_key"], "sk_test_stateleo");
    assert_eq!(after["publishable_key"], "pk_test_stateleo");
    let destinations: Value = server
        .client
        .get(server.url("/internal/destinations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        destinations["data"][0]["webhook_endpoint"]["signing_secret"],
        "whsec_stateleo"
    );

    // The configured key authenticates after the reset.
    let customer = create_customer(&server, "sk_test_stateleo", "reset@example.com").await;
    assert!(customer.starts_with("cus_"));
}

#[tokio::test]
async fn idempotency_keys_replay_and_reject_different_parameters() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let first = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Idempotency-Key", "customer-1")
        .form(&[("email", "idem@example.com")])
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());
    let body: Value = first.json().await.unwrap();
    let customer_id = body["id"].as_str().unwrap().to_string();

    let replay = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Idempotency-Key", "customer-1")
        .form(&[("email", "idem@example.com")])
        .send()
        .await
        .unwrap();
    assert!(replay.status().is_success());
    assert_eq!(
        replay
            .headers()
            .get("idempotent-replay")
            .and_then(|value| value.to_str().ok()),
        Some("true")
    );
    let replayed: Value = replay.json().await.unwrap();
    assert_eq!(replayed["id"], customer_id);

    let conflict = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .header("Idempotency-Key", "customer-1")
        .form(&[("email", "other@example.com")])
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), 400);
    let error: Value = conflict.json().await.unwrap();
    assert_eq!(error["error"]["type"], "idempotency_error");

    let customers: Value = server
        .client
        .get(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customers["data"].as_array().unwrap().len(), 1);
}
