mod common;

use common::*;
use serde_json::{Value, json};

const FROZEN: i64 = 1_577_836_800; // 2020-01-01T00:00:00Z
const MONTH: i64 = 30 * 86_400;

async fn create_clock(server: &TestServer, key: &str, frozen_time: i64, name: &str) -> Value {
    server
        .client
        .post(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("frozen_time", frozen_time.to_string().as_str()),
            ("name", name),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn get_clock(server: &TestServer, key: &str, id: &str) -> Value {
    server
        .client
        .get(server.url(&format!("/v1/test_helpers/test_clocks/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn create_and_retrieve_a_test_clock() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let clock = create_clock(&server, &key, FROZEN, "Annual renewal").await;
    assert!(clock["id"].as_str().unwrap().starts_with("clock_"));
    assert_eq!(clock["object"], "test_helpers.test_clock");
    assert_eq!(clock["frozen_time"], FROZEN);
    assert_eq!(clock["name"], "Annual renewal");
    assert_eq!(clock["status"], "ready");
    assert_eq!(clock["livemode"], false);
    assert_eq!(
        clock["deletes_after"].as_i64().unwrap(),
        clock["created"].as_i64().unwrap() + MONTH
    );
    assert!(clock["status_details"]["advancing"].is_null());
    assert!(clock["status_details"]["internal_failure"].is_null());

    let retrieved = get_clock(&server, &key, clock["id"].as_str().unwrap()).await;
    assert_eq!(retrieved, clock);
}

#[tokio::test]
async fn create_rejects_a_missing_frozen_time() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let response = server
        .client
        .post(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "No time")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "parameter_missing");
    assert_eq!(body["error"]["param"], "frozen_time");
}

#[tokio::test]
async fn create_rejects_an_overlong_name() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let long_name = "x".repeat(301);
    let response = server
        .client
        .post(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("frozen_time", FROZEN.to_string().as_str()),
            ("name", long_name.as_str()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["param"], "name");
}

#[tokio::test]
async fn list_test_clocks_is_newest_first() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let first = create_clock(&server, &key, FROZEN, "First").await;
    let second = create_clock(&server, &key, FROZEN + MONTH, "Second").await;

    let list: Value = server
        .client
        .get(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["object"], "list");
    assert_eq!(list["url"], "/v1/test_helpers/test_clocks");
    assert_eq!(list["data"].as_array().unwrap().len(), 2);
    assert_eq!(list["data"][0]["id"], second["id"]);
    assert_eq!(list["data"][1]["id"], first["id"]);

    let page: Value = server
        .client
        .get(server.url("/v1/test_helpers/test_clocks?limit=1"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["data"].as_array().unwrap().len(), 1);
    assert_eq!(page["data"][0]["id"], second["id"]);
    assert_eq!(page["has_more"], true);
}

#[tokio::test]
async fn delete_a_test_clock() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Delete me").await;
    let id = clock["id"].as_str().unwrap();

    let deleted: Value = server
        .client
        .delete(server.url(&format!("/v1/test_helpers/test_clocks/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["id"], clock["id"]);
    assert_eq!(deleted["object"], "test_helpers.test_clock");
    assert_eq!(deleted["deleted"], true);

    let response = server
        .client
        .get(server.url(&format!("/v1/test_helpers/test_clocks/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "resource_missing");
}

#[tokio::test]
async fn unknown_clock_is_not_found() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    for response in [
        server
            .client
            .get(server.url("/v1/test_helpers/test_clocks/clock_missing"))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap(),
        server
            .client
            .delete(server.url("/v1/test_helpers/test_clocks/clock_missing"))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap(),
    ] {
        assert_eq!(response.status().as_u16(), 404);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["type"], "invalid_request_error");
    }
}

#[tokio::test]
async fn clocks_are_account_scoped() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Platform clock").await;

    // Create a connected account and act on it with the platform key.
    let account: Value = server
        .client
        .post(server.url("/internal/accounts"))
        .json(&json!({ "name": "Connected", "state": "active" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let account_key = account["secret_key"].as_str().unwrap();
    let response = server
        .client
        .get(server.url(&format!(
            "/v1/test_helpers/test_clocks/{}",
            clock["id"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {account_key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 404);
}

#[tokio::test]
async fn clock_routes_resolve_with_the_expected_methods() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Routes").await;
    let id = clock["id"].as_str().unwrap();

    // Collection: POST and GET are allowed, DELETE is not.
    let delete_collection = server
        .client
        .delete(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(delete_collection.status().as_u16(), 405);

    // Item: GET and DELETE are allowed, POST is not.
    let post_item = server
        .client
        .post(server.url(&format!("/v1/test_helpers/test_clocks/{id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(post_item.status().as_u16(), 405);

    // Advance exists and only accepts POST.
    let get_advance = server
        .client
        .get(server.url(&format!("/v1/test_helpers/test_clocks/{id}/advance")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_advance.status().as_u16(), 405);
}

async fn create_plain_customer(server: &TestServer, key: &str, email: &str) -> Value {
    server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("email", email)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn create_clock_for_customer(
    server: &TestServer,
    key: &str,
    frozen_time: i64,
    customer: &str,
) -> reqwest::Response {
    server
        .client
        .post(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("frozen_time", frozen_time.to_string().as_str()),
            ("customer", customer),
        ])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn customer_created_on_a_clock_is_stamped_with_frozen_time() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Customer clock").await;

    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("email", "clocked@example.com"),
            ("test_clock", clock["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(customer["test_clock"], clock["id"]);
    assert_eq!(customer["created"], FROZEN);

    // The clock is expandable on the customer.
    let expanded: Value = server
        .client
        .get(server.url(&format!(
            "/v1/customers/{}?expand[]=test_clock",
            customer["id"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(expanded["test_clock"]["id"], clock["id"]);
    assert_eq!(expanded["test_clock"]["object"], "test_helpers.test_clock");

    // `customer.created` is emitted at the frozen time.
    let events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.created&limit=1"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events["data"][0]["created"], FROZEN);
}

#[tokio::test]
async fn a_clock_can_be_attached_to_an_existing_customer() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_plain_customer(&server, &key, "attach@example.com").await;
    let customer_id = customer["id"].as_str().unwrap();

    let response = create_clock_for_customer(&server, &key, FROZEN, customer_id).await;
    assert_eq!(response.status().as_u16(), 200);
    let clock: Value = response.json().await.unwrap();
    assert!(clock["id"].as_str().unwrap().starts_with("clock_"));

    let attached = get_clock(&server, &key, clock["id"].as_str().unwrap()).await;
    assert_eq!(attached["id"], clock["id"]);
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
    assert_eq!(customer["test_clock"], clock["id"]);
    assert_eq!(customer["created"], FROZEN);
}

#[tokio::test]
async fn a_customer_with_live_subscriptions_cannot_join_a_clock() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_plain_customer(&server, &key, "busy@example.com").await;
    let customer_id = customer["id"].as_str().unwrap();
    let price = create_recurring_price(&server, &key, "month", 1000).await;

    let subscription = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer_id),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap();
    assert!(subscription.status().is_success());

    let response = create_clock_for_customer(&server, &key, FROZEN, customer_id).await;
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "customer");

    // The rejected clock was rolled back entirely.
    let list: Value = server
        .client
        .get(server.url("/v1/test_helpers/test_clocks"))
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
async fn a_customer_cannot_join_a_second_clock() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_plain_customer(&server, &key, "twice@example.com").await;
    let customer_id = customer["id"].as_str().unwrap();

    let first = create_clock_for_customer(&server, &key, FROZEN, customer_id).await;
    assert_eq!(first.status().as_u16(), 200);

    let second = create_clock_for_customer(&server, &key, FROZEN + MONTH, customer_id).await;
    assert_eq!(second.status().as_u16(), 400);
    let body: Value = second.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "customer");

    // The rejected clock was rolled back entirely.
    let list: Value = server
        .client
        .get(server.url("/v1/test_helpers/test_clocks"))
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
async fn unknown_test_clock_is_rejected_without_creating_a_customer() {
    let server = spawn().await;
    let key = main_secret(&server).await;

    let response = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("email", "nope@example.com"),
            ("test_clock", "clock_missing"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "test_clock");

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
    assert_eq!(customers["data"].as_array().unwrap().len(), 0);
}

async fn create_recurring_price(
    server: &TestServer,
    key: &str,
    interval: &str,
    amount: i64,
) -> Value {
    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Clock plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product["id"].as_str().unwrap()),
            ("currency", "usd"),
            ("recurring[interval]", interval),
            ("unit_amount", amount.to_string().as_str()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn create_clock_customer(server: &TestServer, key: &str, frozen_time: i64) -> (Value, Value) {
    let clock = create_clock(server, key, frozen_time, "Subscription clock").await;
    let customer: Value = server
        .client
        .post(server.url("/v1/customers"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("email", "clock-sub@example.com"),
            ("test_clock", clock["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (clock, customer)
}

#[tokio::test]
async fn subscription_inherits_the_clock_and_uses_frozen_time() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;

    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer["id"].as_str().unwrap()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(subscription["test_clock"], clock["id"]);
    assert_eq!(subscription["created"], FROZEN);
    assert_eq!(subscription["current_period_start"], FROZEN);
    assert_eq!(subscription["current_period_end"], FROZEN + MONTH);
    let item = &subscription["items"]["data"][0];
    assert_eq!(item["created"], FROZEN);
    assert_eq!(item["current_period_start"], FROZEN);
    assert_eq!(item["current_period_end"], FROZEN + MONTH);

    // The first invoice also inherits the clock.
    let invoices: Value = server
        .client
        .get(server.url(&format!(
            "/v1/invoices?subscription={}",
            subscription["id"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(invoices["data"][0]["test_clock"], clock["id"]);
    assert_eq!(invoices["data"][0]["billing_reason"], "subscription_create");
    assert_eq!(invoices["data"][0]["created"], FROZEN);
    assert_eq!(invoices["data"][0]["paid_at"], FROZEN);

    // Clock-scoped events carry the simulated time.
    for event_type in [
        "customer.subscription.created",
        "invoice.created",
        "invoice.paid",
    ] {
        let events: Value = server
            .client
            .get(server.url(&format!("/v1/events?type={event_type}&limit=1")))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            events["data"][0]["created"], FROZEN,
            "{event_type} was not stamped with the frozen time"
        );
    }
}

#[tokio::test]
async fn clock_subscription_first_period_follows_the_price_interval() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (_clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "week", 500).await;

    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer["id"].as_str().unwrap()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        subscription["current_period_end"].as_i64().unwrap(),
        FROZEN + 7 * 86_400
    );
}

#[tokio::test]
async fn non_clock_subscription_keeps_the_thirty_day_default() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let customer = create_plain_customer(&server, &key, "plain@example.com").await;
    // A weekly price still gets the shared 30-day default without a clock.
    let price = create_recurring_price(&server, &key, "week", 500).await;

    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer["id"].as_str().unwrap()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(subscription["test_clock"].is_null());
    assert_eq!(
        subscription["current_period_end"].as_i64().unwrap()
            - subscription["current_period_start"].as_i64().unwrap(),
        MONTH
    );

    // The non-clock invoice keeps wall-clock timestamps.
    let invoices: Value = server
        .client
        .get(server.url(&format!(
            "/v1/invoices?subscription={}",
            subscription["id"].as_str().unwrap()
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let created = invoices["data"][0]["created"].as_i64().unwrap();
    let wall = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!((created - wall).abs() < 60);
    assert_eq!(
        invoices["data"][0]["paid_at"],
        invoices["data"][0]["created"]
    );
}

#[tokio::test]
async fn checkout_completion_inherits_the_clock() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1500).await;

    let session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "subscription"),
            ("customer", customer["id"].as_str().unwrap()),
            ("line_items[0][price]", price["id"].as_str().unwrap()),
            ("success_url", "https://example.com/success"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(session["customer"], customer["id"]);
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
    assert_eq!(completed["status"], "complete");
    let subscription_id = completed["subscription"].as_str().unwrap();

    let subscription: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(subscription["test_clock"], clock["id"]);
    assert_eq!(subscription["created"], FROZEN);
    assert_eq!(subscription["current_period_start"], FROZEN);
    assert_eq!(subscription["current_period_end"], FROZEN + MONTH);

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
    assert_eq!(invoices["data"][0]["test_clock"], clock["id"]);
    assert_eq!(invoices["data"][0]["status"], "paid");
}

async fn advance_clock(
    server: &TestServer,
    key: &str,
    id: &str,
    frozen_time: i64,
) -> reqwest::Response {
    server
        .client
        .post(server.url(&format!("/v1/test_helpers/test_clocks/{id}/advance")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("frozen_time", frozen_time.to_string().as_str())])
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn advance_reports_advancing_and_commits_ready() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Advance me").await;
    let id = clock["id"].as_str().unwrap();
    let target = FROZEN + MONTH;

    let response = advance_clock(&server, &key, id, target).await;
    assert_eq!(response.status().as_u16(), 200);
    let advanced: Value = response.json().await.unwrap();
    assert_eq!(advanced["status"], "advancing");
    assert_eq!(
        advanced["status_details"]["advancing"]["target_frozen_time"],
        target
    );
    assert_eq!(advanced["frozen_time"], FROZEN);

    // The committed state is ready at the new time.
    let retrieved = get_clock(&server, &key, id).await;
    assert_eq!(retrieved["status"], "ready");
    assert_eq!(retrieved["frozen_time"], target);
    assert!(retrieved["status_details"]["advancing"].is_null());

    // The three lifecycle events exist and carry the clock.
    for (event_type, expected_created) in [
        (
            "test_helpers.test_clock.created",
            clock["created"].as_i64().unwrap(),
        ),
        ("test_helpers.test_clock.advancing", FROZEN),
        ("test_helpers.test_clock.ready", target),
    ] {
        let events: Value = server
            .client
            .get(server.url(&format!("/v1/events?type={event_type}&limit=1")))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            events["data"][0]["created"], expected_created,
            "{event_type}"
        );
        assert_eq!(
            events["data"][0]["data"]["object"]["id"], clock["id"],
            "{event_type} missing the clock object"
        );
    }
}

#[tokio::test]
async fn advance_rejects_backwards_and_missing_times() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Backwards").await;
    let id = clock["id"].as_str().unwrap();

    for target in [FROZEN - 1, FROZEN] {
        let response = advance_clock(&server, &key, id, target).await;
        assert_eq!(response.status().as_u16(), 400);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["param"], "frozen_time");
    }

    let missing = server
        .client
        .post(server.url(&format!("/v1/test_helpers/test_clocks/{id}/advance")))
        .header("Authorization", format!("Bearer {key}"))
        .body("")
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status().as_u16(), 400);
    let body: Value = missing.json().await.unwrap();
    assert_eq!(body["error"]["param"], "frozen_time");
    assert_eq!(body["error"]["code"], "parameter_missing");

    // A rejected advance leaves the clock where it was.
    let retrieved = get_clock(&server, &key, id).await;
    assert_eq!(retrieved["frozen_time"], FROZEN);
    assert_eq!(retrieved["status"], "ready");
}

#[tokio::test]
async fn advance_limits_an_empty_clock_to_two_years() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let clock = create_clock(&server, &key, FROZEN, "Empty").await;
    let id = clock["id"].as_str().unwrap();
    let two_years = 2 * 365 * 86_400;

    let allowed = advance_clock(&server, &key, id, FROZEN + two_years).await;
    assert_eq!(allowed.status().as_u16(), 200);

    // The cap is measured from the current frozen time.
    let rejected = advance_clock(&server, &key, id, FROZEN + 2 * two_years + 1).await;
    assert_eq!(rejected.status().as_u16(), 400);
    let body: Value = rejected.json().await.unwrap();
    assert_eq!(body["error"]["param"], "frozen_time");
}

#[tokio::test]
async fn advance_limits_a_subscribed_clock_to_two_intervals() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (_clock_value, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription: Value = server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer["id"].as_str().unwrap()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let clock_id = subscription["test_clock"].as_str().unwrap();

    // Two months is allowed.
    let allowed = advance_clock(&server, &key, clock_id, FROZEN + 2 * MONTH).await;
    assert_eq!(allowed.status().as_u16(), 200);

    // Anything beyond two monthly intervals is rejected.
    let rejected = advance_clock(&server, &key, clock_id, FROZEN + 4 * MONTH + 1).await;
    assert_eq!(rejected.status().as_u16(), 400);
    let body: Value = rejected.json().await.unwrap();
    assert_eq!(body["error"]["param"], "frozen_time");
}

async fn create_metered_price(server: &TestServer, key: &str, event_name: &str) -> Value {
    let meter: Value = server
        .client
        .post(server.url("/v1/billing/meters"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("display_name", "API calls"),
            ("event_name", event_name),
            ("default_aggregation[formula]", "sum"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let product: Value = server
        .client
        .post(server.url("/v1/products"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("name", "Metered plan")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    server
        .client
        .post(server.url("/v1/prices"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("product", product["id"].as_str().unwrap()),
            ("currency", "usd"),
            ("recurring[interval]", "month"),
            ("recurring[usage_type]", "metered"),
            ("recurring[meter]", meter["id"].as_str().unwrap()),
            ("unit_amount", "2"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn create_clock_subscription(
    server: &TestServer,
    key: &str,
    customer: &Value,
    price: &Value,
) -> Value {
    server
        .client
        .post(server.url("/v1/subscriptions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("customer", customer["id"].as_str().unwrap()),
            ("items[0][price]", price["id"].as_str().unwrap()),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn subscription_invoices(server: &TestServer, key: &str, subscription: &str) -> Vec<Value> {
    let invoices: Value = server
        .client
        .get(server.url(&format!("/v1/invoices?subscription={subscription}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    invoices["data"].as_array().cloned().unwrap_or_default()
}

#[tokio::test]
async fn advancing_one_period_renews_the_subscription() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    let response =
        advance_clock(&server, &key, clock["id"].as_str().unwrap(), FROZEN + MONTH).await;
    assert_eq!(response.status().as_u16(), 200);

    let renewed: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renewed["status"], "active");
    assert_eq!(renewed["current_period_start"], FROZEN + MONTH);
    assert_eq!(renewed["current_period_end"], FROZEN + 2 * MONTH);
    assert_eq!(
        renewed["items"]["data"][0]["current_period_start"],
        FROZEN + MONTH
    );
    assert_eq!(
        renewed["items"]["data"][0]["current_period_end"],
        FROZEN + 2 * MONTH
    );

    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    assert_eq!(invoices.len(), 2);
    let renewal = invoices
        .iter()
        .find(|invoice| invoice["billing_reason"] == "subscription_cycle")
        .expect("missing renewal invoice");
    assert_eq!(renewal["status"], "paid");
    assert_eq!(renewal["created"], FROZEN + MONTH);
    assert_eq!(renewal["paid_at"], FROZEN + MONTH);

    // The renewal raised the standard invoice and subscription events.
    let events: Value = server
        .client
        .get(server.url("/v1/events?type=invoice.paid&limit=1"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events["data"][0]["created"], FROZEN + MONTH);
}

#[tokio::test]
async fn advancing_two_periods_raises_two_renewal_invoices() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    let response = advance_clock(
        &server,
        &key,
        clock["id"].as_str().unwrap(),
        FROZEN + 2 * MONTH,
    )
    .await;
    assert_eq!(response.status().as_u16(), 200);

    let renewed: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renewed["status"], "active");
    assert_eq!(renewed["current_period_start"], FROZEN + 2 * MONTH);
    assert_eq!(renewed["current_period_end"], FROZEN + 3 * MONTH);

    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    assert_eq!(invoices.len(), 3);
    let mut cycles: Vec<&Value> = invoices
        .iter()
        .filter(|invoice| invoice["billing_reason"] == "subscription_cycle")
        .collect();
    cycles.sort_by_key(|invoice| invoice["created"].as_i64().unwrap());
    assert_eq!(cycles.len(), 2);
    assert_eq!(cycles[0]["created"], FROZEN + MONTH);
    assert_eq!(cycles[1]["created"], FROZEN + 2 * MONTH);
    assert!(cycles.iter().all(|invoice| invoice["status"] == "paid"));
}

#[tokio::test]
async fn metered_usage_is_billed_on_the_renewal_invoice() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    // A clock frozen far from wall-clock time still collects usage: the meter
    // event defaults to the clock's frozen time.
    let frozen = FROZEN;
    let (clock, customer) = create_clock_customer(&server, &key, frozen).await;
    let price = create_metered_price(&server, &key, "clock_api_call").await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    let usage_response = server
        .client
        .post(server.url("/v1/billing/meter_events"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("event_name", "clock_api_call"),
            ("identifier", "clock-usage-1"),
            (
                "payload[stripe_customer_id]",
                customer["id"].as_str().unwrap(),
            ),
            ("payload[value]", "7"),
        ])
        .send()
        .await
        .unwrap();
    assert!(usage_response.status().is_success());
    let usage: Value = usage_response.json().await.unwrap();
    assert_eq!(usage["timestamp"], frozen);

    let response =
        advance_clock(&server, &key, clock["id"].as_str().unwrap(), frozen + MONTH).await;
    assert_eq!(response.status().as_u16(), 200);

    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    let renewal = invoices
        .iter()
        .find(|invoice| invoice["billing_reason"] == "subscription_cycle")
        .expect("missing renewal invoice");
    assert_eq!(renewal["total"], 14);
    let line = &renewal["lines"]["data"][0];
    assert_eq!(line["quantity"], 7);
    assert_eq!(line["period"]["start"], frozen);
    assert_eq!(line["period"]["end"], frozen + MONTH);
}

#[tokio::test]
async fn an_open_invoice_is_recovered_before_renewing() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    // Model a failed first collection.
    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    let failed_id = invoices[0]["id"].as_str().unwrap();
    sqlx::query(
        "UPDATE invoices SET status = 'open', amount_paid = 0, amount_remaining = amount_due,
            paid_at = NULL, attempt_count = 1 WHERE id = ?1",
    )
    .bind(failed_id)
    .execute(&server.pool)
    .await
    .unwrap();

    let response =
        advance_clock(&server, &key, clock["id"].as_str().unwrap(), FROZEN + MONTH).await;
    assert_eq!(response.status().as_u16(), 200);

    // The open invoice was retried at the clock time before the renewal.
    let recovered: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{failed_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recovered["status"], "paid");
    assert_eq!(recovered["attempt_count"], 2);
    assert_eq!(recovered["paid_at"], FROZEN);

    // The renewal still ran after recovery.
    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    let renewal = invoices
        .iter()
        .find(|invoice| invoice["billing_reason"] == "subscription_cycle")
        .expect("missing renewal invoice");
    assert_eq!(renewal["status"], "paid");
}

#[tokio::test]
async fn internal_advance_uses_the_clock_frozen_time() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    // Drive a failed collection through the internal endpoint, which must
    // follow the subscription's clock instead of wall-clock time.
    let failed: Value = server
        .client
        .post(server.url(&format!(
            "/internal/subscriptions/{subscription_id}/advance"
        )))
        .json(&json!({ "outcome": "failed" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(failed["invoice"]["status"], "open");
    assert_eq!(failed["invoice"]["created"], FROZEN);
    assert_eq!(failed["subscription"]["status"], "past_due");

    let invoice_id = failed["invoice"]["id"].as_str().unwrap().to_string();
    let response =
        advance_clock(&server, &key, clock["id"].as_str().unwrap(), FROZEN + MONTH).await;
    assert_eq!(response.status().as_u16(), 200);

    // Recovery is stamped with the same simulated time, never earlier than
    // the invoice it pays.
    let recovered: Value = server
        .client
        .get(server.url(&format!("/v1/invoices/{invoice_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(recovered["status"], "paid");
    assert_eq!(recovered["created"], FROZEN);
    assert_eq!(recovered["paid_at"], FROZEN);
}

#[tokio::test]
async fn cancel_at_period_end_stops_at_the_boundary() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    let updated: Value = server
        .client
        .post(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[("cancel_at_period_end", "true")])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["cancel_at_period_end"], true);

    let response = advance_clock(
        &server,
        &key,
        clock["id"].as_str().unwrap(),
        FROZEN + 2 * MONTH,
    )
    .await;
    assert_eq!(response.status().as_u16(), 200);

    let canceled: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(canceled["status"], "canceled");
    assert_eq!(canceled["current_period_end"], FROZEN + MONTH);

    // No renewal invoice was raised for the canceled subscription.
    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    assert_eq!(invoices.len(), 1);
    assert_eq!(invoices[0]["billing_reason"], "subscription_create");

    let events: Value = server
        .client
        .get(server.url("/v1/events?type=customer.subscription.deleted&limit=1"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(events["data"][0]["created"], FROZEN + MONTH);
}

#[tokio::test]
async fn deleting_a_clock_cancels_and_removes_its_objects() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();
    let customer_id = customer["id"].as_str().unwrap();
    let clock_id = clock["id"].as_str().unwrap();

    // A saved card on the clock customer, from a completed setup session.
    let setup_session: Value = server
        .client
        .post(server.url("/v1/checkout/sessions"))
        .header("Authorization", format!("Bearer {key}"))
        .form(&[
            ("mode", "setup"),
            ("customer", customer_id),
            ("success_url", "https://example.com/success"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    server
        .client
        .post(server.url(&format!(
            "/mock/checkout/{}",
            setup_session["id"].as_str().unwrap()
        )))
        .form(&[("card", "success")])
        .send()
        .await
        .unwrap();
    let with_card: Value = server
        .client
        .get(server.url(&format!(
            "/v1/customers/{customer_id}?expand[]=invoice_settings.default_payment_method"
        )))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let payment_method_id = with_card["invoice_settings"]["default_payment_method"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let invoices = subscription_invoices(&server, &key, subscription_id).await;
    let invoice_id = invoices[0]["id"].as_str().unwrap().to_string();

    let deleted: Value = server
        .client
        .delete(server.url(&format!("/v1/test_helpers/test_clocks/{clock_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(deleted["deleted"], true);

    // The subscription is canceled.
    let canceled: Value = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(canceled["status"], "canceled");

    // The customer and its payment method are gone.
    let customer_response = server
        .client
        .get(server.url(&format!("/v1/customers/{customer_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(customer_response.status().as_u16(), 404);
    let pm_response = server
        .client
        .get(server.url(&format!("/v1/payment_methods/{payment_method_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(pm_response.status().as_u16(), 404);

    // The invoice stays readable and keeps its clock reference.
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
    assert_eq!(invoice["test_clock"], clock_id);

    // The clock is unreadable.
    let clock_response = server
        .client
        .get(server.url(&format!("/v1/test_helpers/test_clocks/{clock_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(clock_response.status().as_u16(), 404);

    // Every deletion event was emitted.
    for event_type in [
        "customer.subscription.deleted",
        "customer.deleted",
        "test_helpers.test_clock.deleted",
    ] {
        let events: Value = server
            .client
            .get(server.url(&format!("/v1/events?type={event_type}&limit=1")))
            .header("Authorization", format!("Bearer {key}"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            !events["data"].as_array().unwrap().is_empty(),
            "{event_type} was not emitted"
        );
    }
}

#[tokio::test]
async fn reset_clears_clocks_and_their_associations() {
    let server = spawn().await;
    let key = main_secret(&server).await;
    let (_clock, customer) = create_clock_customer(&server, &key, FROZEN).await;
    let price = create_recurring_price(&server, &key, "month", 1000).await;
    let subscription = create_clock_subscription(&server, &key, &customer, &price).await;
    let subscription_id = subscription["id"].as_str().unwrap();

    let reset = server
        .client
        .post(server.url("/internal/reset"))
        .send()
        .await
        .unwrap();
    assert!(reset.status().is_success());

    // Reset re-seeds the main account, so fetch the fresh secret key.
    let key = main_secret(&server).await;
    let clocks: Value = server
        .client
        .get(server.url("/v1/test_helpers/test_clocks"))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(clocks["data"].as_array().unwrap().is_empty());

    // The associations are gone with their rows.
    let subscription_response = server
        .client
        .get(server.url(&format!("/v1/subscriptions/{subscription_id}")))
        .header("Authorization", format!("Bearer {key}"))
        .send()
        .await
        .unwrap();
    assert_eq!(subscription_response.status().as_u16(), 404);

    for table in ["test_clocks", "customers", "subscriptions", "invoices"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&server.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table} was not cleared");
    }
}
