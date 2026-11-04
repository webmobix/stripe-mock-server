# Design

## Context

Invoice lines already live in a single JSON column (`invoices.lines`) and the
amounts derived from them are stored in `invoices.subtotal`,
`invoices.tax_total`, `invoices.total`, `invoices.amount_due`, and
`invoices.amount_remaining`. `create_invoice` in `src/api/invoicing.rs` builds
each line with `build_invoice_line` (price × quantity, optional inline tax
rate) and inserts the totals alongside the line JSON. `transition_invoice`
guards its status change with a compare-and-swap `UPDATE ... WHERE status = ?`
and refuses an illegal transition before touching the row. Events are written
inside the same transaction as the change they describe and dispatched after
the commit (`events::emit` + `st.dispatcher.dispatch_event`).

The form parser (`src/form.rs`) already expands `lines[0][price]` and
`lines[][price]` into nested JSON, so the request shapes Stripe uses need no
parser work. The v1 error envelope (`src/error.rs`) offers
`invalid_request_error`, `invalid_param`, `missing_param`, and `not_found`,
which is everything these endpoints need.

## Goals / Non-Goals

**Goals:**

- Three endpoints that behave like Stripe's `add_lines`, `remove_lines`, and
  `update_lines` for a draft invoice.
- One code path for computing an invoice's totals, so a line change cannot
  leave the stored amounts out of step with the stored lines.
- The new endpoints and the relaxed creation rule stay inside the existing
  SQLite schema and the existing v1 auth, error, and event conventions.

**Non-Goals:**

- No `/v1/invoiceitems` resource, so `remove_lines` cannot detach a line onto
  a separate invoice item; the line simply leaves the invoice.
- No discounts, no proration, no billing periods, and no `period` input on
  the new endpoints.
- No support for editing a non-draft invoice: Stripe does not allow it either.
- No change to the invoice lifecycle, the payment intent settlement path, or
  the existing invoice events.

## Decisions

### The line list stays a JSON column

`invoices.lines` remains the single source of truth for an invoice's lines.
A separate `invoice_lines` table would buy nothing here: no endpoint lists
lines across invoices, and every read of an invoice already loads its whole
line array. The stored totals stay in their columns and are rewritten with
the lines.

Alternative considered: a `invoice_lines` table with one row per line. Rejected
— it duplicates the existing data, needs a migration for rows already in
`invoices.lines`, and buys no query the mock runs.

### One uniform line shape, with a nullable `price`

A line is rendered with the same fields whether it came from a catalog price
or from an explicit amount: `id`, `object`, `amount`, `currency`,
`description`, `quantity`, `period`, `price`, `proration`, `discountable`,
`tax_amounts`, `tax_rates`. A catalog line carries the price object; an
ad-hoc line carries `"price": null`, its own `amount`, and its optional
`description`. One shape means `remove_lines` and `update_lines` do not need
to branch on the kind of line, and a missing `price` key is read as ad-hoc
for rows written before this change.

Alternative considered: a tagged union with a `type` discriminator. Rejected —
it adds a field every consumer must understand for information the nullable
`price` already carries.

### Totals are always recomputed from the line list

A single helper derives `subtotal`, `tax_total`, and `total` by summing the
lines' `amount` and their `tax_amounts` amounts, and every write path —
creation, add, remove, update — recomputes before it writes. The tax of a
line is recomputed from the line's stored amount and the percentage held in
its `tax_rates[0].percentage`, which is why an update can change a tax rate
without storing anything new.

Alternative considered: applying incremental deltas (`total += added`). Rejected
— three mutations and a creation path that each adjust the totals by hand is
where drift between `lines` and `total` comes from.

### Draft-only, enforced before anything is mutated

Each handler loads the invoice (account-scoped, so an unknown id is a 404),
then checks that its `status` is `draft` before parsing the body or touching
the row. A non-draft invoice is rejected with an `invalid_request_error`
naming the invoice's status. The check is not a transition in the existing
state machine — a rejected line change leaves the invoice exactly as it was,
including its `updated` timestamp.

### The write is a compare-and-swap on the line list

The handler reads the invoice, computes the new line list, and writes it in
one transaction with
`UPDATE invoices SET ... WHERE id = ? AND status = 'draft' AND lines = ?`,
where the last bind is the line JSON it read. If no row is affected, another
request changed the invoice in between and the server answers with an
`invalid_request_error` asking the client to retry. This mirrors the
compare-and-swap `transition_invoice` already uses for status changes.

