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

use crate::models::{CustomerRow, PaymentIntentRow, PriceRow, ProductRow, SubscriptionRow};

/// Expand every path in `paths` over `value`, returning the modified value.
pub async fn expand_response(
    pool: &SqlitePool,
    account_id: &str,
    mut value: Value,
    paths: &[String],
) -> Value {
    for path in paths {
        let segments: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            continue;
        }
        walk(pool, account_id, &mut value, &segments).await;
    }
    value
}

fn walk<'a>(
    pool: &'a SqlitePool,
    account_id: &'a str,
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
                    walk(pool, account_id, item, segments).await;
                }
            }
            Value::Object(map) => {
                let Some(child) = map.get_mut(*segment) else {
                    return;
                };
                if let Some(object) = resolve(pool, account_id, segment, child).await {
                    *child = object;
                }
                if !rest.is_empty() {
                    walk(pool, account_id, child, rest).await;
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
    field: &str,
    value: &Value,
) -> Option<Value> {
    let id = value.as_str()?;
    match field {
        "product" if id.starts_with("prod_") => {
            let row: ProductRow = sqlx::query_as(
                "SELECT * FROM products WHERE id = ?1 AND account_id = ?2",
            )
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
        value = expand_response(&pool, "acct_1", value, &["items.data.price.product".into()]).await;
        assert_eq!(value["items"]["data"][0]["price"]["product"]["id"], "prod_1");
        assert_eq!(value["items"]["data"][0]["price"]["product"]["object"], "product");
        assert_eq!(value["items"]["data"][1]["price"]["product"]["id"], "prod_1");
    }

    #[tokio::test]
    async fn an_already_expanded_object_is_left_alone() {
        let pool = pool().await;
        let expanded = json!({"id": "prod_1", "object": "product"});
        let mut value = json!({ "product": expanded.clone() });
        value = expand_response(&pool, "acct_1", value, &["product".into()]).await;
        assert_eq!(value["product"], expanded);
    }

    #[tokio::test]
    async fn unknown_paths_are_untouched() {
        let pool = pool().await;
        let mut value = json!({ "id": "price_1", "product": "prod_1" });
        value = expand_response(&pool, "acct_1", value, &["does.not.exist".into()]).await;
        assert_eq!(value["product"], "prod_1");
        assert!(value.get("does").is_none());
    }

    #[tokio::test]
    async fn an_id_from_another_account_stays_an_id() {
        let pool = pool().await;
        add_product(&pool, "acct_2", "prod_other").await;
        let mut value = json!({ "product": "prod_other" });
        value = expand_response(&pool, "acct_1", value, &["product".into()]).await;
        assert_eq!(value["product"], "prod_other");
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
        value = expand_response(&pool, "acct_1", value, &["items.data.price.product".into()]).await;
        assert_eq!(value["items"]["data"][0]["price"]["id"], "price_1");
        assert_eq!(
            value["items"]["data"][0]["price"]["product"]["id"],
            "prod_1"
        );
    }
}
