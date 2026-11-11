# Design

## Context

See `proposal.md` — Why. The mock is a single Rust/Axum process over one SQLite
database. The schema is declared in `src/db.rs` (`SCHEMA` plus an idempotent
`migrate_legacy_columns` pass for databases created by older builds). Every v1
endpoint is a hand-written handler in `src/api/v1.rs`; form bodies are parsed by
`src/form.rs` into nested JSON; rows are projected through `to_json()` methods in
`src/models.rs`; and state changes follow one pattern: open a transaction, write,
`events::emit(...)`, commit, then `dispatcher.dispatch_event(...)`.

Three constraints shape this change:

- `subscriptions.items` and `checkout_sessions.line_items` are JSON columns, not
  normalized tables. Anything that must be read back with a line travels in that JSON.
- Existing database files must keep working. New columns arrive through
  `add_column_if_missing`; new tables use `CREATE TABLE IF NOT EXISTS`.
- Every v1 resource is account-scoped by the resolved account id. Lookups by id must
  include `account_id`, otherwise connected-account data leaks.

The current gaps are visible in the code: `PriceRow` has no lookup key; `get_price`
ignores `expand`; `SubscriptionRow` has no update handler and hides canceled rows;
`CheckoutSessionRow` has only `amount_total`; there is no coupon, tax-rate, or billing
portal resource; and the router in `src/lib.rs` has no POST on `/v1/subscriptions/{id}`.

## Goals / Non-Goals

**Goals:**

- Add the missing resources and parameters without changing existing URLs or removing
  fields.
- Keep all new pricing rules in one checkout pipeline so discount, tax, and total fields
  agree with each other and with what `complete_checkout_session` charges.
- Keep the expansion helper usable by prices, subscriptions, and checkout line items
  without duplicating account-scoped loaders.
- Keep old rows readable: no destructive migration and no requirement to rewrite
  existing JSON columns.

**Non-Goals:**

- Promotion codes, coupon update, coupon expiration sweeps, or redemption history.
- Subscription item add/remove/price-swap, trial changes, billing-cycle-anchor changes,
  or invoice creation for prorations.
- Billing portal configuration, flow data, locale selection, or cancellation controls on
  the portal page; the page lists subscriptions and returns to the app.
- Stripe Tax address resolution or automatic tax. Tax is explicit through tax-rate ids.
- Currency conversion, shipping, or idempotency keys.

## Decisions

### Store the new resources in dedicated tables

`coupons`, `tax_rates`, and `billing_portal_sessions` get their own tables. They are
independent resources with list and retrieve endpoints, unlike invoice lines or
subscription items, which are always read with their parent.

- Alternative rejected: storing coupons and tax rates as JSON on the checkout session
  only. That would make `GET /v1/coupons` and `GET /v1/tax_rates` impossible and prevent a
  coupon from being reused across sessions.

`coupons.times_redeemed`, `max_redemptions`, and `redeem_by` are stored; `valid` is
computed on render. `applies_to` is a JSON column holding `{ "products": [...] }`.
`tax_rates.percentage` is `REAL`, matching the inline tax-rate object already produced by
invoicing; `inclusive` and `active` are integers.

### Add columns rather than a second checkout table

`checkout_sessions` gains `customer_email`, `amount_subtotal`, `amount_discount`,
`amount_tax`, and `discounts`. The existing `amount_total`, `line_items`, and currency
columns stay authoritative. `prices` gains `lookup_key`.

The total columns are nullable; `CheckoutSessionRow::to_json` falls back to
`amount_total` for `amount_subtotal` and to zero for discount/tax when reading a row
created before this change. New rows always write all of them. `discounts` holds the
applied coupon snapshots with their computed amounts, so a later coupon deletion does not
change an existing session's totals or breakdown.

- Alternative rejected: a `checkout_totals` JSON column. Individual integer columns keep
  the money values queryable and match how invoices already store `subtotal`, `tax_total`,
  and `total`.

### Lookup keys are unique per account among active prices

On `POST /v1/prices`, a supplied `lookup_key` is checked against active prices for the
resolved account. A conflict is an `invalid_request_error` unless
`transfer_lookup_key=true`, in which case the previous price's key is cleared in the same
transaction as the new insert. Inactive prices do not reserve a key.

- Alternative rejected: a unique database index. It would block `transfer_lookup_key`
  unless the transfer runs before the insert, and SQLite would return a constraint error
  that does not name `lookup_key`. A scoped query gives the Stripe-shaped error.
