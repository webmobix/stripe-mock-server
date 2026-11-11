# Tasks

## 1. Schema, migrations, and identifiers

- [x] 1.1 In `src/db.rs`, add `coupons`, `tax_rates`, and `billing_portal_sessions` tables to `SCHEMA`, and add `prices.lookup_key`, `checkout_sessions.customer_email`, `amount_subtotal`, `amount_discount`, `amount_tax`, and `discounts`; verify `migrations_apply_and_are_idempotent` passes and the new columns/tables are present.
- [x] 1.2 In `migrate_legacy_columns`, add the new price and checkout columns with `add_column_if_missing`; extend the legacy-database test to assert the columns are added to pre-existing tables.
- [x] 1.3 Add the three new tables to `reset_state` in dependency-safe order; verify the existing reset/persistence tests still pass and a reset leaves the tables empty.
- [x] 1.4 Add coupon and billing portal session id helpers to `src/ids.rs` (`coupon_` and `bps_` prefixes); extend `prefixes_match` and verify `cargo test ids::` passes.

## 2. Expansion support

- [x] 2.1 In `src/api/mod.rs`, generalize the repeated-parameter parser used by `collect_includes` so it also collects `expand`, `expand[]`, indexed `expand[0]`, and comma-separated values; verify with unit tests for every accepted shape.
- [x] 2.2 Add an async expansion walker that traverses dot-separated paths over objects and arrays and resolves `product`, `price`, `customer`, `subscription`, and `payment_intent` through account-scoped loaders; verify unit tests cover a nested list path, an already-expanded object, an unknown path, and an id from another account.
- [x] 2.3 Wire expansion into the price, subscription, and checkout renderers using query parameters on retrieval/list and form parameters on create/update; verify with integration tests that `product`, `items.data.price.product`, and `line_items.data.price.product` produce embedded objects.
- [x] 2.4 Document the `expand[]` syntax and supported paths in `docs/api-foundations.md`; verify the documented curl example expands a price's product against a running mock.

## 3. Price lookup keys

- [x] 3.1 Add `lookup_key` to `PriceRow` and `to_json`, rendering null when absent; verify with a model unit test.
- [x] 3.2 In `create_price`, accept `lookup_key` and `transfer_lookup_key`, reject a duplicate active key, and clear the previous price's key on transfer inside the insert transaction; verify integration tests for create, duplicate rejection, and transfer.
- [x] 3.3 In `get_price` and `list_prices`, expose the lookup key and filter by repeated `lookup_keys[]`; verify integration tests for a matching filter, a non-matching filter, and account isolation.
- [x] 3.4 Update `docs/catalog.md` with lookup-key creation, filtering, uniqueness, transfer, and expansion examples; verify the documented commands against a running mock.

## 4. Coupons

- [x] 4.1 Add a `CouponRow` projection in `src/models.rs` with id, percent/amount discount, currency, duration, duration months, redemption limits, `times_redeemed`, `redeem_by`, name, `applies_to`, metadata, and created, plus a computed `valid`; verify with unit tests for a valid percent coupon, an expired coupon, and a fully redeemed coupon.
- [x] 4.2 Implement `POST /v1/coupons`, `GET /v1/coupons/{id}`, `GET /v1/coupons`, and `DELETE /v1/coupons/{id}` with validation, client-supplied ids, pagination, account scoping, and the deleted tombstone; add the routes in `src/lib.rs`; verify with integration tests.
- [x] 4.3 Add integration tests for both discount forms, both/neither rejection, percentage bounds, amount currency, repeating duration, unsupported duration, duplicate id, retrieve/list/delete, and connected-account isolation; verify `cargo test --test flows coupons` passes.
- [x] 4.4 Add a `docs/coupons.md` reference with create, retrieve, list, and delete examples plus the validation rules; verify the documented commands against a running mock.

## 5. Tax rates

- [x] 5.1 Add a `TaxRateRow` projection in `src/models.rs` with id, active, display name, percentage, inclusive flag, country/state/jurisdiction, description, tax type, metadata, and created; verify with a model unit test.
- [x] 5.2 Implement `POST /v1/tax_rates`, `GET /v1/tax_rates/{id}`, and `GET /v1/tax_rates` with required-field and percentage validation, pagination, `active`/`inclusive` filters, account scoping, and the routes in `src/lib.rs`; verify with integration tests.
- [x] 5.3 Add integration tests for create, inclusive/exclusive values, missing display name, missing inclusive flag, percentage bounds, list filters, unknown id, and connected-account isolation; verify `cargo test --test flows tax_rates` passes.
- [x] 5.4 Add a `docs/tax-rates.md` reference with create, retrieve, and list examples; verify the documented commands against a running mock.

## 6. Subscriptions

