mod common;

use common::*;
use serde_json::{Value, json};

/// Create a customer and return its id.
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

/// Create a one-time price and return its id.
async fn create_price(server: &TestServer, key: &str, amount: i64) -> String {
    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Widget")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product["id"].as_str().unwrap()),
            ("unit_amount", amount.to_string().as_str()),
            ("currency", "usd"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    price["id"].as_str().unwrap().to_string()
}

/// Create a draft invoice for `customer` with one line for `price`.
async fn create_invoice(server: &TestServer, key: &str, customer: &str, price: &str) -> Value {
    server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer),
            ("lines[0][price]", price),
            ("lines[0][quantity]", "1"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The event types the receiver has seen so far.
async fn received_types(receiver: &WebhookReceiver) -> Vec<String> {
    let state = receiver.state.lock().await;
    state
        .received
        .iter()
        .map(|d| d.body["type"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn flush(server: &TestServer) {
    server
        .client
        .post(server.url("/internal/deliveries/flush"))
        .json(&json!({ "timeout_ms": 8000 }))
        .send()
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Customers
// ---------------------------------------------------------------------------

#[tokio::test]
async fn customer_can_be_created_with_tax_ids() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("email", "buyer@example.com"),
            ("tax_id_data[0][type]", "eu_vat"),
            ("tax_id_data[0][value]", "DE123456789"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(customer["id"].as_str().unwrap().starts_with("cus_"));
    let tax_ids = customer["tax_ids"].as_array().unwrap();
    assert_eq!(tax_ids.len(), 1);
    assert!(tax_ids[0]["id"].as_str().unwrap().starts_with("txi_"));
    assert_eq!(tax_ids[0]["type"], "eu_vat");
    assert_eq!(tax_ids[0]["value"], "DE123456789");
}

#[tokio::test]
async fn customer_tax_id_without_a_value_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let response = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("email", "buyer@example.com"),
            ("tax_id_data[0][type]", "eu_vat"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(body["error"]["param"].as_str().unwrap().contains("value"));

    // No customer was created.
    let list: Value = server
        .client
        .get(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn customers_can_be_listed_updated_and_deleted() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let id = create_customer(&server, &key, "buyer@example.com").await;
    create_customer(&server, &key, "other@example.com").await;

    let list: Value = server
        .client
        .get(server.url("/v1/customers?email=buyer@example.com"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["object"], "list");
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["id"], id);

    // An update appends a tax id and keeps the existing one.
    let customer: Value = server
        .client
        .post(server.url(&format!("/v1/customers/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("name", "New Name"),
            ("tax_id_data[0][type]", "de_vat"),
            ("tax_id_data[0][value]", "DE987654321"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customer["name"], "New Name");
    assert_eq!(customer["tax_ids"].as_array().unwrap().len(), 1);

    let response = server
        .client
        .delete(server.url(&format!("/v1/customers/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["id"], id);
    assert_eq!(body["deleted"], true);

    let response = server
        .client
        .get(server.url(&format!("/v1/customers/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    let list: Value = server
        .client
        .get(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn customer_events_are_delivered() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "buyer@example.com")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = customer["id"].as_str().unwrap().to_string();

    server
        .client
        .post(server.url(&format!("/v1/customers/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Updated")])
        .send()
        .await
        .unwrap();
    server
        .client
        .delete(server.url(&format!("/v1/customers/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    flush(&server).await;
    let types = received_types(&receiver).await;
    assert!(types.contains(&"customer.created".to_string()));
    assert!(types.contains(&"customer.updated".to_string()));
    assert!(types.contains(&"customer.deleted".to_string()));

    let state = receiver.state.lock().await;
    let created = state
        .received
        .iter()
        .find(|d| d.body["type"] == "customer.created")
        .unwrap();
    assert_eq!(created.body["data"]["object"]["id"], id);
}

// ---------------------------------------------------------------------------
// Invoices
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invoice_is_created_with_lines_and_totals() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    let invoice: Value = server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("lines[0][price]", price.as_str()),
            ("lines[0][quantity]", "2"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(invoice["id"].as_str().unwrap().starts_with("in_"));
    assert_eq!(invoice["object"], "invoice");
    assert_eq!(invoice["status"], "draft");
    assert_eq!(invoice["currency"], "usd");
    assert_eq!(invoice["subtotal"], 4000);
    assert_eq!(invoice["total"], 4000);
    assert_eq!(invoice["tax_total"], 0);
    let lines = invoice["lines"]["data"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["amount"], 4000);
    assert_eq!(lines[0]["quantity"], 2);
}

#[tokio::test]
async fn invoice_requires_a_customer_and_known_lines() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    // No customer.
    let response = server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("lines[0][price]", price.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "customer");

    // Unknown customer.
    let response = server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", "cus_missing"),
            ("lines[0][price]", price.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    // Unknown price.
    let response = server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("lines[0][price]", "price_missing"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
}

#[tokio::test]
async fn invoice_line_carries_an_inline_tax_rate() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice: Value = server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer.as_str()),
            ("lines[0][price]", price.as_str()),
            ("lines[0][quantity]", "1"),
            ("lines[0][tax_rate][percentage]", "10"),
            ("lines[0][tax_rate][name]", "VAT"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(invoice["tax_total"], 100);
    assert_eq!(invoice["total"], 1100);
    let line = &invoice["lines"]["data"][0];
    assert_eq!(line["amount"], 1000);
    let tax_amounts = line["tax_amounts"].as_array().unwrap();
    assert_eq!(tax_amounts.len(), 1);
    assert_eq!(tax_amounts[0]["amount"], 100);
    assert_eq!(tax_amounts[0]["taxable_amount"], 1000);
    assert_eq!(tax_amounts[0]["tax_rate"]["percentage"], 10);
    assert_eq!(tax_amounts[0]["tax_rate"]["name"], "VAT");
}

#[tokio::test]
async fn invoice_copies_the_customer_tax_ids() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("email", "buyer@example.com"),
            ("tax_id_data[0][type]", "eu_vat"),
            ("tax_id_data[0][value]", "DE123456789"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let customer_id = customer["id"].as_str().unwrap().to_string();
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer_id, &price).await;
    let tax_ids = invoice["customer_tax_ids"].as_array().unwrap();
    assert_eq!(tax_ids.len(), 1);
    assert_eq!(tax_ids[0]["type"], "eu_vat");
    assert_eq!(tax_ids[0]["value"], "DE123456789");
}

#[tokio::test]
async fn invoices_can_be_listed_and_read_with_lines() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let other = create_customer(&server, &key, "other@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    create_invoice(&server, &key, &other, &price).await;

    let list: Value = server
        .client
        .get(server.url(&format!("/v1/invoices?customer={customer}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["object"], "list");
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["id"], invoice_id);

    let lines: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}/lines")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(lines["object"], "list");
    assert_eq!(lines["data"].as_array().unwrap().len(), 1);
    assert_eq!(lines["data"][0]["amount"], 2000);
}

#[tokio::test]
async fn invoice_lifecycle_moves_through_every_stage() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    // Draft -> open.
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let finalized: Value = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(finalized["status"], "open");
    assert_eq!(finalized["amount_due"], 2000);
    assert!(finalized["finalized_at"].is_i64());

    // Finalizing twice is rejected and leaves the status unchanged.
    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["status"], "open");

    // Open -> void.
    let voided: Value = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/void")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(voided["status"], "void");
    assert!(voided["voided_at"].is_i64());
}

#[tokio::test]
async fn invoice_can_be_marked_uncollectible() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let marked: Value = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/mark_uncollectible")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(marked["status"], "uncollectible");

    // A paid invoice cannot be marked uncollectible.
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/pay")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/mark_uncollectible")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
}

#[tokio::test]
async fn unknown_invoice_returns_not_found() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    for path in ["/v1/invoices/in_missing", "/v1/invoices/in_missing/lines"] {
        let response = server
            .client
            .get(server.url(path))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }
    for path in [
        "/v1/invoices/in_missing/finalize",
        "/v1/invoices/in_missing/void",
        "/v1/invoices/in_missing/mark_uncollectible",
        "/v1/invoices/in_missing/pay",
    ] {
        let response = server
            .client
            .post(server.url(path))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 404, "{path}");
    }
}

// ---------------------------------------------------------------------------
// Payment intents
// ---------------------------------------------------------------------------

#[tokio::test]
async fn payment_intent_is_created_and_linked_to_an_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();

    let intent: Value = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("amount", "2000"),
            ("currency", "usd"),
            ("customer", customer.as_str()),
            ("invoice", invoice_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(intent["id"].as_str().unwrap().starts_with("pi_"));
    assert_eq!(intent["amount"], 2000);
    assert_eq!(intent["currency"], "usd");
    assert_eq!(intent["status"], "requires_payment_method");
    assert_eq!(intent["invoice"], invoice_id);

    // Amount is required.
    let response = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("currency", "usd")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "amount");

    // An unknown invoice is rejected.
    let response = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("amount", "100"), ("invoice", "in_missing")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    let list: Value = server
        .client
        .get(server.url(&format!("/v1/payment_intents?invoice={invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["object"], "list");
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn resolving_a_payment_intent_updates_its_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let intent: Value = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("amount", "2000"),
            ("currency", "usd"),
            ("invoice", invoice_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let intent_id = intent["id"].as_str().unwrap().to_string();

    let resolved: Value = server
        .client
        .post(server.url(&format!("/internal/payment_intents/{intent_id}/resolve")))
        .json(&json!({ "status": "succeeded" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["status"], "succeeded");

    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["status"], "paid");
    assert_eq!(invoice["amount_paid"], 2000);
    assert_eq!(invoice["amount_remaining"], 0);
    assert!(invoice["paid_at"].is_i64());
}

#[tokio::test]
async fn a_partial_payment_leaves_the_invoice_open() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let intent: Value = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("amount", "500"),
            ("currency", "usd"),
            ("invoice", invoice_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let intent_id = intent["id"].as_str().unwrap().to_string();

    server
        .client
        .post(server.url(&format!("/internal/payment_intents/{intent_id}/resolve")))
        .json(&json!({ "status": "succeeded" }))
        .send()
        .await
        .unwrap();

    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["status"], "open");
    assert_eq!(invoice["amount_paid"], 500);
    assert_eq!(invoice["amount_remaining"], 1500);
}

#[tokio::test]
async fn a_failed_payment_leaves_the_invoice_unchanged() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let intent: Value = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("amount", "2000"),
            ("currency", "usd"),
            ("invoice", invoice_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let intent_id = intent["id"].as_str().unwrap().to_string();

    let resolved: Value = server
        .client
        .post(server.url(&format!("/internal/payment_intents/{intent_id}/resolve")))
        .json(&json!({ "status": "failed" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resolved["status"], "failed");

    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["status"], "open");
    assert_eq!(invoice["amount_paid"], 0);
    assert_eq!(invoice["amount_remaining"], 2000);
}

#[tokio::test]
async fn resolving_rejects_an_unsupported_status_and_an_unknown_intent() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let intent: Value = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("amount", "100")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let intent_id = intent["id"].as_str().unwrap().to_string();

    let response = server
        .client
        .post(server.url(&format!("/internal/payment_intents/{intent_id}/resolve")))
        .json(&json!({ "status": "refunded" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // The intent is unchanged.
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
    assert_eq!(intent["status"], "requires_payment_method");

    // An unknown intent.
    let response = server
        .client
        .post(server.url("/internal/payment_intents/pi_missing/resolve"))
        .json(&json!({ "status": "succeeded" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn a_closed_invoice_is_not_credited_by_a_later_payment() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/void")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let intent: Value = server
        .client
        .post(server.url("/v1/payment_intents"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("amount", "2000"),
            ("currency", "usd"),
            ("invoice", invoice_id.as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let intent_id = intent["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/internal/payment_intents/{intent_id}/resolve")))
        .json(&json!({ "status": "succeeded" }))
        .send()
        .await
        .unwrap();

    // A voided invoice is terminal and is not reopened by a payment.
    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["status"], "void");
    assert_eq!(invoice["amount_paid"], 0);
    assert_eq!(invoice["amount_remaining"], 2000);
}

// ---------------------------------------------------------------------------
// Paying an invoice
// ---------------------------------------------------------------------------

#[tokio::test]
async fn paying_an_invoice_settles_it_and_emits_the_events() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let paid: Value = server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/pay")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(paid["status"], "paid");
    assert_eq!(paid["amount_paid"], 2000);
    assert_eq!(paid["amount_remaining"], 0);
    assert!(paid["payment_intent"].as_str().unwrap().starts_with("pi_"));

    // The payment intent that settled the invoice references it.
    let intent: Value = server
        .client
        .get(server.url(&format!(
            "/v1/payment_intents/{}",
            paid["payment_intent"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(intent["status"], "succeeded");
    assert_eq!(intent["invoice"], invoice_id);

    flush(&server).await;
    let types = received_types(&receiver).await;
    for expected in [
        "payment_intent.created",
        "payment_intent.succeeded",
        "invoice.payment_succeeded",
        "invoice.paid",
    ] {
        assert!(
            types.contains(&expected.to_string()),
            "missing {expected} in {types:?}"
        );
    }
}

#[tokio::test]
async fn paying_an_invoice_that_is_not_open_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();

    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/pay")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    let invoice: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoice["status"], "draft");
}

#[tokio::test]
async fn invoice_events_are_delivered() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let invoice_id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/pay")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    flush(&server).await;
    let types = received_types(&receiver).await;
    assert!(types.contains(&"invoice.created".to_string()));
    assert!(types.contains(&"invoice.finalized".to_string()));
    assert!(types.contains(&"invoice.paid".to_string()));
    assert!(types.contains(&"invoice.payment_succeeded".to_string()));

    let state = receiver.state.lock().await;
    let created = state
        .received
        .iter()
        .find(|d| d.body["type"] == "invoice.created")
        .unwrap();
    assert_eq!(created.body["data"]["object"]["id"], invoice_id);
}

// ---------------------------------------------------------------------------
// Invoice line editing
// ---------------------------------------------------------------------------

/// Create a one-time price in `currency` and return its id.
async fn create_price_in_currency(
    server: &TestServer,
    key: &str,
    amount: i64,
    currency: &str,
) -> String {
    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Widget")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let price: Value = server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product["id"].as_str().unwrap()),
            ("unit_amount", amount.to_string().as_str()),
            ("currency", currency),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    price["id"].as_str().unwrap().to_string()
}

/// Post to one of the invoice line endpoints and return the raw response.
async fn post_lines(
    server: &TestServer,
    key: &str,
    invoice_id: &str,
    action: &str,
    form: &[(&str, &str)],
) -> reqwest::Response {
    server
        .client
        .post(server.url(&format!("/v1/invoices/{invoice_id}/{action}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(form)
        .send()
        .await
        .unwrap()
}

/// Read an invoice back.
async fn get_invoice(server: &TestServer, key: &str, invoice_id: &str) -> Value {
    server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_catalog_line_and_an_ad_hoc_line_can_be_added() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    assert_eq!(invoice["subtotal"], 1000);

    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price]", price.as_str()),
            ("lines[0][quantity]", "2"),
            ("lines[1][amount]", "500"),
            ("lines[1][description]", "Setup fee"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();

    let lines = updated["lines"]["data"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["amount"], 2000);
    assert_eq!(lines[1]["quantity"], 2);
    assert_eq!(lines[1]["price"]["id"], price);
    assert_eq!(lines[2]["amount"], 500);
    assert_eq!(lines[2]["description"], "Setup fee");
    assert!(lines[2]["price"].is_null());
    assert_eq!(lines[2]["currency"], "usd");

    assert_eq!(updated["subtotal"], 3500);
    assert_eq!(updated["tax_total"], 0);
    assert_eq!(updated["total"], 3500);
    assert_eq!(updated["amount_due"], 3500);
    assert_eq!(updated["amount_remaining"], 3500);
}

#[tokio::test]
async fn a_line_with_an_inline_tax_rate_is_added() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price]", price.as_str()),
            ("lines[0][tax_rate][percentage]", "10"),
            ("lines[0][tax_rate][name]", "VAT"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();

    let line = &updated["lines"]["data"][1];
    let tax_amounts = line["tax_amounts"].as_array().unwrap();
    assert_eq!(tax_amounts.len(), 1);
    assert_eq!(tax_amounts[0]["amount"], 100);
    assert_eq!(tax_amounts[0]["tax_rate"]["percentage"], 10);
    assert_eq!(updated["tax_total"], 100);
    assert_eq!(
        updated["total"],
        updated["subtotal"].as_i64().unwrap() + 100
    );
}

#[tokio::test]
async fn an_invalid_line_leaves_the_invoice_unchanged() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;
    let euro_price = create_price_in_currency(&server, &key, 1000, "eur").await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    // An unknown price.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][price]", "price_missing")],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // A price in another currency.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][price]", euro_price.as_str())],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // An amount that is not a non-negative integer.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][amount]", "-100")],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // No lines at all.
    let response = post_lines(&server, &key, &id, "add_lines", &[]).await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "lines");

    // The invoice still holds its original line and totals.
    let invoice = get_invoice(&server, &key, &id).await;
    let lines = invoice["lines"]["data"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["amount"], 1000);
    assert_eq!(invoice["subtotal"], 1000);
    assert_eq!(invoice["total"], 1000);
}

#[tokio::test]
async fn lines_cannot_be_added_to_a_non_draft_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][price]", price.as_str())],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["status"], "open");
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_line_can_be_removed_and_the_totals_follow() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let added: Value = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price]", price.as_str()),
            ("lines[0][quantity]", "2"),
        ],
    )
    .await
    .json()
    .await
    .unwrap();
    let lines = added["lines"]["data"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(added["total"], 3000);
    let first_id = lines[0]["id"].as_str().unwrap().to_string();

    // Removing one line.
    let response = post_lines(
        &server,
        &key,
        &id,
        "remove_lines",
        &[("lines[0][id]", first_id.as_str())],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    let lines = updated["lines"]["data"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["amount"], 2000);
    assert_eq!(updated["subtotal"], 2000);
    assert_eq!(updated["total"], 2000);
    assert_eq!(updated["amount_due"], 2000);
    assert_eq!(updated["amount_remaining"], 2000);

    // Removing every line.
    let remaining_id = lines[0]["id"].as_str().unwrap().to_string();
    let response = post_lines(
        &server,
        &key,
        &id,
        "remove_lines",
        &[("lines[0][id]", remaining_id.as_str())],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    assert_eq!(updated["lines"]["data"].as_array().unwrap().len(), 0);
    assert_eq!(updated["subtotal"], 0);
    assert_eq!(updated["tax_total"], 0);
    assert_eq!(updated["total"], 0);
    assert_eq!(updated["amount_due"], 0);
    assert_eq!(updated["amount_remaining"], 0);
}

#[tokio::test]
async fn removing_a_line_that_is_not_on_the_invoice_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = post_lines(
        &server,
        &key,
        &id,
        "remove_lines",
        &[("lines[0][id]", "il_missing")],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // The invoice still holds its line and totals.
    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 1);
    assert_eq!(invoice["total"], 1000);
}

#[tokio::test]
async fn an_unknown_line_behavior_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let line_id = invoice["lines"]["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let response = post_lines(
        &server,
        &key,
        &id,
        "remove_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][behavior]", "archive"),
        ],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // The invoice still holds its line.
    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn lines_cannot_be_removed_from_a_non_draft_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let line_id = invoice["lines"]["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let response = post_lines(
        &server,
        &key,
        &id,
        "remove_lines",
        &[("lines[0][id]", line_id.as_str())],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_line_quantity_and_tax_rate_can_be_updated() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let line_id = invoice["lines"]["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Updating the quantity recomputes the line amount and the invoice total.
    let response = post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][quantity]", "3"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    let line = &updated["lines"]["data"][0];
    assert_eq!(line["quantity"], 3);
    assert_eq!(line["amount"], 3000);
    assert_eq!(updated["subtotal"], 3000);
    assert_eq!(updated["total"], 3000);

    // Updating the tax rate recomputes the line tax and the invoice tax total.
    let response = post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][tax_rate][percentage]", "20"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    let line = &updated["lines"]["data"][0];
    assert_eq!(line["tax_amounts"][0]["amount"], 600);
    assert_eq!(updated["tax_total"], 600);
    assert_eq!(updated["total"], 3600);

    // Updating the description leaves the total alone.
    let response = post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][description]", "Revised"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    assert_eq!(updated["lines"]["data"][0]["description"], "Revised");
    assert_eq!(updated["total"], 3600);

    // Updating the amount sets it directly.
    let response = post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][amount]", "250"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    assert_eq!(updated["lines"]["data"][0]["amount"], 250);
    assert_eq!(updated["subtotal"], 250);
    assert_eq!(updated["tax_total"], 50);
    assert_eq!(updated["total"], 300);
}

#[tokio::test]
async fn updating_a_line_that_is_not_on_the_invoice_is_rejected() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[("lines[0][id]", "il_missing"), ("lines[0][quantity]", "3")],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // The invoice still holds its original lines and totals.
    let invoice = get_invoice(&server, &key, &id).await;
    let lines = invoice["lines"]["data"].as_array().unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["amount"], 1000);
    assert_eq!(invoice["subtotal"], 1000);
    assert_eq!(invoice["total"], 1000);
}

#[tokio::test]
async fn lines_cannot_be_updated_on_a_non_draft_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let line_id = invoice["lines"]["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let response = post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][quantity]", "3"),
        ],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["lines"]["data"][0]["amount"], 1000);
}

#[tokio::test]
async fn every_line_change_emits_an_invoice_updated_event() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    let added: Value = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][amount]", "500")],
    )
    .await
    .json()
    .await
    .unwrap();
    let added_line_id = added["lines"]["data"][1]["id"]
        .as_str()
        .unwrap()
        .to_string();

    post_lines(
        &server,
        &key,
        &id,
        "remove_lines",
        &[("lines[0][id]", added_line_id.as_str())],
    )
    .await;

    let line_id = added["lines"]["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    post_lines(
        &server,
        &key,
        &id,
        "update_lines",
        &[
            ("lines[0][id]", line_id.as_str()),
            ("lines[0][quantity]", "2"),
        ],
    )
    .await;

    flush(&server).await;
    let state = receiver.state.lock().await;
    let updates: Vec<&serde_json::Value> = state
        .received
        .iter()
        .map(|d| &d.body)
        .filter(|body| body["type"] == "invoice.updated")
        .collect();
    assert_eq!(updates.len(), 3, "{:?}", received_types(&receiver).await);
    for body in &updates {
        assert_eq!(body["data"]["object"]["id"], id);
    }
}

#[tokio::test]
async fn a_rejected_line_change_emits_no_event_and_keeps_the_timestamp() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 1000).await;

    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();
    let before = get_invoice(&server, &key, &id).await;
    let updated_before = before["updated"].as_i64().unwrap();

    // A rejected line change.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][price]", "price_missing")],
    )
    .await;
    assert_eq!(response.status(), 400);

    let after = get_invoice(&server, &key, &id).await;
    assert_eq!(after["updated"].as_i64().unwrap(), updated_before);
    assert_eq!(after["total"], before["total"]);

    flush(&server).await;
    let types = received_types(&receiver).await;
    assert!(!types.contains(&"invoice.updated".to_string()), "{types:?}");
}

// ---------------------------------------------------------------------------
// Reset
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reset_clears_invoices() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    create_invoice(&server, &key, &customer, &price).await;

    server
        .client
        .post(server.url("/internal/reset"))
        .send()
        .await
        .unwrap();

    // The reset re-seeds the main account, so the key changes.
    let key = main_secret(&server).await;
    let list: Value = server
        .client
        .get(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Stripe SDK 22.6 invoice lifecycle
// ---------------------------------------------------------------------------

/// Create a customer with a default currency and return its id.
async fn create_customer_in_currency(
    server: &TestServer,
    key: &str,
    email: &str,
    currency: &str,
) -> String {
    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", email), ("name", "Buyer"), ("currency", currency)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    customer["id"].as_str().unwrap().to_string()
}

/// Create an invoice and return the raw response.
async fn post_invoice(server: &TestServer, key: &str, form: &[(&str, &str)]) -> reqwest::Response {
    server
        .client
        .post(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(form)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn an_invoice_can_be_created_empty_and_given_its_lines_afterwards() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;

    // No lines at all: a valid empty draft, as the SDK creates it.
    let response = post_invoice(&server, &key, &[("customer", customer.as_str())]).await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["status"], "draft");
    assert_eq!(invoice["currency"], "usd");
    assert_eq!(invoice["subtotal"], 0);
    assert_eq!(invoice["tax_total"], 0);
    assert_eq!(invoice["total"], 0);
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 0);
    let id = invoice["id"].as_str().unwrap().to_string();

    // The lines are added afterwards.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price]", price.as_str()),
            ("lines[0][quantity]", "2"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["subtotal"], 4000);
    assert_eq!(invoice["total"], 4000);
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 1);

    // And the invoice moves through the rest of its life.
    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["status"], "open");
    assert_eq!(invoice["amount_due"], 4000);

    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/pay")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["status"], "paid");
    assert_eq!(invoice["amount_paid"], 4000);
    assert_eq!(invoice["amount_remaining"], 0);
}

#[tokio::test]
async fn an_invoice_with_no_lines_cannot_be_finalized() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;

    let response = post_invoice(&server, &key, &[("customer", customer.as_str())]).await;
    let invoice: Value = response.json().await.unwrap();
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // The invoice is still a draft.
    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["status"], "draft");
}

#[tokio::test]
async fn the_invoice_currency_comes_from_the_request() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let euro_price = create_price_in_currency(&server, &key, 1000, "eur").await;
    let dollar_price = create_price(&server, &key, 1000).await;

    // The request names the currency, and the lines follow it.
    let response = post_invoice(
        &server,
        &key,
        &[
            ("customer", customer.as_str()),
            ("currency", "eur"),
            ("lines[0][price]", euro_price.as_str()),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["currency"], "eur");
    assert_eq!(invoice["lines"]["data"][0]["currency"], "eur");

    // A price in another currency than the invoice is rejected.
    let response = post_invoice(
        &server,
        &key,
        &[
            ("customer", customer.as_str()),
            ("currency", "eur"),
            ("lines[0][price]", dollar_price.as_str()),
        ],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // An ad-hoc line is priced in the invoice's currency.
    let response = post_invoice(
        &server,
        &key,
        &[
            ("customer", customer.as_str()),
            ("currency", "eur"),
            ("lines[0][amount]", "500"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["currency"], "eur");
    assert_eq!(invoice["lines"]["data"][0]["currency"], "eur");

    // A malformed currency is rejected.
    let response = post_invoice(
        &server,
        &key,
        &[("customer", customer.as_str()), ("currency", "euro")],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "currency");
}

#[tokio::test]
async fn the_invoice_currency_falls_back_to_the_customer() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let euro_customer = create_customer_in_currency(&server, &key, "eu@example.com", "eur").await;
    let plain_customer = create_customer(&server, &key, "buyer@example.com").await;
    let euro_price = create_price_in_currency(&server, &key, 1000, "eur").await;

    // The customer's currency is used when the request names none.
    let response = post_invoice(
        &server,
        &key,
        &[
            ("customer", euro_customer.as_str()),
            ("lines[0][price]", euro_price.as_str()),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["currency"], "eur");

    // A customer without a currency falls back to usd.
    let response = post_invoice(&server, &key, &[("customer", plain_customer.as_str())]).await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["currency"], "usd");
}

#[tokio::test]
async fn a_draft_invoice_can_be_deleted() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = server
        .client
        .delete(server.url(&format!("/v1/invoices/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let deleted: Value = response.json().await.unwrap();
    assert_eq!(deleted["id"], id.as_str());
    assert_eq!(deleted["object"], "invoice");
    assert_eq!(deleted["deleted"], true);

    // It is gone: no longer readable and no longer listed.
    let response = server
        .client
        .get(server.url(&format!("/v1/invoices/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    let list: Value = server
        .client
        .get(server.url("/v1/invoices"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(!ids.contains(&id.as_str()), "{ids:?}");

    // Deleting it again is rejected.
    let response = server
        .client
        .delete(server.url(&format!("/v1/invoices/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}

#[tokio::test]
async fn an_invoice_that_is_not_a_draft_cannot_be_deleted() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    let response = server
        .client
        .delete(server.url(&format!("/v1/invoices/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // The invoice is untouched.
    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["status"], "open");
}

#[tokio::test]
async fn deleting_an_invoice_emits_an_invoice_deleted_event() {
    let receiver = spawn_receiver().await;
    let server = spawn_with_receiver(Some(receiver.url.clone())).await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    server
        .client
        .delete(server.url(&format!("/v1/invoices/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();

    flush(&server).await;
    let types = received_types(&receiver).await;
    assert!(types.contains(&"invoice.deleted".to_string()), "{types:?}");

    let state = receiver.state.lock().await;
    let deleted = state
        .received
        .iter()
        .find(|d| d.body["type"] == "invoice.deleted")
        .unwrap();
    assert_eq!(deleted.body["data"]["object"]["id"], id);
}

#[tokio::test]
async fn a_customer_carries_its_currency() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", "eu@example.com"), ("currency", "eur")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customer["currency"], "eur");

    // The currency can be changed afterwards.
    let id = customer["id"].as_str().unwrap().to_string();
    let customer: Value = server
        .client
        .post(server.url(&format!("/v1/customers/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("currency", "gbp")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customer["currency"], "gbp");
}

// ---------------------------------------------------------------------------
// Inline price data
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_line_can_be_added_with_inline_price_data() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;

    let invoice = post_invoice(&server, &key, &[("customer", customer.as_str())]).await;
    let invoice: Value = invoice.json().await.unwrap();
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price_data][currency]", "usd"),
            ("lines[0][price_data][unit_amount]", "2500"),
            ("lines[0][price_data][product_data][name]", "Consulting"),
            ("lines[0][quantity]", "2"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["subtotal"], 5000);
    assert_eq!(invoice["total"], 5000);

    let line = &invoice["lines"]["data"][0];
    assert_eq!(line["amount"], 5000);
    assert_eq!(line["quantity"], 2);
    assert_eq!(line["currency"], "usd");
    // The product described inline names the line.
    assert_eq!(line["description"], "Consulting");
    // The price created for the line is carried by the line.
    assert_eq!(line["price"]["object"], "price");
    assert_eq!(line["price"]["unit_amount"], 2500);
    assert_eq!(line["price"]["currency"], "usd");
    assert_eq!(line["price"]["type"], "one_time");

    // The product and the price are real objects, so they can be read back.
    let product_id = line["price"]["product"].as_str().unwrap().to_string();
    let price_id = line["price"]["id"].as_str().unwrap().to_string();
    let product: Value = server
        .client
        .get(server.url(&format!("/v1/products/{product_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(product["id"], product_id.as_str());
    assert_eq!(product["name"], "Consulting");

    let price: Value = server
        .client
        .get(server.url(&format!("/v1/prices/{price_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(price["id"], price_id.as_str());
    assert_eq!(price["unit_amount"], 2500);
    assert_eq!(price["currency"], "usd");
    assert_eq!(price["product"], product_id.as_str());
}

#[tokio::test]
async fn an_inline_price_can_use_a_decimal_amount_and_an_existing_product() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Widget")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product_id = product["id"].as_str().unwrap().to_string();

    let invoice = post_invoice(&server, &key, &[("customer", customer.as_str())]).await;
    let invoice: Value = invoice.json().await.unwrap();
    let id = invoice["id"].as_str().unwrap().to_string();

    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price_data][currency]", "usd"),
            ("lines[0][price_data][unit_amount_decimal]", "20.50"),
            ("lines[0][price_data][product]", product_id.as_str()),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    let line = &invoice["lines"]["data"][0];
    assert_eq!(line["amount"], 2050);
    assert_eq!(line["price"]["unit_amount"], 2050);
    // The product the line names is used as it is.
    assert_eq!(line["price"]["product"], product_id.as_str());
}

#[tokio::test]
async fn an_inline_price_is_checked_against_the_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;

    let invoice = post_invoice(&server, &key, &[("customer", customer.as_str())]).await;
    let invoice: Value = invoice.json().await.unwrap();
    let id = invoice["id"].as_str().unwrap().to_string();

    // Another currency than the invoice.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price_data][currency]", "eur"),
            ("lines[0][price_data][unit_amount]", "100"),
        ],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // An unknown product.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[
            ("lines[0][price_data][currency]", "usd"),
            ("lines[0][price_data][unit_amount]", "100"),
            ("lines[0][price_data][product]", "prod_missing"),
        ],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // A missing currency.
    let response = post_lines(
        &server,
        &key,
        &id,
        "add_lines",
        &[("lines[0][price_data][unit_amount]", "100")],
    )
    .await;
    assert_eq!(response.status(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "lines[][price_data][currency]");

    // The invoice is untouched.
    let invoice = get_invoice(&server, &key, &id).await;
    assert_eq!(invoice["lines"]["data"].as_array().unwrap().len(), 0);
    assert_eq!(invoice["total"], 0);
}

#[tokio::test]
async fn an_invoice_can_be_created_with_an_inline_price() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;

    let response = post_invoice(
        &server,
        &key,
        &[
            ("customer", customer.as_str()),
            ("lines[0][price_data][currency]", "usd"),
            ("lines[0][price_data][unit_amount]", "1500"),
            ("lines[0][price_data][product_data][name]", "Ad offer"),
        ],
    )
    .await;
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();
    assert_eq!(invoice["subtotal"], 1500);
    assert_eq!(invoice["total"], 1500);
    let line = &invoice["lines"]["data"][0];
    assert_eq!(line["amount"], 1500);
    assert_eq!(line["description"], "Ad offer");
    assert_eq!(line["price"]["unit_amount"], 1500);
}

// ---------------------------------------------------------------------------
// Hosted invoice page
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_finalized_invoice_has_a_hosted_page() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_customer(&server, &key, "buyer@example.com").await;
    let price = create_price(&server, &key, 2000).await;
    let invoice = create_invoice(&server, &key, &customer, &price).await;
    let id = invoice["id"].as_str().unwrap().to_string();

    // A draft invoice has no hosted page yet.
    assert_eq!(invoice["hosted_invoice_url"], Value::Null);
    assert_eq!(invoice["invoice_pdf"], Value::Null);

    let response = server
        .client
        .post(server.url(&format!("/v1/invoices/{id}/finalize")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let invoice: Value = response.json().await.unwrap();

    let hosted = format!("{}/mock/invoices/{id}", server.base_url);
    assert_eq!(invoice["hosted_invoice_url"], hosted.as_str());
    assert_eq!(invoice["invoice_pdf"], format!("{hosted}/pdf").as_str());

    // The page is reachable and describes the invoice.
    let response = server
        .client
        .get(server.url(&format!("/mock/invoices/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.text().await.unwrap();
    assert!(body.contains(&id), "{body}");
    assert!(body.contains("Amount due"), "{body}");

    // And so is its PDF.
    let response = server
        .client
        .get(server.url(&format!("/mock/invoices/{id}/pdf")))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/pdf"
    );
    let body = response.bytes().await.unwrap();
    assert!(body.starts_with(b"%PDF-1.4"), "{:?}", &body[..8]);
}

#[tokio::test]
async fn an_unknown_invoice_has_no_hosted_page() {
    let server = spawn().await;

    let response = server
        .client
        .get(server.url("/mock/invoices/in_missing"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);

    let response = server
        .client
        .get(server.url("/mock/invoices/in_missing/pdf"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}
