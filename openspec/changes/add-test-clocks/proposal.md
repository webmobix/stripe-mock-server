# Proposal

## Why

The mock can advance one subscription at a time through
`POST /internal/subscriptions/{id}/advance`, but the app's test suite drives
renewals, dunning, and cancellation with Stripe **test clocks**: create a
customer at a frozen time, then advance the clock once and let every
subscription renew, invoice, and transition together. That code cannot run
against the mock today because `/v1/test_helpers/test_clocks` does not exist.

## What Changes

- Add `POST`/`GET /v1/test_helpers/test_clocks`, `GET|DELETE
  /v1/test_helpers/test_clocks/{id}`, and
  `POST /v1/test_helpers/test_clocks/{id}/advance` with a
  `test_helpers.test_clock` object (`clock_` id, `frozen_time`, `status`,
  `deletes_after`, `name`, `status_details`).
- `POST /v1/customers` accepts `test_clock`; subscription and invoice objects
  inherit the customer's clock and expose `test_clock`.
- Clock-scoped customers, subscriptions, and invoices are stamped with the
  clock's `frozen_time` instead of wall-clock time.
- Advancing a clock synchronously processes every overdue period of every
  clock-scoped subscription in order: paid `subscription_cycle` invoices,
  metered usage billed in arrears, `cancel_at_period_end` honored, and a
  previously open invoice retried before the next renewal.
- Advance is bounded to two intervals of the shortest subscription (two years
  when the clock has no subscriptions), and backwards advances are rejected.
- Deleting a clock cancels its subscriptions and deletes its customers and
  saved payment methods; `/internal/reset` clears clocks.
- Keep `/internal/subscriptions/{id}/advance` for per-subscription paid/failed
  control; test clocks add the clock-wide path on top.

## Capabilities

### New Capabilities

- `test-clocks`: Stripe-compatible test-clock CRUD, frozen time, clock-wide
  advance, and clock-scoped billing behavior.

### Modified Capabilities

- `billing`: Customers accept a `test_clock`, and subscriptions inherit it and
  are created at its frozen time.

## Impact

- New `src/api/test_clocks.rs`; routes in `src/lib.rs`; `test_clocks` table and
  `test_clock` columns in `src/db.rs`; clock fields in `src/models.rs`;
  `src/api/v1.rs` customer/subscription/checkout paths; `src/api/billing.rs`
  invoice helpers parameterized by simulated time; `src/events.rs` timestamp
  support; `src/ids.rs`; `src/seed.rs`/reset.
- New integration coverage in `tests/test_clocks.rs`, plus Stripe Node SDK
  coverage in `scripts/sdk-e2e/e2e.js`.
- New `docs/test-clocks.md` with README/config cross-links.
- No new dependencies and no schema-breaking changes; existing rows default to
  no clock.
