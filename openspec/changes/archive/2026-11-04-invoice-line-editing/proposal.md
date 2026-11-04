# Proposal

## Why

An application that builds an invoice in steps — open a draft, add the
consulting hours, add a licence, correct a quantity, drop the line that was
never approved — cannot be tested against the mock. Lines are only accepted
when the invoice is created, so a draft invoice can never grow, shrink, or
change after the fact. Stripe exposes `POST /v1/invoices/{id}/add_lines` (and
the sibling `remove_lines` and `update_lines`) for exactly this workflow, and
the mock answers those calls with a 404 because the routes do not exist.

## What Changes

- **Three new invoice line endpoints.** `POST /v1/invoices/{id}/add_lines`,
  `POST /v1/invoices/{id}/remove_lines`, and `POST /v1/invoices/{id}/update_lines`
  let a client edit the lines of a draft invoice. All three are restricted to
  `draft` invoices, mirroring Stripe: a non-draft invoice is rejected with an
  `invalid_request_error` and is left untouched.
- **Invoice creation keeps requiring at least one line.** `POST /v1/invoices`
  still rejects a request with no `lines` with the existing `missing_param`
  error. An invoice is therefore always created with a line, and `add_lines`
  appends to it.
- **Ad-hoc line items.** A line may be priced from the catalog
  (`lines[][price]`, optional `lines[][quantity]`, optional inline
  `lines[][tax_rate]`) or written directly (`lines[][amount]` with an optional
  `lines[][description]`), as on Stripe's bulk line API. Ad-hoc lines use the
  invoice's currency. Both forms are accepted at creation and on `add_lines`.
- **Totals are recomputed after every line change.** `subtotal`, `tax_total`,
  `total`, `amount_due`, and `amount_remaining` are derived from the invoice's
  lines after each add, remove, or update, and the invoice's `updated`
  timestamp moves.
- **A new `invoice.updated` event.** Every successful line change emits
  `invoice.updated` with the updated invoice, so a webhook consumer can follow
  an invoice being assembled. This adds an event type to the mock's catalogue.
- **Documentation.** `docs/invoicing.md` gains the three endpoints, the ad-hoc
  line form, and the `invoice.updated` event.

No other invoice behavior changes: invoice creation still requires a customer
and at least one line, and the lifecycle (`finalize`, `pay`, `void`,
`mark_uncollectible`), the payment intent settlement path, and the webhook
delivery of the existing invoice events are untouched.

## Capabilities

### New Capabilities

None. This change extends the `invoicing` capability that the in-flight
`add-customer-invoicing` change introduces; that change must be archived first
so this delta has a baseline to modify.

### Modified Capabilities

- `invoicing`: invoice lines may be ad-hoc as well as catalog-priced, and the
  lines of a draft invoice can be added, removed, and updated through the new
  bulk endpoints, each of which emits `invoice.updated`.

## Impact

- **Code.** `src/api/invoicing.rs` gains the three handlers plus the shared
  line-building and totals-recomputation helpers; `src/lib.rs` gains the three
  routes; `src/models.rs` may need to render ad-hoc line fields.
- **Database.** No schema change: invoice lines are already stored as a JSON
  array in `invoices.lines`, and the recomputed amounts fit the existing
  columns. Existing rows keep their current shape.
- **API surface.** Three new `POST` routes under `/v1/invoices/{id}`. Invoice
  creation keeps its current validation, including the requirement of at least
  one line.
- **Events.** `invoice.updated` is a new event type; webhook destinations that
  filter on `invoice.*` or `*` start receiving it.
- **Tests.** `tests/invoicing.rs` covers the new endpoints and the
  `invoice.updated` event.
- **Docs.** `docs/invoicing.md`.
