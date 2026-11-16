//! Response expansion for `expand[]` parameters.
//!
//! A path is dot-separated. At an object the walker reads the named field; at
//! an array it applies the same segment to every element. A reference id is
//! replaced with the full object it names when the field name matches one of
//! the supported loaders. Unknown paths, unresolvable ids, and references that
//! belong to another account leave the original value untouched.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use sqlx::SqlitePool;

use crate::models::{
    CustomerRow, InvoiceRow, PaymentIntentRow, PaymentMethodRow, PriceRow, ProductRow,
    SubscriptionRow, TestClockRow,
};

/// Expand every path in `paths` over `value`, returning the modified value.
pub async fn expand_response(
    pool: &SqlitePool,
    account_id: &str,
    base_url: &str,
    mut value: Value,
    paths: &[String],
) -> Value {
    for path in paths {
        let segments: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            continue;
        }
        walk(pool, account_id, base_url, &mut value, &segments).await;
    }
    value
}

/// Expand every path in `paths` over a list envelope and each `data` item.
///
/// The envelope pass makes envelope-relative paths such as `data.product`
/// work, while the per-item pass preserves item-relative paths such as
/// `product`.
pub async fn expand_list_response(
    pool: &SqlitePool,
    account_id: &str,
    base_url: &str,
    mut value: Value,
    paths: &[String],
) -> Value {
    value = expand_response(pool, account_id, base_url, value, paths).await;
    if let Some(Value::Array(items)) = value.get_mut("data") {
        for item in items {
            let current = std::mem::take(item);
            *item = expand_response(pool, account_id, base_url, current, paths).await;
        }
    }
    value
}

fn walk<'a>(
    pool: &'a SqlitePool,
    account_id: &'a str,
    base_url: &'a str,
    slot: &'a mut Value,
    segments: &'a [&'a str],
) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let Some((segment, rest)) = segments.split_first() else {
            return;
        };
        match slot {
            Value::Array(items) => {
                for item in items {
                    walk(pool, account_id, base_url, item, segments).await;
                }
            }
            Value::Object(map) => {
                let Some(child) = map.get_mut(*segment) else {
                    return;
                };
                if let Some(object) = resolve(pool, account_id, base_url, segment, child).await {
                    *child = object;
                }
                if !rest.is_empty() {
                    walk(pool, account_id, base_url, child, rest).await;
                }
            }
            _ => {}
        }
    })
}