- Alternative rejected: allowing duplicates. Consuming apps resolve plans by lookup key;
  duplicates make that ambiguous.

`GET /v1/prices` reads repeated `lookup_keys[]` values and filters in memory, as it
already does for `product`.

### One asynchronous expansion walker keyed by field name

A new helper (in `src/api/expand.rs` or beside `collect_includes` in `src/api/mod.rs`)
walks a rendered `Value` along each dot-separated path. At an array it applies the same
segment to every element; at an object it reads the named field. If the field holds a
string id and more segments follow, it resolves the object through an account-scoped
loader and continues.

Resolution is keyed by field name, not by id prefix alone: `product` resolves `prod_`,
`price` resolves `price_` only when the value is still a string (subscription items
already embed a price object), `customer` resolves `cus_`, `subscription` resolves
`sub_`, and `payment_intent` resolves `pi_`. This covers `product`,
`items.data.price.product`, and `line_items.data.price.product` with one implementation.

Unknown paths, unresolvable ids, and references invisible to the account leave the
original value untouched. The alternative — returning an `invalid_request_error` for an
unsupported path — would break SDK calls that send expansions the mock has not taught
itself yet, and the consuming apps only need the supported subset.

The existing `collect_includes` query parser is generalized into a list-parameter parser
that also collects `expand`, so `expand`, `expand[]`, indexed `expand[0]`, and
comma-separated values all work. Create and update handlers read the parsed form body
instead of a query string.

### Subscription items are self-describing, with lazy fallbacks for old rows

`build_subscription_item` now writes `subscription`, `current_period_start`, and
`current_period_end` into each item in addition to id, price, and quantity.
`SubscriptionRow::to_json` fills those three fields from the parent row when an item
stored by an older build lacks them, so no JSON migration is needed.

