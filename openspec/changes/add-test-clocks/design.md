# Design

## Context

See `proposal.md` for motivation and `specs/test-clocks/spec.md` for the
behavior contract. The mock stores state in SQLite, commits state changes and
their events in one transaction, and dispatches deliveries afterwards. Billing
periods already advance through `POST /internal/subscriptions/{id}/advance`,
which uses `advance_period`, `open_subscription_invoice`, and
`raise_subscription_invoice` from `src/api/billing.rs`. All timestamps currently
come from `time::now_epoch()`.

The consuming app uses Stripe Node SDK 23 test clocks: create a clock and a
customer on it, create subscriptions, advance the clock, then assert renewals,
invoices, dunning, and cancellation.

## Goals / Non-Goals

**Goals:**

- Support the Stripe test-clock resource and lifecycle with observable,
  deterministic behavior.
- Scope simulated time to clock customers, their subscriptions, and their
  invoices without changing wall-clock behavior for every other object.
- Reuse the existing period/invoice machinery so clock renewals cannot drift
  from `/internal/.../advance`.
- Keep advance atomic and synchronous so tests do not need a background worker.

**Non-Goals:**

- Trial periods, proration on advance, quotes, or subscription schedules.
- A background clock worker, `internal_failure` simulation, or automatic
  `deletes_after` cleanup.
- Clocks for connected-account (`customer_account`) subscriptions.
- Clock-aware timestamps on objects outside the customer/subscription/invoice
  chain, including checkout sessions themselves.

## Decisions

### Store the clock on each scoped object

`test_clocks` holds `id`, `account_id`, `name`, `frozen_time`, `status`,
`created`, and `deletes_after`. `customers`, `subscriptions`, and `invoices`
gain a nullable `test_clock` column. Storing it on all three (instead of only
on the customer) lets each object render and expand `test_clock` without joins
and keeps invoices readable after their customer is deleted.

A customer belongs to at most one clock. Attaching is allowed through
`POST /v1/customers` (`test_clock`) or `POST /v1/test_helpers/test_clocks`
(`customer`); both paths validate the clock belongs to the resolved account and
is `ready`. The association is never moved.

### Pass simulated time explicitly

Do not replace `time::now_epoch()` globally: handlers run concurrently and many
paths (auth, idempotency, deliveries, v2) should keep wall-clock time. Instead,
clock-aware paths resolve `now` from the attached clock and pass it into the
billing helpers. This means:

- `create_customer` uses `clock.frozen_time` for `created` and the
  `customer.created` event.
- `create_subscription` uses the clock's frozen time for `created`, the first
  period, item `created`, and the first invoice; the first period ends one
  price interval later (via `subscription_interval`/`advance_period`) rather
  than the shared 30-day default, which stays unchanged for non-clock
  subscriptions.
- Checkout completion for a clock customer stamps the created subscription,
  item periods, and first invoice with the frozen time; the session itself
  stays wall-clock (non-goal).
- `raise_subscription_invoice` gains a `created_at` argument used for
  `created`, `paid_at`, and the invoice event timestamp. Existing callers pass
  the request's `now`; the clock advance passes each period boundary.

### Emit events at simulated time

`events::emit` keeps using wall-clock time. Add `events::emit_at` (same
behavior plus an explicit `created`) and have clock-scoped paths use it.
`insert_event` gains the timestamp parameter; existing wrappers pass
`time::now_epoch()`. Deliveries created for those events use the same timestamp.

### Share period advancement with the internal endpoint

Extract the per-subscription loop from `internal::advance_subscription` into a
billing helper that, given a subscription, a target time, and an outcome:

- retries an open invoice first (paid for clocks, per the configured outcome
  for `/internal/.../advance`);
- raises `subscription_cycle` invoices at each period boundary at or before the
  target, billing metered usage over the ended period;
- honors `cancel_at_period_end` by canceling at the boundary and emitting
  `customer.subscription.deleted` instead of renewing;
- updates subscription and item period fields and emits
  `customer.subscription.updated`.

The internal endpoint calls it once with its outcome. The clock advance calls it
for every subscription attached to the clock, ordered by period end then id, so
invoice and event order is deterministic.

### Advance is synchronous but reports `advancing`

Stripe returns `status=advancing` and later flips to `ready`. The mock performs
all work inside one transaction, then commits `status=ready` and the new
`frozen_time` before responding. The response body is built with
`status=advancing` and `status_details.advancing.target_frozen_time`; a
subsequent retrieval shows `ready`. This satisfies clients that assert the
immediate `advancing` response and clients that poll for `ready`, without a
worker. `test_helpers.test_clock.advancing` is emitted at the old frozen time
and `.ready` at the target inside the same transaction.

### Advance limits use the shortest interval

The target must be strictly later than `frozen_time` (error param
`frozen_time`). When subscriptions exist, the maximum is two intervals of the
shortest subscription interval, measured from the current frozen time and using
the same day approximations as `advance_period` (month = 30 days, year = 365
days). With no subscriptions the maximum is two years. The cap keeps a single
advance from generating an unbounded invoice run.

### Deleting a clock cancels, then deletes

Within one transaction: cancel each active/past-due subscription (set
`canceled`, emit `customer.subscription.deleted`), delete its payment methods
and customer (emit `customer.deleted`), delete the clock, and emit
`test_helpers.test_clock.deleted`. Invoices remain readable with their
`test_clock` set. Resetting state clears `test_clocks` like any other table.

## Risks / Trade-offs

- **Approximate intervals.** Months are 30 days and years 365, matching the
  existing `advance_period`; date-exact Stripe renewals cannot be reproduced.
  Tests should assert period ends using the same arithmetic.
- **Many missed periods.** Two intervals of catch-up bounds the work, but a
  short interval (day/week) can still create several invoices in one call.
  Mitigation: process inside one transaction and keep the limit.
- **Clock-wide atomicity.** A failure in any subscription rolls back the whole
  advance, unlike Stripe's per-object progression. Acceptable for test
  determinism and simpler than partial advancement.
- **Checkout timestamps.** A subscription created through checkout inherits the
  clock, but the session itself is wall-clock. Tests asserting session
  timestamps under a clock would diverge from Stripe; the app's flow does not
  rely on that.
- **Deleted invoices' customer.** Deleting a clock removes customers while
  leaving invoices readable; those invoices reference a deleted `cus_` id. This
  mirrors a financial record surviving customer deletion and keeps assertions
  on the invoice possible.

## Migration Plan

`db::migrate` creates `test_clocks` and adds nullable `test_clock` columns to
`customers`, `subscriptions`, and `invoices`, so existing rows read as no
clock. `/internal/reset` adds `test_clocks` to its delete list. No data rewrite
is required. Rollback is a schema-compatible downgrade: drop the table/columns
or leave them unused.
