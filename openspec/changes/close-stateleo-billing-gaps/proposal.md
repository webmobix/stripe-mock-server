# Proposal

## Why

StateLeo's billing suite already runs against the mock for meters, embedded
checkout, stable credentials, previews, idempotency, and webhook delivery. Seven
gaps remain between the mock and the API surface the suite documents: raw meter
events cannot be inspected, subscription objects lack invoice/cancellation
fields, a direct period advance ignores `cancel_at_period_end`, the subscription
never receives the saved card, subscription invoices lack the `parent`/`pricing`
shape the app reads, and two checkout/anchor parameters are silently dropped.
Closing them removes the fallbacks the application currently needs and makes the
billing page and cancellation tests exercise real Stripe shapes.

## What Changes

- **Raw meter event inspection.** Add `GET /internal/meter_events` with
  `event_name` and `customer` filters, returning accepted events so a test can
  assert exactly what the application sent.
- **Subscription object fields.** Subscriptions SHALL report `latest_invoice`,
  `default_payment_method`, `cancel_at`, `canceled_at`, and `ended_at`, and
  `latest_invoice`/`default_payment_method` SHALL be expandable. The
  `customer.subscription.created` event SHALL carry `latest_invoice`.
- **Subscription default card.** Completing a checkout SHALL set the created
  subscription's `default_payment_method` to the saved card, in addition to the
  customer's `invoice_settings.default_payment_method`.
- **Cancellation on advance.** `POST /internal/subscriptions/{id}/advance` SHALL
  cancel a subscription with `cancel_at_period_end=true`, emitting
  `customer.subscription.deleted` and returning a null invoice, instead of
  raising a renewal.
- **Invoice parent and pricing shape.** Subscription invoices SHALL expose
  `parent.subscription_details`, `total_taxes`, and per-line
  `parent.subscription_item_details` and `pricing.price_details` (with
  `unit_amount_decimal`), additively alongside the existing `subscription` and
  `subscription_item` fields.
- **Billing cycle anchor.** `POST /v1/subscriptions` SHALL accept and echo
  `billing_cycle_anchor_config[day_of_month]`.
- **Checkout echoes.** Sessions SHALL accept, store, and echo
  `payment_method_collection` and `automatic_tax` (reporting
  `automatic_tax.status=null`), and previews SHALL include the new invoice
  fields.

Deliberately not changed: the already-shipped meters, metered/tiered pricing,
embedded checkout, first invoice, preview, idempotency, and delivery behavior;
`POST /mock/checkout` keeps its `303` redirect (the document's `302` is a loose
description, and `303` is the correct status for a form POST); `advance` keeps a
required `outcome` because the current spec explicitly rejects a missing one.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `billing`: subscription `latest_invoice`/`default_payment_method`/cancellation
  fields, cancellation on direct advance, and the billing cycle anchor.
- `checkout`: `payment_method_collection`/`automatic_tax` echoes and the
  subscription default card at completion.
- `invoicing`: subscription invoice `parent`, `total_taxes`, and line `pricing`
  fields, including previews.
- `internal-control-plane`: raw meter event inspection and an advance response
  that may have no invoice when a subscription is cancelled.

## Impact

- New migration columns in `src/db.rs`: subscription
  `latest_invoice`/`default_payment_method`/`cancel_at`/`canceled_at`/`ended_at`
  and checkout `payment_method_collection`/`automatic_tax`; row fields and
  renderers in `src/models.rs`.
- `src/api/billing.rs` (cancellation branch, latest-invoice stamping, invoice
  line `parent`/`pricing`), `src/api/v1.rs` (subscription creation, checkout
  creation/completion, anchor), `src/api/invoicing.rs` (invoice/preview JSON),
  `src/api/expand.rs` (`latest_invoice`), `src/api/payment_methods.rs` (return
  the saved card for subscription assignment), and `src/internal.rs` plus
  `src/lib.rs` (meter events endpoint, nullable advance invoice).
- Tests: extend `tests/stateleo.rs` and `tests/invoicing.rs`; extend
  `scripts/sdk-e2e/e2e.js` with the acceptance flow's remaining assertions.
- Docs: `docs/billing.md`, `docs/invoicing.md`, `docs/checkout.md`,
  `docs/internal.md`, `docs/meters.md`, and `docs/openapi.spec3.json`.
- No new dependencies and no Dockerfile changes.
