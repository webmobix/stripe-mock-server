# Tasks

## 1. Schema and model fields

- [x] 1.1 Add subscription columns (`latest_invoice`, `default_payment_method`, `cancel_at`, `canceled_at`, `ended_at`, `billing_cycle_anchor_config`), `invoices.subscription_metadata`, and checkout columns (`payment_method_collection`, `automatic_tax`) to `SCHEMA` and to `migrate_legacy_columns` with `add_column_if_missing`; verify `cargo test db::` and that the idempotent-migration test still passes.
- [x] 1.2 Extend `SubscriptionRow`, `InvoiceRow`, and `CheckoutSessionRow` with the new fields and render them with null/default fallbacks; verify new `models::` unit tests assert null `latest_invoice`/`default_payment_method`/`cancel_at`, `payment_method_collection=always`, and `automatic_tax={enabled:false,status:null}` for a row without them.

## 2. Subscription fields and latest invoice

- [x] 2.1 Stamp `subscriptions.latest_invoice` inside `raise_subscription_invoice` and reorder direct subscription creation so the row is reloaded and `customer.subscription.created` is emitted after the first invoice; verify an integration test asserts `latest_invoice` on the subscription and in the event payload.
- [x] 2.2 Set `cancel_at` to `current_period_end` when `cancel_at_period_end` turns true and null when it turns false, and emit `canceled_at`/`ended_at` when a subscription is canceled; verify the update and cancellation tests.
- [x] 2.3 Thread the request base URL through `expand_response`/`expand_list_response` and add a `latest_invoice` resolver that renders the full invoice; verify `expand::` unit coverage and an SDK-style retrieval with `expand[]=latest_invoice,default_payment_method`.
- [x] 2.4 Document the new subscription fields, expansion, and `cancel_at` behavior in `docs/billing.md`; verify the documented retrieve request matches the tested shape.

## 3. Checkout card and parameter echoes

- [x] 3.1 Parse, store, and render `payment_method_collection` (`always` default, `if_required` accepted) and `automatic_tax[enabled]` (`status=null`), rejecting an unsupported collection value; verify create, retrieve, and post-completion integration assertions.
- [x] 3.2 Call `attach_default_card` before inserting the checkout subscription and pass its id into the insert so `default_payment_method` is set; verify the subscription retrieval and the `customer.subscription.created` payload carry the card id.
- [x] 3.3 Document the echo parameters and the subscription default card in `docs/checkout.md`; verify the examples match the tested requests.

## 4. Cancellation and billing cycle anchor

- [x] 4.1 Cancel a `cancel_at_period_end` subscription in the `AdvanceRun::OnePeriod` branch after any open-invoice retry, stamping `canceled_at`/`ended_at`, emitting `customer.subscription.deleted`, and returning no invoice; set the same timestamps in the clock-driven branch and make the internal advance response report `invoice: null`; verify `tests/stateleo.rs` and `tests/test_clocks.rs`.
- [x] 4.2 Accept and validate `billing_cycle_anchor_config[day_of_month]` (1–31), store it, and echo it on the subscription; verify valid and invalid-day integration tests.
- [x] 4.3 Document the cancellation-on-advance response and the anchor echo in `docs/billing.md` and `docs/internal.md`; verify the documented `advance` body and response.

## 5. Invoice parent and pricing shape

- [x] 5.1 Add `parent.subscription_item_details` and `pricing.price_details` (with `unit_amount_decimal`) to subscription invoice lines and preview lines, and `pricing` from the resolved price on generic invoice lines; verify line-level assertions in `tests/invoicing.rs` and `tests/stateleo.rs`.
- [x] 5.2 Add `parent.subscription_details` (using the stored `subscription_metadata`) and `total_taxes` to invoice and preview JSON while keeping `subscription`/`subscription_item`; verify a subscription invoice, a taxed invoice, and a preview assertion.
- [x] 5.3 Document the additive invoice fields in `docs/invoicing.md`; verify the documented JSON matches the tested response.

## 6. Raw meter event inspection

- [x] 6.1 Add `GET /internal/meter_events` with `event_name` and `customer` filters returning a v1 list envelope of `identifier`, `event_name`, `customer`, `value`, and `timestamp`, registered outside Stripe auth; verify integration tests for both filters and an unfiltered list.
- [x] 6.2 Document the control-plane endpoint in `docs/internal.md` and `docs/meters.md`; verify the documented query matches the tested response.

## 7. Integration checks

- [x] 7.1 Extend `scripts/sdk-e2e/e2e.js` with the acceptance flow's remaining assertions (subscription default card and `latest_invoice`, invoice parent/pricing, cancellation on advance, meter event inspection) and run `scripts/sdk-e2e/run.sh` to `E2E_SDK_OK`.
- [x] 7.2 Run `cargo test --locked` and confirm the full unit and integration suite passes.
- [x] 7.3 Run `openspec validate --all --strict` and confirm the change and specs have no errors or warnings.

## Workflow follow-up

- Archive the change after review and a green validation run.
