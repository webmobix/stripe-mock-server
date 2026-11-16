# Tasks

## 1. Schema and models

- [x] 1.1 Add a `test_clocks` table and nullable `test_clock` columns on `customers`, `subscriptions`, and `invoices` in `src/db.rs`, add `test_clocks` to `reset_state`, and verify `cargo test db::` passes and a migrated pre-existing database still opens.
- [x] 1.2 Add `TestClockRow` and `test_clock` fields to `CustomerRow`, `SubscriptionRow`, and `InvoiceRow` in `src/models.rs`, render `test_clock` on each object, and verify `cargo test models::` covers the rendered field and the null default.
- [x] 1.3 Add `ids::test_clock_id()` with the `clock_` prefix and resolve `test_clock` in `src/api/expand.rs`, and verify a unit test expands a `clock_` reference and leaves another account's clock as an id.

## 2. Test clock CRUD

- [x] 2.1 Add `src/api/test_clocks.rs` with create, retrieve, list, and delete handlers: require `frozen_time`, cap `name` at 300 characters, accept `customer`, set `status=ready` and `deletes_after=created+30 days`, paginate newest first, and emit created/deleted events; verify integration tests cover create, retrieve, list, delete, missing `frozen_time`, and an unknown clock.
- [x] 2.2 Register `/v1/test_helpers/test_clocks` and `/v1/test_helpers/test_clocks/{id}` (`GET`, `DELETE`) plus `/advance` in `src/lib.rs`, and verify the routes resolve with the expected methods.
- [x] 2.3 Write the create/retrieve/list/delete and object-shape sections of `docs/test-clocks.md` and link the page from the README documentation index; verify every curl example matches the tested request shapes.

## 3. Clock-scoped billing

- [x] 3.1 Make `POST /v1/customers` accept `test_clock` (and clock creation accept `customer`), validate it belongs to the account and is ready, reject a second clock, stamp `customer.created` with `frozen_time`, and verify integration tests for attach, unknown clock, and second-clock rejection.
- [x] 3.2 Make `POST /v1/subscriptions` inherit a clock customer's clock, render `test_clock`, use the frozen time for `created` and periods, and end the first period one price interval later; verify an integration test asserts the subscription and item period bounds and that non-clock subscriptions keep the existing 30-day default.
- [x] 3.3 Inherit the clock when checkout completion creates a subscription and its first invoice for a clock customer, and verify an integration test creates an embedded or hosted session for that customer and inspects the resulting subscription and invoice.
- [x] 3.4 Add an explicit `created_at` argument to `raise_subscription_invoice` and `events::emit_at`, pass each caller's `now`, and verify invoice `created`/`paid_at` and event timestamps equal the simulated time for clock paths and wall-clock for existing paths.

## 4. Clock advance

- [x] 4.1 Extract the per-subscription period loop from `internal::advance_subscription` into a shared billing helper that retries an open invoice, raises boundary invoices, updates item periods, and honors `cancel_at_period_end`; verify `tests/stateleo.rs` and `tests/invoicing.rs` still pass.
- [x] 4.2 Add `POST /v1/test_helpers/test_clocks/{id}/advance` with backwards-time, two-interval, and two-year limit validation, `advancing` response plus committed `ready` state, and created/advancing/ready events; verify integration tests cover a forward advance, rejection cases, and a retrieval that reports `ready`.
- [x] 4.3 Process every clock subscription in order: raise one paid `subscription_cycle` invoice per crossed boundary, bill metered usage for the ended period, recover an open invoice before renewing, and cancel at a period boundary; verify integration tests for one renewal, two missed periods, metered arrears, recovery, and `cancel_at_period_end`.

## 5. Clock deletion and reset

- [x] 5.1 Implement clock deletion as one transaction that cancels subscriptions, deletes customers and their payment methods, emits the subscription/customer/clock deletion events, and leaves invoices readable; verify an integration test asserts the canceled subscription, gone customer, readable invoice, and `not_found` clock.
- [x] 5.2 Verify `/internal/reset` clears clocks and their associations; add an integration test that resets after creating a clock and customer and asserts an empty clock list.

## 6. Documentation and SDK integration

- [x] 6.1 Finish `docs/test-clocks.md` with attach, advance, renewal, dunning/recovery, cancellation, deletion, and timestamp sections plus cross-links from `docs/billing.md` and `docs/internal.md`; verify the examples match the tested flows and the README index links resolve.
- [x] 6.2 Extend `scripts/sdk-e2e/e2e.js` to create a clock and clock customer, create a subscription, advance the clock, and assert the renewal invoice and `ready` status through `stripe.testHelpers.testClocks` and `stripe.customers.create({ test_clock: clock.id })`.

## 7. Validation

- [x] 7.1 Run `cargo test --locked` and confirm the full unit and integration suite passes, including `tests/test_clocks.rs`.
- [x] 7.2 Run `openspec validate --all --strict` and confirm the change and all specs validate.
- [x] 7.3 Run `scripts/sdk-e2e/run.sh` with Stripe Node SDK 23 and confirm it ends with `E2E_SDK_OK`.
