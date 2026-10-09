use std::path::PathBuf;

use stripe_mock::config::Config;
use stripe_mock::{db, seed};

fn temp_db() -> PathBuf {
    std::env::temp_dir().join(format!(
        "stripe-mock-persist-{}.db",
        stripe_mock::ids::random_string(10)
    ))
}

fn config_for(path: &str) -> Config {
    let text = format!(
        r#"
[server]
bind = "127.0.0.1"
port = 12199
public_base_url = "http://127.0.0.1:12199"
[database]
path = "{path}"
[main_account]
business_type = "company"
country = "US"
email = "owner@example.com"
name = "Mock Platform"
"#
    );
    Config::from_str(&text).unwrap()
}

async fn read_keys(pool: &sqlx::SqlitePool, account_id: &str) -> Vec<String> {
    let mut keys: Vec<String> =
        sqlx::query_scalar("SELECT key FROM api_keys WHERE account_id = ?1 ORDER BY key")
            .bind(account_id)
            .fetch_all(pool)
            .await
            .unwrap();
    keys.sort();
    keys
}

#[tokio::test]
async fn generated_keys_and_secrets_are_stable_across_restart() {
    let path = temp_db();
    let path_str = path.to_string_lossy().to_string();
    let config = config_for(&path_str);

    let pool = db::connect(&path_str).await.unwrap();
    db::migrate(&pool).await.unwrap();
    seed::seed(&pool, &config).await.unwrap();
    let account_id: String = sqlx::query_scalar("SELECT id FROM accounts WHERE is_main = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let keys_before = read_keys(&pool, &account_id).await;
    assert_eq!(keys_before.len(), 2);
    pool.close().await;

    let pool = db::connect(&path_str).await.unwrap();
    db::migrate(&pool).await.unwrap();
    seed::seed(&pool, &config).await.unwrap();
    let account_id_after: String = sqlx::query_scalar("SELECT id FROM accounts WHERE is_main = 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let keys_after = read_keys(&pool, &account_id_after).await;

    assert_eq!(account_id, account_id_after);
    assert_eq!(keys_before, keys_after);
    pool.close().await;
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn event_destination_secret_is_stable_across_restart() {
    let path = temp_db();
    let path_str = path.to_string_lossy().to_string();
    let config = Config::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1"
port = 12198
public_base_url = "http://127.0.0.1:12198"
[database]
path = "{path_str}"
[main_account]

[[event_destinations]]
name = "platform-app"
event_payload = "snapshot"
enabled_events = ["*"]
webhook_endpoint = {{ url = "http://localhost:4000/hook" }}
"#
    ))
    .unwrap();

    let pool = db::connect(&path_str).await.unwrap();
    db::migrate(&pool).await.unwrap();
    seed::seed(&pool, &config).await.unwrap();
    let secret_before: String =
        sqlx::query_scalar("SELECT secret FROM event_destinations WHERE name = 'platform-app'")
            .fetch_one(&pool)
            .await
            .unwrap();
    pool.close().await;

    let pool = db::connect(&path_str).await.unwrap();
    db::migrate(&pool).await.unwrap();
    seed::seed(&pool, &config).await.unwrap();
    let secret_after: String =
        sqlx::query_scalar("SELECT secret FROM event_destinations WHERE name = 'platform-app'")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(secret_before.starts_with("whsec_"));
    assert_eq!(secret_before, secret_after);
    pool.close().await;
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn snapshot_and_thin_destinations_can_share_a_name() {
    let path = temp_db();
    let path_str = path.to_string_lossy().to_string();
    let config = Config::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1"
port = 12197
public_base_url = "http://127.0.0.1:12197"
[database]
path = "{path_str}"
[main_account]

[[event_destinations]]
name = "app"
event_payload = "snapshot"
enabled_events = ["*"]
webhook_endpoint = {{ url = "http://localhost:4000/hook" }}

[[event_destinations]]
name = "app"
event_payload = "thin"
enabled_events = ["*"]
webhook_endpoint = {{ url = "http://localhost:4000/hook" }}
"#
    ))
    .unwrap();

    let pool = db::connect(&path_str).await.unwrap();
    db::migrate(&pool).await.unwrap();
    seed::seed(&pool, &config).await.unwrap();
    seed::seed(&pool, &config).await.unwrap();
    let payloads: Vec<String> = sqlx::query_scalar(
        "SELECT event_payload FROM event_destinations WHERE name = 'app' ORDER BY event_payload",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(payloads, vec!["snapshot", "thin"]);
    pool.close().await;
    let _ = std::fs::remove_file(&path);
}
