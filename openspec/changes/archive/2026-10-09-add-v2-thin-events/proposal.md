# Proposal

## Why

`build-stripe-mock` deliberately deferred **v2 thin events and dynamic event destinations** (see its proposal "Out of scope for MVP"). Consumers of the mock that create Accounts through the v2 API (`POST /v2/core/accounts`) need the same follow-up signal real Stripe emits: a `v2.core.account.created` thin event delivered to a configured event destination. Today the mock emits only v1 snapshot events to config-file destinations, so v2 account onboarding cannot be driven end-to-end by an event consumer.

## What Changes

- Add an **event destination** resource (`ed_test_` id, `v2.core.event_destination`) created and managed at runtime via the v2 API: create, retrieve, list, update, delete, enable, disable, ping. Destinations of `type = "webhook_endpoint"` carry a URL and a generated `whsec_` signing secret.
- Add **thin event emission**: a v2 event row (`evt_test_`, `version = "v2"`) and its delivery records are written in the same transaction as the state change, and delivered to matching enabled webhook event destinations by `enabled_events` (exact names, `prefix.*`, or `*`).
- Emit `v2.core.account.created` whenever an account is created through `POST /v2/core/accounts`.
- Deliver thin events with the unversioned `v2.core.event` payload (no `data`/`changes` envelope), HMAC-signed with the destination's secret, reusing the existing per-destination FIFO queue, 200 ms spacing, and retry journal.
- Add v2 event read endpoints: `GET /v2/core/events` and `GET /v2/core/events/{id}` returning the full (versioned) `v2.core.event`, including `data`, `reason`, and `changes`/`context` when present.
- Expose event destinations through the internal control plane so their signing secrets can be retrieved for verification (`/internal/destinations`).

**Out of scope for this change**: Amazon EventBridge and Azure Event Grid transports (only `webhook_endpoint` is implemented); v2 event types other than the account lifecycle events emitted here; `events_from` organization scoping beyond `@self`; and `snapshot` payloads (added later by `unify-event-destinations`, which also removed the `[[webhook_targets]]` config concept in favor of a single event-destination model).

## Capabilities

### New Capabilities

- `v2-thin-events`: runtime event destinations (create/read/update/delete/enable/disable/ping), thin `v2.core.event` emission on the v2 account lifecycle, destination-based event matching, signed thin delivery, and the v2 events read API.

### Modified Capabilities

- None. The existing v1 event behavior in `webhook-dispatch` is unchanged; v2 thin events are delivered to event destinations. (The follow-up `unify-event-destinations` change removed the separate `webhook_targets` config/table so all webhook receivers are event destinations.)

## Impact

- New `event_destinations` table and additive schema changes to `events` (v2 rows reuse the existing `version` discriminator; the `data` column stores the v2 event envelope).
- `src/events.rs` gains `emit_thin` and a shared event-filter matcher; `src/dispatch.rs` renders the thin payload for `version = "v2"` events and resolves destinations; `src/models.rs` gains `EventDestinationRow` plus v2 event serialization.
- `src/api/v2.rs` and `src/lib.rs` gain the event-destination and v2-events routes; `src/internal.rs` gains destination secret exposure.
- `src/api/v2.rs::create_account` becomes transactional so the account row and its `v2.core.account.created` event commit atomically.
- Documentation updates: `docs/webhooks.md` (thin events vs v1 targets) and `README.md` (how to create a destination and verify a thin delivery), plus OpenSpec artifacts.
