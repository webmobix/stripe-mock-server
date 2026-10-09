# Unify webhook configuration around event destinations

## Why

The mock models webhooks with **two unrelated configuration concepts**:

- `[[webhook_targets]]` — v1 snapshot endpoints with a `scope` of
  `platform | connected | all` and a per-account allow-list.
- `[[event_destinations]]` — v2 thin endpoints.

Stripe has exactly one concept: the **event destination**
(`v2.core.event_destination`). You create a destination, choose
`event_payload` (`snapshot` or `thin`), `events_from` (`@self` / `@accounts`),
`enabled_events`, and a transport (`webhook_endpoint.url`). To receive both
formats you register **two** destinations — Stripe states this explicitly:
*"If you are using thin events, you will need to register a separate webhook
endpoint."* The `webhook_targets` split is an artifact of the mock being built
in stages; it does not exist in Stripe and makes the config more confusing than
the product it emulates.

## What Changes

- **Remove `[[webhook_targets]]`** (config section and `webhook_targets` table)
  and the `scope` / account allow-list model. **Breaking, no compatibility
  shim** — Stripe is the spec.
- Make `[[event_destinations]]` the **single webhook concept**, mirroring the
  Stripe object: `name`, `description`, `type`, `event_payload`, `events_from`,
  `enabled_events`, `enabled`, and `webhook_endpoint.url`.
- A **snapshot** destination receives v1 `event` objects; a **thin**
  destination receives `v2.core.event` notifications.
- Replace `scope` with **`events_from`** (`@self` = "Your account",
  `@accounts` = "Connected accounts").
- Emit **both event families** for a v2 Account change, as Stripe does: a v1
  snapshot `account.updated` and thin `v1.account.updated` (Connected accounts
  scope) alongside the v2 thin events (Your account scope).
- Accept `event_payload = "snapshot" | "thin"` on
  `POST /v2/core/event_destinations`.
- Remove `/internal/targets`; deliveries reference a destination.

## Impact

- Affected capabilities: `server-configuration`, `webhook-dispatch`,
  `internal-control-plane`, and the `v2-thin-events` change.
- Existing `[[webhook_targets]]` configurations must migrate to
  `[[event_destinations]]`; the `webhook_targets` table and `wht_` ids are gone.
