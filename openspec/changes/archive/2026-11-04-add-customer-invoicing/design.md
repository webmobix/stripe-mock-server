# Design

## Context

See `proposal.md` — Why. The mock persists every resource in a single SQLite
database whose schema lives in `src/db.rs` (`SCHEMA` plus an idempotent
`migrate_legacy_columns` pass for pre-existing database files), and every v1
endpoint is a hand-written handler in `src/api/v1.rs` that parses a
form-encoded body with `src/form.rs`, writes rows with `sqlx`, and renders a
row through a `to_json()` projection in `src/models.rs`. State changes that
must be observable as webhooks follow a fixed pattern: open a transaction, do
the write, call `events::emit(...)` with the rendered object, commit, then
`dispatcher.dispatch_event(&event_id)` after the commit.

Two tables already exist for this change but are unused by any endpoint:
`customers` (only `email`, `name`, `created`, `metadata`) and
`payment_intents` (only populated as a side effect of checkout completion).
The `payment_intents` row is created directly by `complete_checkout_session`
with a hard-coded `status = 'succeeded'`.

`src/form.rs` already expands Stripe's bracketed notation
(`lines[0][price]`, `lines[][quantity]`) into nested JSON, so the invoice
parameters in the specs need no new parsing machinery.

## Goals / Non-Goals

**Goals:**

- A customer with tax ids, an invoice with priced and taxed lines, and a
  payment intent that settles it — all reachable over the public v1 API.
- Invoice status transitions that behave like Stripe's: `draft` → `open` →
  `paid` / `void` / `uncollectible`, with invalid transitions rejected.
- A single settlement path (`payment_intents.invoice` + the resolve endpoint)
  that both `POST /v1/invoices/{id}/pay` and a test can drive.
- No change to existing customer, subscription, checkout, or webhook
  behavior.

**Non-Goals:**

- A `/v1/tax_rates` API or Stripe Tax. Tax rates are inline on a line.
- `POST /v1/invoiceitems`, invoice voiding by credit note, dunning,
  subscriptions driving invoices, or auto-advancing invoices.
- Hosted invoice pages. The mock has no invoice UI; invoices are read and
  driven through the API and the control plane.
- Partial-payment schedules, refunds, credit notes, or `amount_shipping`.
- Idempotency keys and automatic tax computation.

## Decisions

### Store invoice lines as a JSON column, not a second table

The invoice's lines, their `tax_amounts`, and the per-line tax rate are stored
as one JSON column on the `invoices` row, exactly as `checkout_sessions` stores
`line_items` and `subscriptions` stores `items`. A separate `invoice_lines`
table would buy nothing here: lines are never queried independently of their
invoice, are always written with the invoice, and are only ever rendered as
part of the invoice object.

- Alternative rejected: a normalized `invoice_lines` table. It would add a
  second write path and a second transaction boundary for no query benefit.

### Express a tax rate inline on the line

`lines[][tax_rate][percentage]` (plus an optional `name`) is the tax rate
input. The server materializes it as a `tax_rate` object on the line
(`txr_` id, `percentage`, `name`, `inclusive: false`, `object: "tax_rate"`)
and derives `tax_amounts` from it. Tax is computed once, at invoice creation,
and stored — there is no tax calculation engine to re-run later.

- Alternative rejected: a `tax_rates` resource plus ids on the line. That is
  the real Stripe shape, but it expands this change into a tax API and forces
  callers to create a tax rate before every invoice. The inline form is
  mock-friendly and still produces a Stripe-shaped invoice object.

### Invoice currency comes from the lines

The invoice `currency` is taken from the first line's price and every other
line must agree; a mismatch is an `invalid_request_error`. The invoice amount
fields are integer minor units throughout, matching `prices.unit_amount` and
`checkout_sessions.amount_total`. Tax is rounded half-up to the minor unit,
per line, before the totals are summed.

### Settle payments through a linked payment intent

A payment intent may reference an invoice (`payment_intents.invoice`). The
settlement rule lives in one place: resolving a payment intent as `succeeded`
adds its `amount` to the invoice's `amount_paid`, recomputes
`amount_remaining`, and marks the invoice `paid` once `amount_paid` reaches
`amount_due`. `POST /v1/invoices/{id}/pay` is a thin wrapper that creates a
payment intent for `amount_due`, links it to the invoice, and resolves it as
`succeeded` — so paying an invoice and paying it from a test take the same
code path and produce the same events.

- Alternative rejected: `pay` writing `amount_paid` directly. Two writers for
  the invoice totals means the "payment updates the invoice" rule can drift
  between them.
- Alternative rejected: a public `POST /v1/payment_intents/{id}/confirm`. There
  is no Stripe endpoint that settles a payment intent without a payment method,
  and the mock has no payment methods. Putting resolution on `/internal`
  keeps the public surface honest and gives the test a deterministic lever.

