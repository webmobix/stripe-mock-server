# Proposal

## Why

Other applications are starting to use this mock as their Stripe substitute, and several
pieces of the API they already call are missing or incomplete. The consequences are
concrete: plans have to be resolved out of a hand-maintained `STRIPE_PRICE_MAP`, product
names need an extra request, seat synchronization fails against a 404, the "Manage
subscriptions" button has nowhere to go, canceled subscriptions disappear from history,
and checkout responses cannot express discounted or taxed totals. Each gap forces a
workaround in the consuming app or a test that cannot assert real behavior.

## What Changes

- **Price lookup keys.** `POST /v1/prices` accepts `lookup_key`, the price object exposes
  it, and `GET /v1/prices` filters by repeated `lookup_keys[]`. Lookup keys are unique
  among active prices for an account; `transfer_lookup_key=true` moves a key from a
  previous price.
- **Response expansion.** The server accepts repeated `expand[]` parameters and replaces
  reference ids with full objects along the requested path, including `price.product`,
  `items.data.price.product` on subscriptions, and `line_items.data.price.product` on
  checkout sessions.
- **Subscription updates.** `POST /v1/subscriptions/{id}` updates item quantities,
  accepts `proration_behavior`, recalculates proration figures from the daily rate, and
  creates a `customer.subscription.updated` event.
- **Subscription item periods.** Each `items.data[]` entry carries its own
  `current_period_start` and `current_period_end`, so "next payment date" is populated.
- **Canceled subscriptions stay readable.** `GET /v1/subscriptions/{id}` returns a
  canceled subscription instead of a 404.
- **Billing portal sessions.** `POST /v1/billing_portal/sessions` creates a `bps_`
  session with a `url`, and a minimal hosted page lists the customer's subscriptions and
  honors `return_url`.
- **Checkout customer email.** `POST /v1/checkout/sessions` accepts `customer_email`;
  when no customer is supplied, the server creates and links a `cus_` customer
  immediately, and exposes `customer_email` and `customer_details.email`.
- **Coupons and tax rates.** New `POST/GET /v1/coupons` and `POST/GET /v1/tax_rates`
  resources (with retrieve, list, and coupon delete) back Stripe-shaped discount and tax
  inputs on checkout sessions.
- **Checkout totals.** Checkout sessions accept coupon discounts and tax rates, and
  return `amount_subtotal`, an updated `amount_total`, `discounts`, and
  `total_details.amount_discount`, `amount_tax`, and `breakdown`, with tax computed on
  the discounted subtotal.

No existing endpoint changes its URL or removes a field. Existing clients that do not
send the new parameters keep their current behavior.

## Capabilities

### New Capabilities

- `coupons`: creates, retrieves, lists, and deletes account-scoped coupons with percent
  or amount discounts, their duration and redemption limits, and exposes the validated
  coupon used for checkout discounts.
- `tax-rates`: creates, retrieves, and lists account-scoped tax rates with a percentage,
  inclusive/exclusive behavior, and display metadata for use on checkout line items.

### Modified Capabilities

- `api-foundations`: adds `expand[]` response expansion, path traversal over embedded
  lists, and lenient handling of paths the server cannot expand.
- `catalog`: prices gain `lookup_key` on create, retrieve, list, and filter, with
  account-scoped uniqueness and optional transfer, and price responses support product
  expansion.
- `billing`: subscriptions gain an update endpoint with item quantity changes and
  proration figures, per-item period fields, retrieval of canceled subscriptions, and
  billing portal sessions with a hosted URL.
- `checkout`: sessions gain `customer_email` customer creation, coupon discounts, tax
  rates, and the subtotal/discount/tax breakdown in the session response.
- `hosted-simulation-ui`: adds the mock billing portal page reached from a billing
  portal session URL.

## Impact

- **Code.** `src/api/v1.rs` gains price lookup-key filtering, subscription update,
  billing portal, coupon, and tax-rate handlers plus checkout pricing; a small expansion
  helper will be shared by the price, subscription, and checkout renderers.
  `src/models.rs` gains coupon, tax-rate, and billing-portal row projections and the
  checkout total fields. `src/mock.rs` gains the billing portal page. `src/lib.rs`,
  `src/ids.rs`, and `src/db.rs` gain routes, ids, tables, and migrations.
- **Database.** New `coupons`, `tax_rates`, and `billing_portal_sessions` tables;
  `prices.lookup_key`; `checkout_sessions.customer_email`, `amount_subtotal`,
  `amount_discount`, `amount_tax`, and `discounts`. `reset_state` is extended.
- **API surface.** New coupon, tax-rate, billing-portal, subscription-update, and mock
  billing-portal routes; new parameters on price, subscription, and checkout requests.
- **Events.** `customer.subscription.updated` is emitted on subscription updates and
  `customer.created` when checkout creates a customer from `customer_email`. Coupon,
  tax-rate, and billing-portal calls emit no events.
- **Compatibility.** Additive only. Lookup-key duplicates are rejected unless
  `transfer_lookup_key` is set, matching Stripe's uniqueness rule.
- **Tests.** New integration coverage in `tests/flows.rs` and `tests/invoicing.rs` where
  relevant, plus model unit tests for totals and coupon rendering.
- **Docs.** `docs/catalog.md`, `docs/billing.md`, `docs/checkout.md`, and
  `docs/hosted-ui.md` are updated; the OpenAPI description gains the new routes.
