use serde_json::Value;
use sqlx::{Sqlite, Transaction};

use crate::ids;
use crate::models::EventDestinationRow;
use crate::time;

/// The API version echoed on emitted v1 events.
pub const API_VERSION: &str = "2026-09-30.endive";
/// Discriminator stored on every v1 event row.
pub const EVENT_VERSION: &str = "v1";
/// Discriminator stored on every v2 thin event row.
pub const EVENT_VERSION_V2: &str = "v2";
/// `event_payload` for destinations that receive v1 snapshot events.
pub const PAYLOAD_SNAPSHOT: &str = "snapshot";
/// `event_payload` for destinations that receive v2 thin events.
pub const PAYLOAD_THIN: &str = "thin";

/// Insert an event row and one pending delivery per destination chosen by
/// `select`, all in the caller's transaction.
async fn insert_event(
    tx: &mut Transaction<'_, Sqlite>,
    id: String,
    owner_account_id: &str,
    event_account: Option<&str>,
    event_type: &str,
    version: &str,
    data: &Value,
    select: impl Fn(&EventDestinationRow) -> bool,
) -> Result<String, sqlx::Error> {
    let created = time::now_epoch();
    let data = serde_json::to_string(data).unwrap_or_else(|_| "null".to_string());

    sqlx::query(
        "INSERT INTO events (id, account_id, event_account, type, created, api_version, version, data, request, livemode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, 0)",
    )
    .bind(&id)
    .bind(owner_account_id)
    .bind(event_account)
    .bind(event_type)
    .bind(created)
    .bind(API_VERSION)
    .bind(version)
    .bind(&data)
    .execute(&mut **tx)
    .await?;

    let destinations: Vec<EventDestinationRow> =
        sqlx::query_as("SELECT * FROM event_destinations WHERE status = 'enabled'")
            .fetch_all(&mut **tx)
            .await?;

    for destination in destinations.iter().filter(|d| select(d)) {
        sqlx::query(
            "INSERT INTO deliveries (id, event_id, destination_id, status, attempts, http_status, last_error, created, updated)
             VALUES (?1, ?2, ?3, 'pending', 0, NULL, NULL, ?4, ?4)",
        )
        .bind(ids::id("dl_"))
        .bind(&id)
        .bind(&destination.id)
        .bind(created)
        .execute(&mut **tx)
        .await?;
    }

    Ok(id)
}

/// Create a v1 snapshot event and its pending delivery rows in the caller's
/// transaction, so state change, event, and deliveries commit atomically.
pub async fn emit(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    is_connected: bool,
    event_type: &str,
    data_object: Value,
) -> Result<String, sqlx::Error> {
    let event_account = is_connected.then_some(account_id);
    insert_event(
        tx,
        ids::event_id(),
        account_id,
        event_account,
        event_type,
        EVENT_VERSION,
        &data_object,
        |d| destination_matches(d, account_id, event_account, PAYLOAD_SNAPSHOT, event_type),
    )
    .await
}

/// Shared allow-list matching for destinations.
///
/// Supports `*` (all), exact names, and `prefix.*` (matches `prefix.<rest>`).
pub fn event_filter_matches(filters: &[String], event_type: &str) -> bool {
    if filters.is_empty() {
        return false;
    }
    if filters.iter().any(|f| f == "*") {
        return true;
    }
    filters.iter().any(|f| match f.strip_suffix(".*") {
        Some(prefix) => {
            event_type.starts_with(prefix)
                && matches!(
                    event_type.as_bytes().get(prefix.len()),
                    Some(b'.') | Some(b'[')
                )
        }
        None => f == event_type,
    })
}

