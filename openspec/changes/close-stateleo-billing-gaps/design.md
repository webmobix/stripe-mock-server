# Design

## Context

The mock already has the billing machinery this change builds on: subscription
invoices are raised by `raise_subscription_invoice` in `src/api/billing.rs`,
invoice JSON is rendered by `InvoiceRow::to_json`, subscription JSON by
`SubscriptionRow::to_json`, and both are embedded in webhook event payloads.
Subscriptions are stored as one row with their items serialized to JSON, and
invoices store their lines as JSON. `SubscriptionRow::to_json` is synchronous
and is called from event emission inside open transactions, which constrains
where extra data can come from.

`expand[]` is handled by `src/api/expand.rs`, which resolves reference ids by
field name through the pool but has no access to the request's base URL, so an
expanded invoice cannot currently render its hosted URL.

See `proposal.md` for motivation and the delta specs for required behavior.

## Goals / Non-Goals

**Goals:**

- Make every gap in the proposal observable through the public API or the
  internal control plane, without changing already-shipped behavior.
- Keep existing JSON fields and tests intact while adding the newer Stripe
  invoice shapes.
- Keep invoice and subscription rendering synchronous so event payloads and
  embedded objects stay consistent with API responses.

**Non-Goals:**

- Aligning billing periods to `billing_cycle_anchor_config[day_of_month]`
  beyond accepting and echoing it (the source document permits this).
- Changing `POST /mock/checkout` from `303` to `302`, or making `advance`
  `outcome` optional; both would break documented behavior that tests already
  assert.
- Backfilling `latest_invoice` or invoice `parent` details for rows created
  before the migration.
- General payment-method CRUD or card-source parity beyond what checkout saves.

## Decisions

### Persist subscription presentation fields as columns

Add `latest_invoice`, `default_payment_method`, `cancel_at`, `canceled_at`,
`ended_at`, and `billing_cycle_anchor_config` to `subscriptions`. The renderer
reads them directly.

Rationale: `SubscriptionRow::to_json` is synchronous and used inside event
emission. Querying invoices or payment methods at render time would force an
async renderer through many call sites and could yield different payloads for
the API and the event. Storing the ids keeps both identical.

Alternative considered: enrich subscription JSON in the HTTP layer only. That
would leave `customer.subscription.*` events missing `latest_invoice`, which
the spec requires.

### Raise the first invoice before emitting `customer.subscription.created`

`raise_subscription_invoice` stamps `subscriptions.latest_invoice` in the same
transaction. Subscription creation then reloads the row and emits
`.created` with the field populated.

Rationale: the spec requires `latest_invoice` in the created event. The
alternative — minting the invoice id before inserting the invoice — duplicates
id generation and risks a dangling reference on failure.

Trade-off: `invoice.created`/`invoice.paid` are now enqueued before
`customer.subscription.created`. Stripe does not guarantee event ordering, and
the suite waits per event, so this is acceptable.

### Attach the card before inserting the checkout subscription

Checkout completion resolves the customer, calls `attach_default_card`, and
passes the returned payment-method id into the subscription insert. This makes
`default_payment_method` present on `.created` without a second
`customer.subscription.updated` event.

### Snapshot subscription metadata on the invoice

Add `invoices.subscription_metadata`, populated from the subscription's
metadata when the invoice is raised. `InvoiceRow::to_json` uses it to build
`parent.subscription_details.metadata` synchronously.

Alternative considered: store a pre-rendered `parent` JSON column. A metadata
snapshot is smaller and keeps the parent assembled in one place.

### Build line `parent` and `pricing` at line construction

Subscription lines gain `parent.subscription_item_details` (subscription and
item ids) and `pricing.price_details` (price and product ids plus
`unit_amount_decimal`) where the line is built, in both
`subscription_invoice_lines` and the preview builder. Generic invoice lines get
`pricing` from their resolved price and `parent=null`. `total_taxes` is derived
in `InvoiceRow::to_json` from the lines' `tax_amounts`, matching the checkout
tax-breakdown shape.

Rationale: line-level data is already assembled per line; deriving it at render
time would require re-resolving prices. Existing `subscription` and
`subscription_item` fields stay, so current tests and consumers are unaffected.

### Cancel inside `advance_subscription_periods`

The `AdvanceRun::OnePeriod` branch checks `cancel_at_period_end` before building
a renewal. It sets `status=canceled`, `canceled_at`/`ended_at` to the current
period end, emits `customer.subscription.deleted`, and returns `None`. The
internal handler renders `invoice: null` instead of failing on a missing
invoice. The clock-driven `Through` branch gets the same timestamp treatment.
An existing open invoice is still retried first, so failure recovery keeps
working.

### Add base URL to expansion

`expand_response` and `expand_list_response` take the request's base URL and
pass it to `resolve`, which can then render a full invoice, including
`hosted_invoice_url` and `invoice_pdf`, for `latest_invoice`. This is a
mechanical signature change across existing call sites; the alternative of
rendering relative links would produce JSON that differs from real Stripe.

### Validate and store the new checkout parameters

`payment_method_collection` accepts `always` or `if_required` (default
`always`). `automatic_tax[enabled]` is parsed as a boolean (default false) and
stored as JSON. The session renderer reports `automatic_tax` as
`{enabled, status: null}`. Neither changes pricing, matching the source
document's "accept and echo" requirement.

## Risks / Trade-offs

- **Event order changes** → Run the full integration suite and assert events by
  type, not by global ordering; the SDK e2e already waits per event.
- **Expansion signature churn (33 call sites)** → Purely mechanical; compiler
  catches every missed call, and expansion unit tests cover the new path.
- **Older rows have null new fields** → Renderers emit `null`/false defaults;
  no JSON migration is needed, and no test depends on pre-migration rows.
- **`parent` duplication with legacy fields** → Both are intentional for
  backwards compatibility; docs call out that the legacy pair is retained.
- **Anchor day is echoed but not applied** → Documented limitation; the source
  document explicitly accepts echo-only behavior.

## Migration Plan

1. Add the new columns to `SCHEMA` and to `migrate_legacy_columns` with
   `add_column_if_missing`, so existing `stripe-mock.db` files upgrade in
   place.
2. Ship with defaults: `latest_invoice`/`default_payment_method`/`cancel_at`/
   `canceled_at`/`ended_at` null, `payment_method_collection='always'`,
   `automatic_tax='{"enabled":false}'`.
3. Rollback is a code revert; the extra nullable columns are harmless to older
   builds.

## Open Questions

None.