Alternative considered: a plain `UPDATE` inside the transaction, relying on
SQLite's write lock. Rejected — the read happens before the transaction, so
two concurrent `add_lines` calls could both read the same lines and the
second write would silently drop the first one's line.

### `invoice.updated` is the event for every line change

Each successful add, remove, or update emits `invoice.updated` in the same
transaction as the change, with the updated invoice as `data.object`, and
dispatches it after the commit like every other event in this module. A
rejected change emits nothing. `invoice.updated` is the name Stripe uses for
"the invoice changed", and it keeps the event catalogue to one new entry
rather than one per endpoint.

Alternative considered: `invoice.lines_added` and friends. Rejected — three
near-identical event types for one kind of change, and no way for a consumer
to write a single handler for "the invoice was edited".

### The request parameters mirror Stripe's names

- `add_lines`: `lines[]`, where an item is either `price` (+ optional
  `quantity`, optional `tax_rate[percentage]`, `tax_rate[name]`) or `amount`
  (+ optional `description`).
- `remove_lines`: `lines[][id]`, the `il_` id of the line to detach, plus an
  optional `lines[][behavior]` of `delete` or `unassign`.
- `update_lines`: `lines[][id]` plus the fields to change: `quantity`,
  `amount`, `description`, `price`, or `tax_rate[percentage]`.

`behavior` is accepted for compatibility and both values detach the line,
because the mock has no invoice item for `unassign` to leave behind; the
difference is not observable here and is documented in `docs/invoicing.md`.
An item with no updatable field is rejected rather than treated as a no-op.

### Errors follow the existing v1 envelope

| Condition | Response |
| --- | --- |
| Unknown invoice id for the account | `not_found` (404) |
| Invoice is not a draft | `invalid_request_error` (400) |
| Missing `lines` array | `invalid_request_error` naming `lines` |
| Line with neither `price` nor `amount` | `invalid_request_error` naming the line |
| Unknown price id | `invalid_param` on `lines[][price]` |
| Unknown line id in `remove_lines` / `update_lines` | `invalid_request_error` naming the line id |
| Line currency differs from the invoice currency | `invalid_request_error` |
| Non-integer or negative `amount` | `invalid_param` |

`create_invoice` keeps all of its current errors: a missing customer, an
unknown customer, and a missing `lines` array.

### An invoice always has a currency and at least one line

Invoice creation keeps its current rule: at least one line is required, and a
request without `lines` is rejected with the existing `missing_param` error.
The invoice's currency is fixed at creation — by the currency of its first
catalog-priced line, or `usd` when the invoice's only line is ad-hoc — and it
does not change when lines are added: adding a catalog line in another
currency is rejected, and an ad-hoc line always uses the invoice's currency.

Alternative considered: letting a later line set the currency of an invoice.
Rejected — the invoice currency would then depend on the order of the calls,
and the stored `currency` column would change under the client.

### Creation accepts the same line forms as `add_lines`

Creation and `add_lines` parse a line item the same way: a catalog price
with an optional quantity and inline tax rate, or an explicit amount with an
optional description. One parser keeps the two entry points consistent, so a
line built at creation and a line built by `add_lines` are the same object.

## Risks / Trade-offs

- [`invoice.updated` reaches every wildcard subscriber] → It is a genuine
  state change that a consumer may want, and the delivery path is unchanged;
  the new event type is listed in `docs/invoicing.md`.
- [Ad-hoc lines have a null `price`, so a client that reads `line.price.id`
  sees null] → Ad-hoc lines are opt-in; a client that only uses catalog
  prices is unaffected. Documented in `docs/invoicing.md`.
- [The compare-and-swap can report a spurious conflict under concurrent
  writes] → The error tells the client to retry, and SQLite's 5s busy timeout
  absorbs the contention that produces it.
- [The delta modifies requirements of the `invoicing` capability, which does
  not exist in `openspec/specs/` yet] → The `add-customer-invoicing` change is
  implemented but not archived; archiving it first creates the capability, and
  this change must be archived after it.

## Migration Plan

No database migration: the schema is unchanged and existing invoice rows keep
their stored lines and totals. No existing request changes behavior — invoice
creation still requires a customer and at least one line — and the three new
routes are purely additive. To roll back, revert the three routes and the
handlers in `src/api/invoicing.rs`; no stored data needs to be repaired.

## Open Questions

None. The remaining choices (which fields a line may carry, the `behavior`
values, and the event name) are settled above and do not change the specs.
