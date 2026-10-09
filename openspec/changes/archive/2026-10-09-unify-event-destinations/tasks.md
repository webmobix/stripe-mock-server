# Tasks

## 1. Remove the v1 target model

- [x] 1.1 Remove `[[webhook_targets]]` from the config model and validation; replace with `[[event_destinations]]` fields (`name`, `description`, `type`, `event_payload`, `events_from`, `enabled_events`, `enabled`, `webhook_endpoint.url`); add config tests for parsing, invalid payload, and missing URL.
- [x] 1.2 Drop the `webhook_targets` table and `wht_` ids; rename `deliveries.target_id` to `destination_id`; remove `TargetRow` and `ids::target_id`.
- [x] 1.3 Rewrite seeding to seed only event destinations (idempotent by name, secret-stable); update the persistence test to `event_destinations`.

## 2. Unify matching, dispatch, and rendering

- [x] 2.1 Rewrite `destination_matches` to filter by `event_payload`, `events_from` scope, and `enabled_events`; unit-test self/connected/thin/snapshot/disabled cases.
- [x] 2.2 Route v1 `emit` to snapshot destinations and `emit_thin` to thin destinations; update dispatch to resolve the destination and render by event version.
- [x] 2.3 Remove `/internal/targets` and `internal::list_targets`; expose secrets only at `/internal/destinations`.

## 3. v2 Account dual emission

- [x] 3.1 Add `events::account_v1_related_object` and emit thin `v1.account.updated` (connected scope) alongside snapshot `account.updated` on `POST /v2/core/accounts/{id}`.
- [x] 3.2 Emit thin `v1.account.updated` on onboarding completion next to the existing snapshot `account.updated`.
- [x] 3.3 Accept `event_payload = "snapshot" | "thin"` in the v2 create API.
- [x] 3.4 Add an integration test asserting the three forms and the `@self`/`@accounts` routing.

## 4. Docs and validation

- [x] 4.1 Update `config.toml`, `docs/configuration.md`, `docs/webhooks.md`, `docs/internal.md`, and `README.md` to the single event-destination model.
- [x] 4.2 Update the SDK e2e harness (`run.sh`, `e2e.js`) to snapshot destinations and `/internal/destinations`.
- [x] 4.3 Reconcile `build-stripe-mock` and `add-v2-thin-events` artifacts with the unified model.
- [x] 4.4 `cargo test` (65 tests), `openspec validate` for all three changes, and `scripts/sdk-e2e/run.sh` (`E2E_SDK_OK`).