- [x] 6.1 Extend `build_subscription_item` to write `subscription`, `current_period_start`, and `current_period_end`, and make `SubscriptionRow::to_json` fill those fields for items stored by older builds; verify with model unit tests for new and legacy item JSON.
- [x] 6.2 Remove the canceled-subscription 404 from `get_subscription`; verify with an integration test that a canceled subscription is retrievable with `status` of `canceled` and still appears in the list.
- [x] 6.3 Implement `POST /v1/subscriptions/{id}` to update item quantities by item id (or by price on a one-item subscription), `cancel_at_period_end`, and `metadata`, rejecting unknown items, unknown prices, and invalid quantities, and emitting `customer.subscription.updated`; add the POST route; verify with integration tests.
- [x] 6.4 Compute `proration_behavior` figures from the remaining period fraction and return a top-level `prorations` array on the update response, with `none` returning an empty array and `always_invoice` creating no invoice; verify with unit tests for the formula and integration tests for increase, decrease, disabled, and unsupported behavior.
- [x] 6.5 Update `docs/billing.md` with subscription updates, item periods, canceled retrieval, prorations, and the new event; verify the documented commands against a running mock.

## 7. Checkout customer email

- [x] 7.1 Add `customer_email` to `CheckoutSessionRow` and render `customer_email` and `customer_details.email`, falling back to the linked customer's email; verify with model unit tests.
- [x] 7.2 Convert `create_checkout_session` to a transaction that creates and links a customer when `customer_email` is supplied without a customer, emits `customer.created` in the same transaction, and dispatches after commit; verify with integration tests for email-only, existing-customer, and event behavior.
- [x] 7.3 Update `docs/checkout.md` with the `customer_email` behavior and `customer_details`; verify the documented example against a running mock.

## 8. Checkout discounts, tax, and totals

- [x] 8.1 Add `amount_subtotal`, `amount_discount`, `amount_tax`, and `discounts` to `CheckoutSessionRow` and render `total_details` with discount/tax breakdowns and legacy fallbacks; verify with model unit tests.
- [x] 8.2 Implement coupon validation and sequential discount computation at session creation, cap discounts at the subtotal, and allocate the discount across lines proportionally with a deterministic rounding remainder; verify with unit tests including an uneven two-line split.
- [x] 8.3 Resolve line-level and session-level tax-rate ids, reject unknown or inactive rates, and compute exclusive and inclusive tax on discounted line amounts; store each line's `tax_rates` and `tax_amounts`; verify with unit tests for both tax modes and a line-level override.
- [x] 8.4 Wire discounts and tax into `create_checkout_session`, persisting `amount_subtotal`, `amount_discount`, `amount_tax`, `amount_total`, and the coupon snapshots; verify with integration tests for a discounted-taxed total, a zero-discount session, unknown coupon, invalid coupon, currency mismatch, unknown tax rate, and inactive tax rate.
- [x] 8.5 Increment `times_redeemed` for each applied coupon inside the completion transaction after the session is claimed; verify with integration tests that a completed session increments once and an expired session does not.
- [x] 8.6 Make the payment-mode completion use the session `amount_total` for its payment intent; verify with an integration test asserting the intent amount equals the discounted-taxed total.
- [x] 8.7 Update `docs/checkout.md` with coupon, tax-rate, and totals examples and the breakdown fields; verify the documented commands against a running mock.

## 9. Billing portal sessions and page

- [x] 9.1 Add a `BillingPortalSessionRow` projection in `src/models.rs` with id, customer, return url, url, and created; verify with a model unit test.
- [x] 9.2 Implement `POST /v1/billing_portal/sessions` with required `customer` and `return_url`, account-scoped customer lookup, a generated `bps_` id, and a mock URL, and add the route; verify with integration tests for success, unknown customer, missing return url, and no event.
- [x] 9.3 Add `GET /mock/billing-portal/{id}` to `src/mock.rs` rendering the customer and their subscriptions with quantities and period ends, a return link, an empty-list state, and the not-found page, and register the route in `src/lib.rs`; verify with integration tests.
- [x] 9.4 Update `docs/billing.md` and `docs/hosted-ui.md` with portal session creation and the hosted page; verify the documented flow against a running mock.

## 10. Integration and documentation

- [x] 10.1 Update `docs/openapi.spec3.json` with the new routes, parameters, and response fields; verify it parses as JSON and `openspec validate --all --strict` passes.
- [x] 10.2 Extend `scripts/sdk-e2e/e2e.js` with lookup-key lookup, price/product expansion, a subscription quantity update, a billing portal session, and a discounted-taxed checkout; verify `scripts/sdk-e2e/run.sh` succeeds.
- [x] 10.3 Update the README's API overview and known limitations so the new endpoints, lookup keys, expansion, portal page, and proration figures are discoverable; verify the linked docs match the implemented behavior.
- [x] 10.4 Run `cargo test --locked` and `openspec validate --all --strict`, and confirm both pass with no warnings.
