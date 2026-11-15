# Design

## Context

The mock is a single Rust process backed by SQLite. v1 resources are stored in
typed tables with JSON columns for nested objects; state changes and their events
commit in one transaction and deliveries are dispatched afterwards. This change
extends that model rather than introducing a new one.

The consuming app uses Stripe Node SDK 23 and the newer API vocabulary:
`embedded_page` rather than `embedded`, `billing.meter`/`billing.meter_event`,
`billing_scheme=tiered`, and `invoices.createPreview`.

## Goals / Non-Goals

**Goals:**

- Make usage reporting, plan pricing, embedded purchase/activation, renewal
  failure/recovery, upcoming invoices, card display, and idempotent retries
  testable end to end with SDK 23.
- Keep new state in SQLite so summaries, invoices, and cards survive retrieval
  and are isolated per account.
- Make test reset deterministic: configured keys and signing secrets survive
  `/internal/reset`.
- Preserve existing behavior for the flows already covered by the SDK e2e.

**Non-Goals:**

- Exact Stripe Tax, invoice item, discount/proration, or schedule behavior.
- Decimal price amounts for the base `unit_amount` (tier decimals are
  supported).
- A full usage-record pipeline or asynchronous meter-event validation.
- Idempotency for v2 endpoints.
- Concurrent idempotent request coalescing; the mock caches after a handler
  runs and is intended for sequential tests.

## Decisions

### Meters store events, summaries aggregate on read

`billing_meters` stores configuration; `meter_events` stores one row per event
with `(account_id, identifier)` unique. Summaries query events for a meter and
customer over a window and aggregate in Rust. This keeps duplicate detection an
indexed lookup and lets summaries reflect events reported earlier in the same
test without a background worker.

Only one active meter may use an event name. Deactivation records
`deactivated_at`; price creation and event reporting both require an active
meter.

### Meter event identity and errors

The v1 `billing.meter_event` object has no exposed `id`; the caller's
`identifier` is the key. A generated `idmp_` identifier is used when omitted. A
duplicate returns HTTP 400 with `invalid_request_error` and code
`duplicate_meter_event`, matching Stripe's retry contract. An unknown event name
returns code `no_meter`.

### Tiered amounts are shared domain logic

`src/api/billing.rs` owns `amount_for_quantity`:
`per_unit` multiplies the unit amount; `graduated` bills each band's units;
`volume` bills the whole quantity at the containing band. `flat_amount` is added
per used band. Checkout totals, subscription first invoices, renewals, and
previews all call this function, so pricing cannot drift between endpoints.

Tiers are stored normalized in the price's `tiers` JSON. The final tier must be
`inf`; bounds must increase. A tiered price reports `unit_amount=null`.

### Metered lines have no quantity

For a metered price the quantity is derived from meter events, never from the
request. Subscription creation, subscription update, checkout creation, and
preview overrides reject a supplied quantity. Subscription items store
`quantity: null`.

### First invoices are paid; renewals can fail

Creating a subscription raises a paid `subscription_create` invoice. This makes
the initial purchase and activation visible without a second control-plane call.
`POST /internal/subscriptions/{id}/advance` advances the period using the item
interval and raises a `subscription_cycle` invoice. A paid outcome marks it paid
and the subscription `active`; failed leaves it `open`, increments
`attempt_count`, and sets `past_due`.

Metered usage is billed in arrears: licensed lines use the new period, metered
lines use the period that just ended. If an open invoice exists, `advance`
retries that invoice instead of raising another, so `failed` then `paid` is
recovery on the same invoice.

### Embedded checkout is the same session with a client secret

`ui_mode` is stored on the session. Hosted sessions keep `url` and report
`client_secret=null`; embedded/elements sessions require `return_url`, store a
`cs_test_..._secret_...` client secret, and report `url=null`. Completion uses
the existing transactional path and redirects to `return_url` for embedded
sessions. `subscription_data.metadata` is stored as JSON and applied when the
subscription is created.

### A card is created once per customer

Checkout completion calls `attach_default_card`, which reuses an existing saved
card or creates a Visa `pm_` ending `4242`, then writes
`invoice_settings.default_payment_method`. Payment methods are a first-class
table with retrieve, customer-list, customer-retrieve, and
`expand[]=invoice_settings.default_payment_method` support.

### Configured secrets are re-seeded

`ensure_keys` replaces generated keys of a kind with a configured value.
`normalize_destination_secrets` prefers `webhook_endpoint.signing_secret` for a
URL and otherwise derives a stable value from the URL. Reset deletes state and
calls the same seeding path, so configured values survive while generated ones
may rotate.

### Preview builds an invoice object without writing it

`create_preview` resolves the customer and subscription (or a customer's most
recent live subscription), prices licensed lines for the next period and
metered lines over the current period, and returns an `upcoming_in_` draft. It
does not insert a row, emit events, or appear in list results.

### Idempotency is response caching middleware

A v1 middleware buffers the request body and keys the cache by idempotency key,
hashed credential, method, and path. A matching body replays the stored status,
body, and content type with `Idempotent-Replay: true`; a different body returns
`idempotency_error` before the handler runs. Responses below 500 are cached.
Reset clears the table.

## Risks / Trade-offs

- **Metered renewal timing.** Stripe's mixed-period billing is approximated:
  licensed lines use the new period and metered lines the ended period. This
  matches the common test assertion (usage billed at renewal) but not every
  proration nuance.
- **Paid first invoice.** Real Stripe may leave the first invoice open depending
  on payment behavior. The mock chooses paid so activation is deterministic;
  renewal failure is exercised through `advance`.
- **Zero-total failed renewals.** A metered-only period with no usage can raise a
  zero-total invoice even on `failed`. Tests for failure should use a licensed
  component.
- **Idempotency races.** Concurrent requests with the same key can both run
  before either caches a response. This is acceptable for sequential test
  suites.
- **Configured key collisions.** Inserting a configured key that already belongs
  to another account would leave this account without that key; configuration is
  assumed to be unique.

## Migration Plan

`db::migrate` adds the new columns to existing `prices`, `checkout_sessions`,
`customers`, and `invoices` tables and creates the meter, payment-method, and
idempotency tables. Existing rows default to licensed/per-unit, hosted checkout,
`send_invoice`, and no saved card. No data rewrite is required.
