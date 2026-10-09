# Configuration reference

The server reads a TOML file at `STRIPE_MOCK_CONFIG` (default `config.toml`).
The committed [`config.toml`](../config.toml) is a valid sample.

## `[server]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `bind` | string | `"127.0.0.1"` | Interface the server (including `/internal/**`) binds to. |
| `port` | integer | required | TCP port. |
| `public_base_url` | string | required | Base URL used when minting onboarding/checkout URLs and redirecting. |

## `[database]`

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `path` | string | `"stripe-mock.db"` | SQLite file. Created if missing. |
| `reset_on_start` | bool | `false` | When true, clear all state and reseed before serving. |

## `[main_account]`

Describes the platform account created on first boot. All fields optional.

| Field | Type | Description |
|-------|------|-------------|
| `business_type` | string | e.g. `company`, `individual`. |
| `country` | string | Two-letter country code. |
| `email` | string | Account email. |
| `name` | string | Business name used in the v1/v2 identity. |

## `[[event_destinations]]`

Zero or more event destinations — the single webhook concept, mirroring Stripe's
`v2.core.event_destination`. A `whsec_` signing secret is generated **per endpoint
URL** (not per destination) and persisted so it is stable across restarts.
Destinations that share a `webhook_endpoint.url` — for example a snapshot and a
thin destination pointed at the same route — report the **same** secret, so one
receiver verifies both with a single `whsec_`. (This is a deliberate mock
convenience: real Stripe issues a distinct secret per destination.) Declaring the
same `name` on restart updates the existing destination in place.

Each destination has exactly one payload format. Register **two** destinations to
receive both formats, exactly as Stripe requires a separate endpoint per format.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | required | Human name (unique per account and `event_payload`; a snapshot and a thin destination may share a name). |
| `event_payload` | `"snapshot"` \| `"thin"` | required | `snapshot` receives v1 `event` objects; `thin` receives `v2.core.event` notifications. |
| `events_from` | array | `["@self"]` | `"@self"` = this account; `"@accounts"` = accounts you manage (connected). |
| `enabled_events` | array of strings | `["*"]` | `*`, exact names, or `"prefix.*"` patterns. |
| `webhook_endpoint.url` | string | required | Receiver URL. |
| `enabled` | bool | `true` | Maps to destination `status` (`enabled`/`disabled`). |
| `description` | string | unset | Optional user-defined description. |
| `type` | string | `"webhook_endpoint"` | Only `webhook_endpoint` is supported. |

### Example

```toml
# v1 snapshot events (Your account + Connected accounts)
[[event_destinations]]
name = "snapshot-app"
event_payload = "snapshot"
events_from = ["@self", "@accounts"]
enabled_events = ["*"]
webhook_endpoint = { url = "http://localhost:4000/stripe/webhook" }

# v2 thin events (Your account)
[[event_destinations]]
name = "thin-app"
event_payload = "thin"
events_from = ["@self"]
enabled_events = ["v2.core.account.*"]
webhook_endpoint = { url = "http://localhost:4002/stripe/thin" }
```

### Scopes

`events_from` replaces a separate `scope` concept and matches Stripe's
"Your account" / "Connected accounts" split:

- `@self` — events owned by the destination's account ("Your account").
- `@accounts` — events emitted for connected accounts ("Connected accounts").

For a v2 Account, v1 events (`account.updated` and thin `v1.account.updated`) use
the **Connected accounts** scope, while v2 events (`v2.core.account.*`) use the
**Your account** scope. `events_from` is a list, so include both if one
destination should cover both.

### Payload format is not the same as event family

`event_payload` (the delivered payload shape) and the event family (v1 vs v2)
are independent axes:

- A **thin** destination can receive both v1-derived and v2-derived event types.
  For example, one thin destination with `events_from = ["@self", "@accounts"]`
  and `enabled_events = ["*"]` receives `v1.account.updated` **and**
  `v2.core.account.*`.
- A **snapshot** destination likewise receives snapshot events regardless of
  whether the underlying resource is v1 or v2.

The only thing a single destination cannot do is deliver the *same* event in
*both* payload formats. For that, add a second destination — Stripe calls this
the **dual-destination strategy** — and point both at the same URL if you like.
So: `enabled_events` / `events_from` decide **what** reaches a destination;
`event_payload` decides **how** it is shaped.

## Environment

| Variable | Default | Description |
|----------|---------|-------------|
| `STRIPE_MOCK_CONFIG` | `config.toml` | Path to the TOML config. |
| `RUST_LOG` | `stripe_mock=info` | Tracing filter; request lines are logged at `info`. |
