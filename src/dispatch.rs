use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::models::{DeliveryRow, EventDestinationRow, EventRow};
use crate::time;

type HmacSha256 = Hmac<Sha256>;

/// Strategy seam for delivering events to webhook targets.
#[async_trait]
pub trait DeliveryEngine: Send + Sync {
    /// Queue a delivery for asynchronous, ordered sending.
    async fn enqueue(&self, destination_id: &str, delivery_id: &str);
}

/// Compute the `t=<ts>,v1=<hmac>` Stripe-Signature header value.
pub fn sign_payload(secret: &str, timestamp: i64, payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(format!("{timestamp}.{payload}").as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    format!("t={timestamp},v1={signature}")
}

/// Per-target FIFO delivery engine with a fixed delay and inline retries.
pub struct Dispatcher {
    pool: SqlitePool,
    client: reqwest::Client,
    senders: Mutex<HashMap<String, mpsc::UnboundedSender<String>>>,
    delay: Duration,
    max_attempts: u32,
}

impl Dispatcher {
    pub fn new(pool: SqlitePool) -> Arc<Self> {
        Arc::new(Self {
            pool,
            client: reqwest::Client::new(),
            senders: Mutex::new(HashMap::new()),
            delay: Duration::from_millis(200),
            max_attempts: 3,
        })
    }

    #[cfg(test)]
    pub fn new_with_delay(pool: SqlitePool, delay: Duration, max_attempts: u32) -> Arc<Self> {
        Arc::new(Self {
            pool,
            client: reqwest::Client::new(),
            senders: Mutex::new(HashMap::new()),
            delay,
            max_attempts,
        })
    }

    /// Spawn a worker for every event destination. Call after seeding or reset.
    pub async fn start(&self) -> Result<(), sqlx::Error> {
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM event_destinations")
            .fetch_all(&self.pool)
            .await?;
        for id in ids {
            self.ensure_destination(&id);
        }
        // Resume deliveries persisted as pending before a restart or reset.
        let pending: Vec<(String, String)> =
            sqlx::query_as("SELECT id, destination_id FROM deliveries WHERE status = 'pending' ORDER BY created, id")
                .fetch_all(&self.pool)
                .await?;
        for (delivery_id, destination_id) in pending {
            self.send(&destination_id, &delivery_id);
        }
        Ok(())
    }

    fn ensure_destination(&self, destination_id: &str) {
        let mut senders = self.senders.lock().expect("senders lock");
        if senders.contains_key(destination_id) {
            return;
        }
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        senders.insert(destination_id.to_string(), tx);
        let pool = self.pool.clone();
        let client = self.client.clone();
        let delay = self.delay;
        let max_attempts = self.max_attempts;
        tokio::spawn(async move {
            while let Some(delivery_id) = rx.recv().await {
                tokio::time::sleep(delay).await;
                process_delivery(&pool, &client, &delivery_id, max_attempts).await;
            }
        });
    }

    fn send(&self, destination_id: &str, delivery_id: &str) {
        self.ensure_destination(destination_id);
        let senders = self.senders.lock().expect("senders lock");
        if let Some(sender) = senders.get(destination_id) {
            let _ = sender.send(delivery_id.to_string());
        }
    }

    /// Enqueue every still-pending delivery for an event. Call after commit.
    pub async fn dispatch_event(&self, event_id: &str) -> Result<(), sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, destination_id FROM deliveries WHERE event_id = ?1 AND status = 'pending'",
        )
        .bind(event_id)
        .fetch_all(&self.pool)
        .await?;
        for (delivery_id, destination_id) in rows {
            self.send(&destination_id, &delivery_id);
        }
        Ok(())
    }

    /// Block until every delivery for an event reaches a terminal state.
    pub async fn wait_for_event(
        &self,
        event_id: &str,
        timeout: Duration,
    ) -> Result<Vec<DeliveryRow>, ()> {
        let deadline = Instant::now() + timeout;
        loop {
            let pending: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM deliveries WHERE event_id = ?1 AND status = 'pending'",
            )
            .bind(event_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|_| ())?;
            if pending == 0 {
                let rows: Vec<DeliveryRow> =
                    sqlx::query_as("SELECT * FROM deliveries WHERE event_id = ?1")
                        .bind(event_id)
                        .fetch_all(&self.pool)
                        .await
                        .map_err(|_| ())?;
                return Ok(rows);
            }
            if Instant::now() >= deadline {
                return Err(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Block until no pending deliveries remain.
    pub async fn flush(&self, timeout: Duration) -> Result<(), ()> {
        let deadline = Instant::now() + timeout;
        loop {
            let pending: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM deliveries WHERE status = 'pending'")
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|_| ())?;
            if pending == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Reset a terminal delivery and re-queue it.
    pub async fn retry(&self, delivery_id: &str) -> Result<bool, sqlx::Error> {
        let destination_id: Option<String> =
            sqlx::query_scalar("SELECT destination_id FROM deliveries WHERE id = ?1")
                .bind(delivery_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some(destination_id) = destination_id else {
            return Ok(false);
        };
        sqlx::query("UPDATE deliveries SET status = 'pending', attempts = 0, http_status = NULL, last_error = NULL, updated = ?1 WHERE id = ?2")
            .bind(time::now_epoch())
            .bind(delivery_id)
            .execute(&self.pool)
            .await?;
        self.send(&destination_id, delivery_id);
        Ok(true)
    }
}

#[async_trait]
impl DeliveryEngine for Dispatcher {
    async fn enqueue(&self, destination_id: &str, delivery_id: &str) {
        self.send(destination_id, delivery_id);
    }
}

async fn process_delivery(
    pool: &SqlitePool,
    client: &reqwest::Client,
    delivery_id: &str,
    max_attempts: u32,
) {
    let delivery: Option<DeliveryRow> =
        match sqlx::query_as("SELECT * FROM deliveries WHERE id = ?1")
            .bind(delivery_id)
            .fetch_optional(pool)
            .await
        {
            Ok(row) => row,
            Err(_) => return,
        };
    let Some(delivery) = delivery else { return };
    if delivery.status != "pending" {
        return;
    }
    let event: EventRow = match sqlx::query_as("SELECT * FROM events WHERE id = ?1")
        .bind(&delivery.event_id)
        .fetch_one(pool)
        .await
    {
        Ok(row) => row,
        Err(_) => {
            mark_delivery(
                pool,
                &delivery.id,
                "failed_exhausted",
                0,
                None,
                Some("event not found"),
            )
            .await;
            return;
        }
    };

    // Resolve the destination and render the payload in its configured format.
    let destination: EventDestinationRow =
        match sqlx::query_as("SELECT * FROM event_destinations WHERE id = ?1")
            .bind(&delivery.destination_id)
            .fetch_one(pool)
            .await
        {
            Ok(row) => row,
            Err(_) => {
                mark_delivery(
                    pool,
                    &delivery.id,
                    "failed_exhausted",
                    0,
                    None,
                    Some("destination not found"),
                )
                .await;
                return;
            }
        };
    if destination.status != "enabled" {
        mark_delivery(
            pool,
            &delivery.id,
            "failed_exhausted",
            0,
            None,
            Some("destination disabled"),
        )
        .await;
        return;
    }
    let rendered = if event.is_v2() {
        event.to_v2_thin()
    } else {
        let mut json = event.to_json(crate::events::pending_webhooks(pool, &event.id).await);
        // A snapshot destination can pin the API version its events are rendered with.
        if let (Some(version), Some(obj)) =
            (&destination.snapshot_api_version, json.as_object_mut())
        {
            obj.insert("api_version".into(), serde_json::json!(version));
        }
        json
    };
    let payload = serde_json::to_string(&rendered).unwrap_or_else(|_| "{}".to_string());
    let (url, secret) = (destination.url, destination.secret);

    let mut last_status: Option<i64> = None;
    let mut last_error: Option<String> = None;

    for attempt in 1..=max_attempts {
        let timestamp = time::now_epoch();
        let signature = sign_payload(&secret, timestamp, &payload);
        let result = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Stripe-Signature", signature)
            .header("X-Stripe-Mock-Delivery-Id", &delivery.id)
            .header("X-Stripe-Mock-Event-Id", &event.id)
            .body(payload.clone())
            .timeout(Duration::from_secs(5))
            .send()
            .await;

        match result {
            Ok(response) => {
                let status = response.status().as_u16() as i64;
                let ok = (200..300).contains(&status);
                record_attempt(
                    pool,
                    &delivery.id,
                    attempt,
                    if ok { "succeeded" } else { "failed" },
                    Some(status),
                    None,
                )
                .await;
                if ok {
                    mark_delivery(
                        pool,
                        &delivery.id,
                        "succeeded",
                        attempt as i64,
                        Some(status),
                        None,
                    )
                    .await;
                    return;
                }
                last_status = Some(status);
                last_error = Some(format!("HTTP {status}"));
            }
            Err(err) => {
                record_attempt(
                    pool,
                    &delivery.id,
                    attempt,
                    "failed",
                    None,
                    Some(&err.to_string()),
                )
                .await;
                last_error = Some(err.to_string());
            }
        }

        if attempt < max_attempts {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    mark_delivery(
        pool,
        &delivery.id,
        "failed_exhausted",
        max_attempts as i64,
        last_status,
        last_error.as_deref(),
    )
    .await;
}

async fn record_attempt(
    pool: &SqlitePool,
    delivery_id: &str,
    attempt: u32,
    status: &str,
    http_status: Option<i64>,
    error: Option<&str>,
) {
    let _ = sqlx::query(
        "INSERT INTO delivery_attempts (delivery_id, attempt, status, http_status, error, created) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(delivery_id)
    .bind(attempt as i64)
    .bind(status)
    .bind(http_status)
    .bind(error)
    .bind(time::now_epoch())
    .execute(pool)
    .await;
}

async fn mark_delivery(
    pool: &SqlitePool,
    delivery_id: &str,
    status: &str,
    attempts: i64,
    http_status: Option<i64>,
    last_error: Option<&str>,
) {
    let _ = sqlx::query(
        "UPDATE deliveries SET status = ?1, attempts = ?2, http_status = ?3, last_error = ?4, updated = ?5 WHERE id = ?6",
    )
    .bind(status)
    .bind(attempts)
    .bind(http_status)
    .bind(last_error)
    .bind(time::now_epoch())
    .bind(delivery_id)
    .execute(pool)
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_verifiable() {
        let secret = "whsec_testsecret";
        let payload = r#"{"id":"evt_1"}"#;
        let signature = sign_payload(secret, 1_760_000_000, payload);
        assert!(signature.starts_with("t=1760000000,v1="));
        // Recompute independently.
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("1760000000.{payload}").as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());
        assert!(signature.ends_with(&expected));
    }
}