/// Resolve a reference id held in a field named `field`, scoped to the account.
async fn resolve(
    pool: &SqlitePool,
    account_id: &str,
    base_url: &str,
    field: &str,
    value: &Value,
) -> Option<Value> {
    let id = value.as_str()?;
    match field {
        "product" if id.starts_with("prod_") => {
            let row: ProductRow =
                sqlx::query_as("SELECT * FROM products WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "price" if id.starts_with("price_") => {
            let row: PriceRow =
                sqlx::query_as("SELECT * FROM prices WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "customer" if id.starts_with("cus_") => {
            let row: CustomerRow =
                sqlx::query_as("SELECT * FROM customers WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "subscription" if id.starts_with("sub_") => {
            let row: SubscriptionRow =
                sqlx::query_as("SELECT * FROM subscriptions WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "payment_intent" if id.starts_with("pi_") => {
            let row: PaymentIntentRow =
                sqlx::query_as("SELECT * FROM payment_intents WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "payment_method" | "default_payment_method" if id.starts_with("pm_") => {
            let row: PaymentMethodRow =
                sqlx::query_as("SELECT * FROM payment_methods WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "test_clock" if id.starts_with("clock_") => {
            let row: TestClockRow =
                sqlx::query_as("SELECT * FROM test_clocks WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json())
        }
        "latest_invoice" if id.starts_with("in_") => {
            let row: InvoiceRow =
                sqlx::query_as("SELECT * FROM invoices WHERE id = ?1 AND account_id = ?2")
                    .bind(id)
                    .bind(account_id)
                    .fetch_optional(pool)
                    .await
                    .ok()??;
            Some(row.to_json(base_url))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn pool() -> SqlitePool {
        let pool = crate::db::connect_in_memory().await.unwrap();
        crate::db::migrate(&pool).await.unwrap();
        pool
    }

    async fn add_product(pool: &SqlitePool, account: &str, id: &str) {
        sqlx::query(
            "INSERT INTO products (id, account_id, name, active, created, metadata) \
             VALUES (?1, ?2, 'Plan', 1, 0, '{}')",
        )
        .bind(id)
        .bind(account)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn expands_a_nested_list_path() {
        let pool = pool().await;
        add_product(&pool, "acct_1", "prod_1").await;
        let mut value = json!({
            "items": {
                "object": "list",
                "data": [
                    { "price": { "id": "price_1", "product": "prod_1" } },
                    { "price": { "id": "price_2", "product": "prod_1" } }
                ]
            }
        });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["items.data.price.product".into()],
        )
        .await;
        assert_eq!(
            value["items"]["data"][0]["price"]["product"]["id"],
            "prod_1"
        );
        assert_eq!(
            value["items"]["data"][0]["price"]["product"]["object"],
            "product"
        );
        assert_eq!(
            value["items"]["data"][1]["price"]["product"]["id"],
            "prod_1"
        );
    }

    #[tokio::test]
    async fn expands_a_list_envelope_path() {
        let pool = pool().await;
        add_product(&pool, "acct_1", "prod_1").await;
        let value = json!({
            "object": "list",
            "data": [
                { "id": "price_1", "product": "prod_1" },
                { "id": "price_2", "product": "prod_1" }
            ]
        });
        let value = expand_list_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["data.product".into()],
        )
        .await;
        assert_eq!(value["data"][0]["product"]["id"], "prod_1");
        assert_eq!(value["data"][0]["product"]["object"], "product");
        assert_eq!(value["data"][1]["product"]["id"], "prod_1");
    }

    #[tokio::test]
    async fn expands_a_list_item_relative_path() {
        let pool = pool().await;
        add_product(&pool, "acct_1", "prod_1").await;
        let value = json!({
            "object": "list",
            "data": [
                { "id": "price_1", "product": "prod_1" },
                { "id": "price_2", "product": "prod_1" }
            ]
        });
        let value = expand_list_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["product".into()],
        )
        .await;
        assert_eq!(value["data"][0]["product"]["id"], "prod_1");
        assert_eq!(value["data"][1]["product"]["object"], "product");
    }

    #[tokio::test]
    async fn expands_a_nested_list_envelope_path() {
        let pool = pool().await;
        add_product(&pool, "acct_1", "prod_1").await;
        let value = json!({
            "object": "list",
            "data": [
                {
                    "id": "sub_1",
                    "items": {
                        "data": [
                            { "price": { "id": "price_1", "product": "prod_1" } }
                        ]
                    }
                }
            ]
        });
        let value = expand_list_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["data.items.data.price.product".into()],
        )
        .await;
        assert_eq!(
            value["data"][0]["items"]["data"][0]["price"]["product"]["id"],
            "prod_1"
        );
        assert_eq!(
            value["data"][0]["items"]["data"][0]["price"]["product"]["object"],
            "product"
        );
    }

    #[tokio::test]
    async fn an_already_expanded_object_is_left_alone() {
        let pool = pool().await;
        let expanded = json!({"id": "prod_1", "object": "product"});
        let mut value = json!({ "product": expanded.clone() });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["product".into()],
        )
        .await;
        assert_eq!(value["product"], expanded);
    }

    #[tokio::test]
    async fn unknown_paths_are_untouched() {
        let pool = pool().await;
        let mut value = json!({ "id": "price_1", "product": "prod_1" });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["does.not.exist".into()],
        )
        .await;
        assert_eq!(value["product"], "prod_1");
        assert!(value.get("does").is_none());
    }

    #[tokio::test]
    async fn an_id_from_another_account_stays_an_id() {
        let pool = pool().await;
        add_product(&pool, "acct_2", "prod_other").await;
        let mut value = json!({ "product": "prod_other" });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["product".into()],
        )
        .await;
        assert_eq!(value["product"], "prod_other");
    }

    #[tokio::test]
    async fn expands_a_test_clock_scoped_to_the_account() {
        let pool = pool().await;
        for (account, id) in [("acct_1", "clock_mine"), ("acct_2", "clock_other")] {
            sqlx::query(
                "INSERT INTO test_clocks (id, account_id, name, frozen_time, status, created, deletes_after) \
                 VALUES (?1, ?2, NULL, 0, 'ready', 0, 0)",
            )
            .bind(id)
            .bind(account)
            .execute(&pool)
            .await
            .unwrap();
        }
        let mut value = json!({ "test_clock": "clock_mine" });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["test_clock".into()],
        )
        .await;
        assert_eq!(value["test_clock"]["id"], "clock_mine");
        assert_eq!(value["test_clock"]["object"], "test_helpers.test_clock");
        assert_eq!(value["test_clock"]["frozen_time"], 0);

        let mut other = json!({ "test_clock": "clock_other" });
        other = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            other,
            &["test_clock".into()],
        )
        .await;
        assert_eq!(other["test_clock"], "clock_other");
    }

    #[tokio::test]
    async fn expands_latest_invoice_with_a_rendered_invoice() {
        let pool = pool().await;
        sqlx::query(
            "INSERT INTO invoices (id, account_id, customer, status, created, updated) \
             VALUES ('in_1', 'acct_1', 'cus_1', 'paid', 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let mut value = json!({ "latest_invoice": "in_1" });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["latest_invoice".into()],
        )
        .await;
        assert_eq!(value["latest_invoice"]["object"], "invoice");
        assert_eq!(value["latest_invoice"]["id"], "in_1");
        assert_eq!(
            value["latest_invoice"]["hosted_invoice_url"],
            "http://127.0.0.1:12111/mock/invoices/in_1"
        );
    }

    #[tokio::test]
    async fn expands_a_price_reference_before_following_the_path() {
        let pool = pool().await;
        add_product(&pool, "acct_1", "prod_1").await;
        sqlx::query(
            "INSERT INTO prices (id, account_id, product_id, currency, unit_amount, price_type, active, created, metadata) \
             VALUES ('price_1', 'acct_1', 'prod_1', 'usd', 1000, 'recurring', 1, 0, '{}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        // A subscription item whose price is still a bare id.
        let mut value = json!({ "items": { "data": [ { "price": "price_1" } ] } });
        value = expand_response(
            &pool,
            "acct_1",
            "http://127.0.0.1:12111",
            value,
            &["items.data.price.product".into()],
        )
        .await;
        assert_eq!(value["items"]["data"][0]["price"]["id"], "price_1");
        assert_eq!(
            value["items"]["data"][0]["price"]["product"]["id"],
            "prod_1"
        );
    }
}