/// Decide whether an event destination receives an event.
///
/// A destination is a single payload format (`snapshot` or `thin`), listens to
/// specific accounts (`events_from`), and filters by event type
/// (`enabled_events`). `@self` covers events owned by the destination's account;
/// `@accounts` covers events emitted for a different (connected) account, which
/// matches Stripe's "Your account" vs "Connected accounts" scopes.
pub fn destination_matches(
    destination: &EventDestinationRow,
    event_account_id: &str,
    event_account: Option<&str>,
    event_payload: &str,
    event_type: &str,
) -> bool {
    if destination.status != "enabled" {
        return false;
    }
    if destination.event_payload != event_payload {
        return false;
    }
    if !event_filter_matches(&destination.enabled_events(), event_type) {
        return false;
    }
    let from = destination.events_from();
    let owns_event = destination.account_id == event_account_id;
    let self_scope = owns_event && from.iter().any(|f| f == "@self");
    let connected_scope =
        event_account.is_some() && !owns_event && from.iter().any(|f| f == "@accounts");
    self_scope || connected_scope
}

/// The `related_object` for a v2 account.
pub fn account_related_object(account_id: &str) -> Value {
    serde_json::json!({
        "id": account_id,
        "type": "v2.core.account",
        "url": format!("/v2/core/accounts/{account_id}"),
    })
}

/// The `related_object` for a v1 account, used by thin `v1.account.*` events.
pub fn account_v1_related_object(account_id: &str) -> Value {
    serde_json::json!({
        "id": account_id,
        "type": "account",
        "url": format!("/v1/accounts/{account_id}"),
    })
}

/// Build a thin event envelope (`related_object` plus the fetched-only fields).
pub fn thin_envelope(related_object: Value, data: Value, changes: Value) -> Value {
    serde_json::json!({
        "related_object": related_object,
        "data": data,
        "changes": changes,
        "context": Value::Null,
        "reason": Value::Null,
    })
}

/// Map a capability name to its configuration section using the stored config.
pub fn capability_section(configuration: &Value, capability: &str) -> String {
    configuration
        .as_object()
        .and_then(|sections| {
            sections.iter().find_map(|(section, value)| {
                value
                    .get("capabilities")
                    .and_then(|c| c.as_object())
                    .and_then(|caps| caps.contains_key(capability).then(|| section.clone()))
            })
        })
        .unwrap_or_else(|| "merchant".to_string())
}

/// Create a v2 thin event and its pending delivery rows in the caller's
/// transaction. `envelope` supplies `related_object` and the fetched-only
/// `data`/`changes`/`context`/`reason` fields (values may be `null`).
///
/// When `only_destination` is set, the event is delivered solely to that
/// destination (used by the `ping` endpoint), bypassing the `enabled_events`
/// and scope filters. The destination must still be enabled and thin, since a
/// snapshot destination cannot receive a thin payload.
pub async fn emit_thin(
    tx: &mut Transaction<'_, Sqlite>,
    owner_account_id: &str,
    event_account: Option<&str>,
    event_type: &str,
    envelope: Value,
    only_destination: Option<&str>,
) -> Result<String, sqlx::Error> {
    insert_event(
        tx,
        ids::thin_event_id(),
        owner_account_id,
        event_account,
        event_type,
        EVENT_VERSION_V2,
        &envelope,
        |d| match only_destination {
            Some(only) => d.id == only && d.event_payload == PAYLOAD_THIN,
            None => {
                destination_matches(d, owner_account_id, event_account, PAYLOAD_THIN, event_type)
            }
        },
    )
    .await
}

