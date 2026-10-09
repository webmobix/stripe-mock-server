# Tasks

## 1. Schema and identifiers

- [x] 1.1 Add the `event_destinations` table to `src/db.rs` (`CREATE TABLE IF NOT EXISTS`) and include it in `db::reset_state`; verify migrations apply cleanly to a fresh database and re-run idempotently.
- [x] 1.2 Add `ids::event_destination_id()` (`ed_test_`) and `ids::thin_event_id()` (`evt_test_`) with unit tests asserting the prefixes.
- [x] 1.3 Add the `[[event_destinations]]` config section and seed destinations at boot (idempotent by name, secret-stable); verify a config parse test and a seeding test.

## 2. Event model and emission

- [x] 2.1 Add `EventDestinationRow` to `src/models.rs` with `enabled_events`, `status`, and `to_json(include_url, include_secret)` rendering the `v2.core.event_destination` shape.
- [x] 2.2 Add `EventRow::to_v2_thin()` and `EventRow::to_v2_full()` rendering the delivered thin body and the full fetched event respectively; verify unit tests assert the presence/absence of `data`/`changes`/`reason`/`context`.
- [x] 2.3 Implement `events::emit_thin` writing a `version = "v2"` event plus pending deliveries to matching enabled destinations in the caller's transaction; verify a test that only matching, enabled destinations get delivery rows.
- [x] 2.4 Refactor event-filter matching into a shared helper used by both v1 targets and v2 destinations; verify existing filtering tests still pass and add destination-matching tests.

## 3. Account lifecycle emission

- [x] 3.1 Make `POST /v2/core/accounts` transactional and emit `v2.core.account.created` with `related_object` for the new account in the same transaction; verify the event exists and is dispatched after the response.
- [x] 3.2 Confirm v1 snapshot events are not routed to v2 destinations and vice versa; verify with a test.
- [x] 3.3 Implement `POST /v2/core/accounts/{id}` (update) and emit `v2.core.account.updated` plus `v2.core.account[configuration.<section>].capability_status_updated` for newly requested capabilities; verify an update test.
- [x] 3.4 Emit thin account events on onboarding completion: `capability_status_updated` on KYC pass and `v2.core.account[requirements].updated` on failure, plus `v2.core.account.updated` in both cases; verify an onboarding test.

## 4. Event destination API

- [x] 4.1 Implement `POST /v2/core/event_destinations` (validation, `ed_test_` id, generated secret, include-gated secret/url) and `GET`/`POST`/`DELETE` by id; verify create/retrieve/update/delete tests.
- [x] 4.2 Implement `GET /v2/core/event_destinations` list with pagination and `include` handling; verify a list test.
- [x] 4.3 Implement `.../{id}/enable`, `.../{id}/disable`, and `.../{id}/ping`; verify status transitions and that `ping` enqueues a delivery.
- [x] 4.4 Register the new v2 routes in `src/lib.rs`.

## 5. v2 events read API

- [x] 5.1 Implement `GET /v2/core/events` with newest-first ordering, pagination, and `types`/`object_id` filters; verify a filtered list test.
- [x] 5.2 Implement `GET /v2/core/events/{id}` returning the full event and 404 otherwise; verify tests.

## 6. Delivery

- [x] 6.1 Render the thin v2 payload in `src/dispatch.rs::process_delivery` by branching on `event.version` and resolving the destination row; verify a delivery test asserts the thin body and a valid signature.
- [x] 6.2 Start dispatcher workers for existing event destinations on boot/reset; verify a destination created before boot still receives deliveries.

## 7. Internal control plane

- [x] 7.1 Add `GET /internal/destinations` exposing destinations with secrets and register the route; verify the secret is returned.

## 8. Documentation and verification

- [x] 8.1 Update `docs/webhooks.md` (thin events vs v1 targets, destination management, thin payload) and `README.md` (create a destination, create a v2 account, verify the thin webhook with `jq`); verify every documented command runs as written.
- [x] 8.2 Extend `tests/webhooks.rs` (or a new `tests/thin_events.rs`) with an end-to-end receiver that verifies a `v2.core.account.created` thin delivery signature.
- [x] 8.3 Run the full test suite, the SDK e2e script, and `openspec validate "add-v2-thin-events"`; verify everything is green.