The update handler reads the stored items array, matches entries by item id (or by the
single item's price when no id is sent), replaces quantities, serializes the array back,
and keeps the subscription's period fields. `cancel_at_period_end` and `metadata` are
updated only when supplied.

### Proration figures are response-only

`POST /v1/subscriptions/{id}` accepts `proration_behavior` of `create_prorations`
(default), `none`, or `always_invoice`. When a quantity changes and prorations are
enabled, it computes, per changed item:

```
amount = round(unit_amount * quantity_delta * remaining / period_length)
```

where `remaining = max(0, current_period_end - now)` and
`period_length = current_period_end - current_period_start`, both clamped so a completed
period yields zero. Rounding is half away from zero, matching invoice tax rounding.
The figures are returned in a top-level `prorations` array on the update response and are
not persisted; `always_invoice` returns the same figures without creating an invoice.

- Alternative rejected: creating draft invoices for prorations. The user explicitly chose
  no invoice creation, and it would couple subscriptions to the invoicing subsystem.
- Alternative rejected: accepting `proration_behavior` and computing nothing. Seat-sync
  callers need the endpoint to work, but tests also need to assert the amount change;
  a response-only field gives them that without inventing invoices.

### Canceled subscriptions are ordinary retrievable rows

The `status == "canceled"` early return is removed from `get_subscription`. Cancellation
already writes the row, so retrieval and listing need no other change. `DELETE` on an
already-canceled subscription keeps its current behavior.

### Checkout customer creation is part of session creation

`create_checkout_session` is converted to a single transaction. If `customer_email` is
present and neither `customer` nor `customer_account` is, a `cus_` row is inserted and a
`customer.created` event is emitted in that transaction; the session records the new
customer. Events are dispatched after commit, following the existing pattern.

- Alternative rejected: deferring creation to checkout completion. The apps currently
  pre-create a customer in hosted mode; creating at session creation lets them drop that
  step and gives the session a stable customer immediately.
- Alternative rejected: storing the email without creating a customer. The session would
  then have no customer for subscription completion, which is the problem being solved.

`customer_details.email` is rendered from `customer_email`, falling back to the linked
customer's email, so a session created with an existing customer still exposes it.

### Checkout pricing is one ordered pipeline

Session creation runs these steps before insert:

1. Resolve each line's price and build the line amount (`unit_amount * quantity`).
2. Validate coupons and compute discount amounts sequentially against the remaining
   subtotal. `amount_off` is capped at the remainder; `percent_off` is applied to the
   remainder and rounded half away from zero. The discount stops at zero.
3. Allocate the total discount across lines in proportion to line amount, with the
   rounding remainder assigned to the largest lines, so per-line tax is computed on a
   discounted amount that sums back to `amount_discount`.
4. Resolve tax-rate ids: a line's own `tax_rates[]` replaces the session-level
   `tax_rates[]`. Exclusive tax is `round(discounted * p / 100)` and is added to the
   total. Inclusive tax is `round(discounted * p / (100 + p))`, is reported in
   `amount_tax`, but is already inside the line amount and is not added again.
5. `amount_subtotal` is the pre-discount sum, `amount_discount` the applied discount,
   `amount_tax` the sum of line tax amounts, and `amount_total` the discounted subtotal
   plus exclusive tax.

Each line stores its resolved `tax_rates` and `tax_amounts` in `line_items`, so retrieval
does not recompute tax. `CheckoutSessionRow::to_json` derives
`total_details.breakdown.discounts` from the stored coupon entries and aggregates
`total_details.breakdown.taxes` by tax-rate id from the line tax amounts.

- Alternative rejected: taxing before discounting. The consuming app's assertion is a
  discounted-then-taxed total, and Stripe applies discounts before tax.
- Alternative rejected: storing only the aggregate discount. Proportional allocation is
  needed for line-level rates to produce a tax sum consistent with `amount_discount`.
- Alternative rejected: `automatic_tax`. It needs an address and a tax engine; explicit
  tax-rate ids are deterministic for a mock.

Session-level `tax_rates[]` is a mock extension; Stripe Checkout normally derives tax
from `automatic_tax`. Line-level `tax_rates[]` mirrors the invoice line shape already
supported by this mock.

### Coupon redemption happens at completion

Coupon validity is checked at session creation, but `times_redeemed` increments inside
`complete_checkout_session`'s transaction, after the session is claimed as complete. A
session that expires or is never completed therefore does not consume a redemption, and
the claim makes concurrent completions increment at most once.

### Billing portal session plus a minimal page

`billing_portal_sessions` stores the generated `bps_` id, account, customer, return url,
url, and created time. The response is rendered from the row. The URL is
`{public_base_url}/mock/billing-portal/{id}`; the page handler loads the session and its
customer's subscriptions and renders them with the shared shell. It does not require
authentication, like the other hosted pages.

- Alternative rejected: returning an external Stripe URL. Opening it would leave the
  mock and depend on the network.
- Alternative rejected: returning a URL with no page. The consuming app's "Manage
  subscriptions" button would open a 404.

## Risks / Trade-offs

- **Approximated Stripe shapes** → Keep the fields the specs name, reuse existing object
  shapes (`coupon`, `tax_rate`, `billing_portal.session`), and document mock-only fields
  (`prorations`, session-level `tax_rates[]`) in the docs so consumers do not mistake
  them for hosted Stripe behavior.
- **Large router and handler surface in one module** → Group new handlers beside their
  domain in `src/api/v1.rs` and keep validation helpers small; split only the expansion
  walker into its own module. A future change can split coupons/tax rates out.
- **Expansion performs one query per expanded reference** → Acceptable for a local mock
  with small lists. Resolution is account-scoped, and failures are non-fatal.
- **Proration rounding can differ from Stripe's by a cent** → Use the same half-away-from-
  zero rule as invoice tax, document the formula, and assert the mock's formula in tests.
- **Discount allocation across lines is complex** → Cover it with a unit test over two
  lines whose proportional shares do not divide evenly, asserting the allocated parts sum
  to `amount_discount`.
- **Old checkout rows have no subtotal** → Render fallbacks keep old rows readable; no
  data rewrite is required.
- **Coupon redemption races** → The session completion claim (`status = 'open'`) is the
  gate; the increment is in the same transaction as that claim.

## Migration Plan

1. `SCHEMA` adds the three new tables and the new columns for fresh databases.
2. `migrate_legacy_columns` adds `prices.lookup_key` and the five checkout columns when
   missing, using `add_column_if_missing`; existing rows keep null/default values.
3. `reset_state` adds `billing_portal_sessions`, `coupons`, and `tax_rates` to its delete
   list, ordered so no parent is removed before a child.
4. Rollback is code-only: the added columns and tables are inert to previous builds, so
   reverting the binary leaves the database readable.

## Open Questions

None. The mock-specific choices (response-only `prorations`, session-level
`tax_rates[]`, and a list-only portal page) are recorded above and in the specifications.
