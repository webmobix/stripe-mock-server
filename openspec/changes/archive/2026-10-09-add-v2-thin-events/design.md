# Design

## Context

The mock already models v1 events and webhook delivery:

- `events` rows carry a `version` discriminator (currently always `"v1"`), documented as a seam "so v2 thin events can be added later".
- `Dispatcher` runs one FIFO worker per destination, signs with the destination secret, and journals attempts.
- `EventRow::to_json` renders the v1 snapshot shape.

> **Superseded by `unify-event-destinations`.** The original `webhook_targets` config section, its `scope` field, and the separate `target_id` column were removed; everything now goes through a single `event_destinations` table and `deliveries.destination_id`. This change is unarchived, so the sections below are updated to the final state where they would otherwise contradict the code.

Real Stripe v2 event destinations differ from v1 webhook endpoints:

- They are created at runtime (`POST /v2/core/event_destinations`), not in the dashboard/config.
- `event_payload` is `thin` (unversioned notification) or `snapshot` (versioned full object).
- `type` is a transport enum (`webhook_endpoint`, `amazon_eventbridge`, `azure_event_grid`).
- The delivered thin body is only `created`, `id`, `livemode`, `object`, `type`, `related_object`; the fuller `data`/`reason`/`changes` payload is returned by `GET /v2/core/events/{id}`.

## Goals / Non-Goals

**Goals**

- Create/manage event destinations through the v2 API.
- Emit `v2.core.account.created` when an account is created via the v2 API, delivered to matching thin destinations.
- Reuse the existing delivery engine (FIFO, 200 ms spacing, 3 retries, journal) and signing.
- Keep v1 behavior byte-for-byte unchanged.

**Non-goals**

- Non-webhook transports (EventBridge/Azure).
- Org/multi-account `events_from` scoping beyond `@self` / `@accounts`.
- Event types beyond the account lifecycle points implemented here.

## Decisions

### D1: One `event_destinations` table (unified)

This change originally kept `event_destinations` separate from a `webhook_targets` table and chose the row from the event's `version`. `unify-event-destinations` removed `webhook_targets`; there is now a single `event_destinations` table and `deliveries` references `destination_id`. Dispatch still selects on the destination's `event_payload` (`snapshot` → v1 render, `thin` → v2 render) rather than on a separate config concept.

Schema:

```
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
```

Added to `db::reset_state`. The table is created with `CREATE TABLE IF NOT EXISTS`, so no migration of existing rows is needed (existing databases gain the new empty table on boot).

### D2: Reuse `events.version` and the `data` column for the v2 envelope

v2 event rows use `version = "v2"` and `id` prefixed `evt_test_`. The `data` column stores a JSON envelope:

```json
{ "related_object": {"id": "...", "type": "v2.core.account", "url": "..."},
  "data": null, "changes": null, "context": null, "reason": null }
```

`EventRow::to_v2_thin()` renders the delivered body (no envelope fields); `EventRow::to_v2_full()` renders `GET /v2/core/events/{id}`. `EventRow::to_json()` remains the v1 snapshot renderer and is only invoked for v1 rows.

### D3: `emit_thin` writes event + deliveries atomically, dispatch after commit

`events::emit_thin(tx, owner_account_id, event_type, envelope)` follows the same contract as `events::emit`: insert the event and its pending `deliveries`, and let the caller commit before `dispatcher.dispatch_event(event_id)`. Matching selects `status = 'enabled'` destinations for the owning account whose `enabled_events` match the type (`*`, exact, or `prefix.*`).

### D4: Destination payload rendering

`process_delivery` loads the event and the destination, then renders `event.to_v2_thin()` when the event row is v2 and `event.to_json()` when it is v1. The destination's `event_payload` guarantees the two agree. Signing is unchanged (`sign_payload` over the serialized body) and the `X-Stripe-Mock-Delivery-Id` / `X-Stripe-Mock-Event-Id` headers are preserved.

### D5: Destination API shape

Implement only `type = "webhook_endpoint"`:

- `POST /v2/core/event_destinations` requires `name`, `enabled_events`, `event_payload`, `type`, and `webhook_endpoint.url`. `event_payload` may be `thin` or `snapshot` (unified by `unify-event-destinations`). Generates `ed_test_` id and a `whsec_` secret keyed by endpoint URL (destinations on the same URL share it). The secret is returned only when `include` contains `webhook_endpoint.signing_secret` (Stripe contract); `webhook_endpoint.url` is returned when `include` contains `webhook_endpoint.url`.
- `GET`/`POST`/`DELETE /v2/core/event_destinations/{id}`, and `POST .../{id}/enable|disable|ping`.
- `ping` enqueues a synthetic delivery to the destination so integrators can verify connectivity.
- Unsupported transports or an unknown `event_payload` return the v2 `invalid_request_error` envelope.

`events_from` defaults to `["@self"]`; `status_details` is `null` while enabled and `{"disabled": {"reason": "user"}}` after `disable`.

### D6: v2 events read API

- `GET /v2/core/events` lists v2 rows newest-first with v2 pagination (`limit`, `starting_after`/`page`), optional `types` filter, and `object_id` filter against the stored `related_object.id`.
- `GET /v2/core/events/{id}` returns `to_v2_full()`; 404 for unknown or v1 ids.

### D7: Internal exposure

`GET /internal/destinations` lists event destinations with secrets, so a receiver can obtain the signing secret after creation (Stripe returns it only at create time). After `unify-event-destinations` this is the only webhook surface; `/internal/targets` was removed.

## Risks / Trade-offs

- **Two delivery target sources**: replaced by a single destination table; dispatch renders by event version. Tested for both snapshot and thin paths.
- **Real Stripe redacts destination URL/secret after create**: the mock's internal endpoint intentionally diverges to keep local verification ergonomic; documented as such.
- **Prefix matching for bracketed v2 types**: `prefix.*` matches `v2.core.account.*` → `v2.core.account.created` but not `v2.core.account[configuration...].updated` (next char is `[`). Integrators use exact names or `*` for those, matching the existing target semantics.

## Migration Plan

1. Add the table in `db.rs` (idempotent) and to `reset_state`.
2. Add IDs, model row, emission, dispatch branch, routes, and internal endpoint.
3. Update docs and OpenSpec tasks/specs.
4. Verify: unit + integration tests, the SDK e2e script, and `openspec validate "add-v2-thin-events"`.

No data migration is required; existing databases gain an empty table.

## Open Questions

- None blocking. Non-webhook transports remain deferred to a future change; `snapshot` payloads were added by `unify-event-destinations`.
