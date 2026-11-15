# Proposal

## Why

The mock fits Stripe SDK 23 for the app's subscription checkout flow, but eight
areas block the rest of the product. In the order they block work:

1. Usage reporting cannot be exercised because there are no billing meters,
   meter events, or event summaries.
2. The real plan catalogue uses metered and tiered prices, and real checkout and
   subscription items reject a quantity on a metered price.
3. Embedded checkout is unsupported: no `ui_mode`, `return_url`, `client_secret`,
   or `subscription_data.metadata`.
4. Keys and signing secrets are generated, so a test cannot hard-code them and
   they do not survive `/internal/reset`.
5. There is no way to end a billing period, so first/renewal invoices, a failed
   renewal, and recovery cannot be tested.
6. The billing page has no upcoming-invoice endpoint (`invoices.createPreview`).
7. A completed checkout leaves the customer without a default card.
8. Idempotency keys are not honored.

## What Changes

- **Billing meters and usage.** Add `/v1/billing/meters`,
  `/v1/billing/meters/{id}` (retrieve, update, deactivate, reactivate),
  `/v1/billing/meter_events`, and
  `/v1/billing/meters/{id}/event_summaries` with `sum`, `count`, and `last`
  aggregation and hour/day grouping. Duplicate identifiers return
  `duplicate_meter_event`.
- **Metered and tiered prices.** Prices accept `recurring.usage_type=metered`
  with `recurring.meter`, plus `billing_scheme=tiered`, `tiers_mode`, and
  graduated/volume `tiers[]`. Tiered and metered amounts feed checkout totals,
  first invoices, renewals, and previews. A quantity on a metered line is
  rejected.
- **Embedded checkout.** Sessions accept `ui_mode=embedded_page` (and the
  `embedded` alias), require `return_url`, report `client_secret`, and omit
  `url`. `subscription_data.metadata` is copied to the created subscription.
- **Configurable secrets.** `[main_account]` accepts `secret_key` and
  `publishable_key`; `webhook_endpoint` accepts `signing_secret`. Configured
  values are re-applied after `/internal/reset`.
- **Billing periods.** Subscription creation raises a paid
  `subscription_create` invoice. `POST /internal/subscriptions/{id}/advance`
  ends a period with `outcome=paid|failed`, raises a `subscription_cycle`
  invoice, and retries an existing open invoice so failure and recovery are
  testable.
- **Invoice preview.** Add `POST /v1/invoices/create_preview` and
  `GET /v1/invoices/upcoming`, returning a non-persistent draft
  `upcoming_in_` invoice priced from a subscription or proposed items.
- **Default card.** Completing a checkout saves a Visa `pm_` card on the
  customer and sets `invoice_settings.default_payment_method`; add payment
  method retrieval/list endpoints and expansion.
- **Idempotency.** Honor `Idempotency-Key` on v1 `POST`/`DELETE`/`PATCH`,
  replaying cached responses and rejecting reuse with different parameters.

## Capabilities

### New Capabilities

- `billing-meters`: meters, meter events, duplicate-identifier rejection, and
  event summaries.

### Modified Capabilities

- `catalog`: metered and tiered prices, and rejection of quantities on metered
  lines.
- `billing`: first and renewal subscription invoices, and a default card on the
  customer.
- `checkout`: embedded sessions, subscription metadata, and the saved card.
- `invoicing`: subscription invoice fields and the upcoming-invoice preview.
- `internal-control-plane`: configured secrets surviving reset and the
  billing-period advance call.
- `server-configuration`: fixed main-account keys and endpoint signing secrets.
- `api-foundations`: idempotency keys.

## Impact

- New modules `src/api/billing.rs`, `src/api/meters.rs`,
  `src/api/payment_methods.rs`, and `src/idempotency.rs`.
- Routes in `src/lib.rs`; schema and migrations in `src/db.rs`; models in
  `src/models.rs`; checkout/subscription changes in `src/api/v1.rs`, invoice
  and preview changes in `src/api/invoicing.rs`, the advance call in
  `src/internal.rs`, and seeding in `src/seed.rs`.
- New integration coverage in `tests/stateleo.rs`; Stripe SDK 23 coverage in
  `scripts/sdk-e2e/e2e.js`.
- Documentation in `docs/`, including a new `docs/meters.md`, and the sample
  `config.toml`.
- No new dependencies and no Dockerfile changes.