/// Number of deliveries for an event that have not yet settled.
pub async fn pending_webhooks(pool: &sqlx::SqlitePool, event_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM deliveries WHERE event_id = ?1 AND status = 'pending'")
        .bind(event_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// SQL predicate (bound to `?1` = key account, `?2` = 1 if the key is the
/// platform) for events a key may see: events it owns, plus, for the platform,
/// connected-scope events (owned by the connected account itself, e.g. the thin
/// `v1.account.updated`).
pub const EVENT_VISIBLE_SQL: &str =
    "(account_id = ?1 OR (?2 = 1 AND event_account IS NOT NULL AND event_account = account_id))";

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(
        status: &str,
        account_id: &str,
        events: &str,
        payload: &str,
        from: &str,
    ) -> EventDestinationRow {
        EventDestinationRow {
            id: "ed_test_1".into(),
            account_id: account_id.into(),
            name: "d".into(),
            description: None,
            url: "http://localhost".into(),
            enabled_events: events.into(),
            event_payload: payload.into(),
            events_from: Some(from.into()),
            status: status.into(),
            metadata: None,
            snapshot_api_version: None,
            secret: "whsec_x".into(),
            created: 0,
            updated: 0,
        }
    }

    #[test]
    fn filter_matching_supports_wildcards() {
        let filters = vec!["v2.core.account.*".to_string()];
        assert!(event_filter_matches(&filters, "v2.core.account.created"));
        assert!(!event_filter_matches(&filters, "v2.core.other.created"));
        assert!(event_filter_matches(&["*".to_string()], "anything"));
        assert!(event_filter_matches(&["a.b".to_string()], "a.b"));
        assert!(!event_filter_matches(&[], "a.b"));
    }

    #[test]
    fn snapshot_destination_receives_self_events() {
        let dest = destination(
            "enabled",
            "acct_owner",
            "[\"*\"]",
            PAYLOAD_SNAPSHOT,
            "[\"@self\"]",
        );
        // Platform-owned v1 event.
        assert!(destination_matches(
            &dest,
            "acct_owner",
            None,
            PAYLOAD_SNAPSHOT,
            "account.updated"
        ));
        // Same event rendered thin or sent to connected scope does not match.
        assert!(!destination_matches(
            &dest,
            "acct_owner",
            None,
            PAYLOAD_THIN,
            "account.updated"
        ));
        // A connected event does not match an @self-only destination.
        assert!(!destination_matches(
            &dest,
            "acct_conn",
            Some("acct_conn"),
            PAYLOAD_SNAPSHOT,
            "account.updated"
        ));
    }

    #[test]
    fn connected_destination_matches_connected_events() {
        let dest = destination(
            "enabled",
            "acct_owner",
            "[\"*\"]",
            PAYLOAD_SNAPSHOT,
            "[\"@accounts\"]",
        );
        assert!(destination_matches(
            &dest,
            "acct_conn",
            Some("acct_conn"),
            PAYLOAD_SNAPSHOT,
            "account.updated"
        ));
        assert!(!destination_matches(
            &dest,
            "acct_owner",
            None,
            PAYLOAD_SNAPSHOT,
            "account.updated"
        ));
    }

    #[test]
    fn thin_destination_receives_v2_account_events() {
        // v2 account events are platform ("Your account") scope even when they
        // concern a connected account, so @self receives them.
        let dest = destination(
            "enabled",
            "acct_owner",
            "[\"v2.core.account.*\"]",
            PAYLOAD_THIN,
            "[\"@self\"]",
        );
        assert!(destination_matches(
            &dest,
            "acct_owner",
            Some("acct_conn"),
            PAYLOAD_THIN,
            "v2.core.account.created"
        ));
        assert!(!destination_matches(
            &dest,
            "acct_owner",
            Some("acct_conn"),
            PAYLOAD_THIN,
            "v2.core.other.created"
        ));

        let connected_only = destination(
            "enabled",
            "acct_owner",
            "[\"*\"]",
            PAYLOAD_THIN,
            "[\"@accounts\"]",
        );
        assert!(!destination_matches(
            &connected_only,
            "acct_owner",
            Some("acct_conn"),
            PAYLOAD_THIN,
            "v2.core.account.created"
        ));
    }

    #[test]
    fn disabled_destination_is_skipped() {
        let disabled = destination(
            "disabled",
            "acct_owner",
            "[\"*\"]",
            PAYLOAD_THIN,
            "[\"@self\"]",
        );
        assert!(!destination_matches(
            &disabled,
            "acct_owner",
            None,
            PAYLOAD_THIN,
            "v2.core.account.created"
        ));
    }
}
