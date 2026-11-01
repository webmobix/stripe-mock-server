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
