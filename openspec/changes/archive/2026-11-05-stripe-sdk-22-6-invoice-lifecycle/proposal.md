# Proposal

## Why

The Stripe SDK 22.6.x defaults to API version `2026-08-26.dahlia`, in which an
invoice is created empty and its lines are added afterwards. The mock rejects
that flow: `POST /v1/invoices` insists on a `lines` array and answers
`400 Missing required param: lines`, so an application that follows the SDK's
own sequence — `invoices.create`, then `invoices.addLines` — never gets past
the first call.

Two more gaps follow from the same SDK contract. The invoice's currency was
derived from the first catalog-priced line, which no longer exists when the
invoice is created empty, and the SDK's `invoices.del` has no route at all, so
an application cannot roll back a draft it no longer needs.

## What Changes

- **Invoice creation no longer requires lines.** `POST /v1/invoices` accepts a
  request with a `customer` and no `lines`, and answers with an empty draft
  whose `lines.data` is empty and whose `subtotal`, `tax_total`, and `total`
  are 0. A `lines` array is still accepted and still priced, so the older
  one-shot flow keeps working.
- **The currency comes from the request.** The invoice's currency is fixed at
  creation: the request's `currency`, then the customer's currency, then `usd`.
  It never follows the lines, so an invoice created before it has any can still
  be priced correctly. A catalog price in another currency than the invoice is
  rejected, exactly as it already is on `add_lines`.
- **Customers carry a currency.** `POST /v1/customers` and
  `POST /v1/customers/{id}` accept a `currency`, and the customer exposes it,
  so it can be the fallback for the invoices raised against it.
- **`DELETE /v1/invoices/{id}`.** A draft invoice can be deleted; the invoice
  is removed and the tombstone Stripe returns (`id`, `object`, `deleted`) is
  given back. An invoice that is not a draft is rejected, since it has to be
  voided instead. The deletion emits an `invoice.deleted` event.
- **Finalizing needs a line.** `POST /v1/invoices/{id}/finalize` rejects an
  invoice that has no lines, because the lines may have been added after the
  invoice was created and are only known at that point.

No other invoice behavior changes: the customer is still required, the line
forms and the totals are unchanged, the line editing endpoints still work on
draft invoices only, and the lifecycle (`finalize`, `pay`, `void`,
`mark_uncollectible`) and the payment intent settlement path are untouched.

## Capabilities

### New Capabilities

None. This change extends the `invoicing` and `billing` capabilities.

### Modified Capabilities

- `invoicing`: an invoice can be created without lines, its currency comes
  from the request or the customer, a draft invoice can be deleted, and an
  invoice with no lines cannot be finalized.
- `billing`: a customer carries a currency that an invoice can fall back to.

## Impact

- **Code.** `src/api/invoicing.rs` rewrites the currency resolution in
  `create_invoice`, adds `delete_invoice`, and guards `finalize_invoice`;
  `src/api/v1.rs` accepts a `currency` on the customer endpoints;
  `src/lib.rs` registers the `DELETE` route; `src/models.rs` renders the
  customer's currency.
- **Database.** `customers` gains a nullable `currency` column, added by the
  existing idempotent column migration. Existing rows keep a null currency and
  keep working; no invoice row changes shape.
- **API surface.** One new `DELETE /v1/invoices/{id}` route. Invoice creation
  stops rejecting a request without lines.
- **Events.** `invoice.deleted` is a new event type; webhook destinations that
  filter on `invoice.*` or `*` start receiving it.
- **Tests.** `tests/invoicing.rs` covers the empty invoice, the currency
  precedence, the deletion, and the finalize guard.
- **Docs.** `docs/invoicing.md`.