### Resolution is a control-plane endpoint

`POST /internal/payment_intents/{id}/resolve` takes `{"status": "succeeded" |
"failed" | "canceled"}` and requires no Stripe authentication, matching the
rest of `/internal/**`. It shares the cross-origin guard already applied to
that path group, so the existing safety property holds.

### Explicit invoice status transition table

Transitions are validated against a small table rather than with ad-hoc
checks: `draft → open` (finalize), `open → paid` (pay), `draft|open → void`
(void), `open → uncollectible` (mark uncollectible). A transition outside the
table is an `invalid_request_error` and leaves the invoice untouched. `paid`,
`void`, and `uncollectible` are terminal.

The table governs the four lifecycle endpoints. The settlement path is a
separate mechanism: resolving a payment intent as `succeeded` credits the
invoice it references and marks it `paid` once the balance is covered, without
consulting the table. A closed invoice (`paid`, `void`, `uncollectible`) is
never credited or reopened.

- Alternative rejected: allowing any transition to any non-draft status. It
  would let an application drive an invoice into states Stripe would never
  produce, which defeats the purpose of using the mock to test that flow.

### Customer tax ids live on the customer row

`tax_ids` is a JSON column on the `customers` row, holding entries of
`{id, object: "tax_id", type, value, created, livemode}`. Update appends to
the list; there is no removal endpoint in this change. When an invoice is
created, the customer's tax ids are copied to the invoice's `customer_tax_ids`,
so an invoice is self-contained.

- Alternative rejected: a `customer_tax_ids` table. Same reasoning as invoice
  lines — the ids are always read with the customer and never queried apart
  from it.

### Deleting a customer removes the row

`DELETE /v1/customers/{id}` removes the customer row, so subsequent reads
return `not_found`. Existing subscriptions and invoices keep their stored
`customer` id, as they do in Stripe after a customer is deleted.

### Additive schema changes with idempotent migrations

New columns (`customers.tax_ids`, `payment_intents.invoice`) and the new
`invoices` table are added to `SCHEMA`, and `migrate_legacy_columns` adds the
missing columns to a pre-existing database file with `ALTER TABLE ... ADD
COLUMN` guarded by `has_column`. `reset_state` gains the new tables so
`POST /internal/reset` clears them. This matches the existing migration
approach and keeps a checked-in `stripe-mock.db` usable.

### Event types and their emission points

| Event | Emitted when |
|---|---|
| `customer.created` / `customer.updated` / `customer.deleted` | customer created / updated / deleted |
| `invoice.created` | invoice created |
| `invoice.finalized` | invoice finalized |
| `invoice.paid` | invoice becomes `paid` |
| `invoice.payment_succeeded` | a payment intent for the invoice succeeded |
| `invoice.payment_failed` | a payment intent for the invoice failed |
| `invoice.voided` / `invoice.marked_uncollectible` | invoice voided / marked uncollectible |
| `payment_intent.created` | payment intent created |
| `payment_intent.succeeded` / `payment_intent.payment_failed` / `payment_intent.canceled` | payment intent resolved |

`payment_intent.succeeded` already exists for the checkout completion path;
the same type is reused here so an application can handle one event name for
both sources.

## Risks / Trade-offs

- [The invoice `number`, `due_date`, `collection_method`, and hosted page are
  absent, so an application reading those fields sees `null`] → They are
  documented as unsupported in `docs/invoicing.md`; the fields are omitted
  rather than faked, so a test cannot mistake a placeholder for real data.
- [Inline tax rates diverge from Stripe's tax rate resource] → Documented in
  `docs/invoicing.md`; a `/v1/tax_rates` resource can be added later without
  changing the invoice line shape.
- [The invoice `status` set omits `past_due` and `open`→`past_due` automatic
  transitions] → No server-side time advance exists in the mock, so
  `past_due` is unreachable by design.
- [A payment intent can be resolved more than once, e.g. `succeeded` then
  `failed`] → The second resolution overwrites the status and re-emits its
  event, but the invoice is only credited on a `succeeded` resolution; a
  failed resolution after a success leaves the invoice amounts as they were.
  This mirrors a mock being driven by hand and keeps the resolve endpoint
  idempotent in shape if not in effect.
- [Partial payments accumulate `amount_paid` but the invoice status only
  becomes `paid` at full coverage] → The remaining balance is visible in
  `amount_remaining`, which is the field an application would read.
- [New columns on `customers` and `payment_intents` do not exist in a
  database file created by an older build] → `migrate_legacy_columns` adds
  them on boot, and `SCHEMA` uses `CREATE TABLE IF NOT EXISTS` so a new
  database is unaffected.
