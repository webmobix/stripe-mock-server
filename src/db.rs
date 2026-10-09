use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Executor, Row, SqlitePool};
use std::str::FromStr;

const SCHEMA: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL,
    livemode INTEGER NOT NULL DEFAULT 0,
    is_main INTEGER NOT NULL DEFAULT 0,
    business_type TEXT,
    email TEXT,
    country TEXT,
    default_currency TEXT NOT NULL DEFAULT 'usd',
    identity TEXT,
    configuration TEXT,
    capabilities TEXT,
    requirements TEXT,
    defaults TEXT,
    details_submitted INTEGER NOT NULL DEFAULT 0,
    charges_enabled INTEGER NOT NULL DEFAULT 0,
    payouts_enabled INTEGER NOT NULL DEFAULT 0,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS api_keys (
    key TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    created INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS account_links (
    token TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    link_type TEXT NOT NULL,
    refresh_url TEXT,
    return_url TEXT,
    created INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS products (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    name TEXT,
    description TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created INTEGER NOT NULL,
    default_price TEXT,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS prices (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    product_id TEXT NOT NULL,
    currency TEXT NOT NULL,
    unit_amount INTEGER,
    price_type TEXT NOT NULL,
    recurring_interval TEXT,
    recurring_interval_count INTEGER,
    nickname TEXT,
    active INTEGER NOT NULL DEFAULT 1,
    created INTEGER NOT NULL,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS customers (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    email TEXT,
    name TEXT,
    created INTEGER NOT NULL,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS subscriptions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    customer TEXT,
    customer_account TEXT,
    status TEXT NOT NULL,
    created INTEGER NOT NULL,
    current_period_start INTEGER,
    current_period_end INTEGER,
    cancel_at_period_end INTEGER NOT NULL DEFAULT 0,
    items TEXT NOT NULL,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS checkout_sessions (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
    created INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    success_url TEXT,
    cancel_url TEXT,
    customer TEXT,
    customer_account TEXT,
    line_items TEXT NOT NULL,
    payment_intent TEXT,
    subscription TEXT,
    amount_total INTEGER,
    currency TEXT,
    url TEXT,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS payment_intents (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    amount INTEGER,
    currency TEXT,
    status TEXT NOT NULL,
    customer TEXT,
    created INTEGER NOT NULL,
    checkout_session TEXT,
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    event_account TEXT,
    type TEXT NOT NULL,
    created INTEGER NOT NULL,
    api_version TEXT NOT NULL,
    version TEXT NOT NULL,
    data TEXT NOT NULL,
    request TEXT,
    livemode INTEGER NOT NULL DEFAULT 0
);

-- Event destinations are the single webhook concept: a snapshot or thin
-- destination with a webhook endpoint transport.
CREATE TABLE IF NOT EXISTS event_destinations (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    url TEXT NOT NULL,
    enabled_events TEXT NOT NULL,
    event_payload TEXT NOT NULL DEFAULT 'thin',
    events_from TEXT,
    status TEXT NOT NULL DEFAULT 'enabled',
    metadata TEXT,
    snapshot_api_version TEXT,
    secret TEXT NOT NULL,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS deliveries (
    id TEXT PRIMARY KEY,
    event_id TEXT NOT NULL,
    destination_id TEXT NOT NULL,
    status TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    http_status INTEGER,
    last_error TEXT,
    created INTEGER NOT NULL,
    updated INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS delivery_attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    delivery_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    status TEXT NOT NULL,
    http_status INTEGER,
    error TEXT,
    created INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_products_account ON products(account_id);
CREATE INDEX IF NOT EXISTS idx_prices_account ON prices(account_id);
CREATE INDEX IF NOT EXISTS idx_events_account ON events(account_id);
CREATE INDEX IF NOT EXISTS idx_deliveries_event ON deliveries(event_id);
CREATE INDEX IF NOT EXISTS idx_deliveries_status ON deliveries(status);
CREATE INDEX IF NOT EXISTS idx_keys_account ON api_keys(account_id);
"#;

pub async fn connect(path: &str) -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    Ok(pool)
}

pub async fn connect_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
}

pub async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    pool.execute(sqlx::raw_sql(SCHEMA)).await?;
    migrate_legacy_columns(pool).await?;
    Ok(())
}

/// Bring an existing database created by an older build up to date. `SCHEMA` only
/// runs `CREATE TABLE IF NOT EXISTS`, so a pre-existing table keeps its old column
/// names; without these guards a stale `stripe-mock.db` silently fails every write
/// (for example the pre-unification `deliveries.target_id`). Idempotent.
async fn migrate_legacy_columns(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Renamed by `unify-event-destinations`.
    if has_column(pool, "deliveries", "target_id").await?
        && !has_column(pool, "deliveries", "destination_id").await?
    {
        pool.execute("ALTER TABLE deliveries RENAME COLUMN target_id TO destination_id")
            .await?;
        tracing::info!("migrated schema: deliveries.target_id -> deliveries.destination_id");
    }
    // Columns added to `events` after the initial schema.
    for (column, decl) in [
        ("event_account", "TEXT"),
        ("request", "TEXT"),
        ("livemode", "INTEGER NOT NULL DEFAULT 0"),
    ] {
        add_column_if_missing(pool, "events", column, decl).await?;
    }
    Ok(())
}

/// Whether `table` currently has a column named `column`.
async fn has_column(pool: &SqlitePool, table: &str, column: &str) -> Result<bool, sqlx::Error> {
    let rows = sqlx::query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().any(|row| {
        row.try_get::<String, _>("name")
            .map(|name| name == column)
            .unwrap_or(false)
    }))
}

/// `ALTER TABLE ... ADD COLUMN` when the column is absent (SQLite has no
/// `ADD COLUMN IF NOT EXISTS`).
async fn add_column_if_missing(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    decl: &str,
) -> Result<(), sqlx::Error> {
    if !has_column(pool, table, column).await? {
        pool.execute(format!("ALTER TABLE {table} ADD COLUMN {column} {decl}").as_str())
            .await?;
        tracing::info!("migrated schema: added {table}.{column}");
    }
    Ok(())
}

/// Delete all mutable state, leaving the schema intact. Used by `/internal/reset`
/// and reset-on-start before seeding.
pub async fn reset_state(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for table in [
        "delivery_attempts",
        "deliveries",
        "events",
        "payment_intents",
        "checkout_sessions",
        "subscriptions",
        "customers",
        "prices",
        "products",
        "account_links",
        "api_keys",
        "event_destinations",
        "accounts",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_and_are_idempotent() {
        let pool = connect_in_memory().await.unwrap();
        migrate(&pool).await.unwrap();
        // Re-running must not error.
        migrate(&pool).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn fresh_file_database_migrates() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "stripe-mock-test-{}.db",
            crate::ids::random_string(8)
        ));
        let path_str = path.to_string_lossy().to_string();
        let pool = connect(&path_str).await.unwrap();
        migrate(&pool).await.unwrap();
        migrate(&pool).await.unwrap();
        drop(pool);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn legacy_deliveries_column_is_renamed() {
        let pool = connect_in_memory().await.unwrap();
        // Simulate a database created before `target_id` was renamed.
        sqlx::query(
            "CREATE TABLE deliveries (id TEXT PRIMARY KEY, event_id TEXT NOT NULL, target_id TEXT NOT NULL, \
             status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, http_status INTEGER, last_error TEXT, \
             created INTEGER NOT NULL, updated INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();

        migrate(&pool).await.unwrap();

        assert!(
            has_column(&pool, "deliveries", "destination_id")
                .await
                .unwrap()
        );
        assert!(!has_column(&pool, "deliveries", "target_id").await.unwrap());
        // A delivery insert in the new shape now succeeds.
        sqlx::query(
            "INSERT INTO deliveries (id, event_id, destination_id, status, created, updated) \
             VALUES ('dl_1', 'evt_1', 'ed_test_1', 'pending', 0, 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
    }
}
