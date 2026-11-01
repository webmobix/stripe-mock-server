# Proposal

## Why

Applications that bill customers end-to-end need customers with tax details,
invoices that move through their real lifecycle, and a payment intent whose
resolution updates the invoice. The mock today stops at customers
(create/retrieve only, no tax) and subscriptions, and its `payment_intents`
table has no API surface at all — so a customer's own invoice flow cannot be
exercised against the mock. Invoices and tax are explicitly listed as deferred
work in the README, and this change delivers the minimum that lets an
application be tested end to end: create a customer with tax information, raise
an invoice, move it through every stage, pay it with a payment intent, and
observe the minimum customer and invoice webhooks.

## What Changes

- **Customer surface completed.** `GET /v1/customers` (list), `POST
  /v1/customers/{id}` (update), and `DELETE /v1/customers/{id}` join the
  existing create/retrieve endpoints. Customers accept `tax_id_data[][type]`
  and `tax_id_data[][value]` on create and update, and expose them as
  `customer.tax_ids`. Customer changes emit `customer.created`,
  `customer.updated`, and `customer.deleted`.
- **New invoicing capability.** `POST /v1/invoices`, `GET /v1/invoices`,
  `GET /v1/invoices/{id}`, and `GET /v1/invoices/{id}/lines` create, list, and
  read invoices. Invoice lines are priced from the catalog (`lines[][price]`,
  `lines[][quantity]`) and may carry an inline tax rate
  (`lines[][tax_rate][percentage]`), which the invoice turns into per-line
  `tax_amounts` and a `total_tax_amounts` summary.
- **Invoice lifecycle.** `POST /v1/invoices/{id}/finalize` moves a draft to
  `open`, `POST /v1/invoices/{id}/pay` moves an open invoice to `paid`,
  `POST /v1/invoices/{id}/void` moves it to `void`, and `POST
  /v1/invoices/{id}/mark_uncollectible` moves it to `uncollectible`. Invalid
  transitions are rejected with the v1 `invalid_request_error` envelope.
- **New payments capability.** `POST /v1/payment_intents`, `GET
  /v1/payment_intents`, and `GET /v1/payment_intents/{id}` expose the payment
  intent resource that today exists only as a table. A payment intent can be
  created with `invoice` to attach it to an invoice.
- **Payment intent resolution.** `POST /internal/payment_intents/{id}/resolve`
  settles a payment intent as `succeeded`, `failed`, or `canceled`. A successful
  payment intent attached to an invoice adds its amount to the invoice's
  `amount_paid`, marks the invoice `paid` once the invoice is fully covered,
  and clears `amount_remaining`.
- **Minimum customer and invoice webhooks.** The mock emits `customer.created`,
  `customer.updated`, `customer.deleted`, `invoice.created`,
  `invoice.finalized`, `invoice.paid`, `invoice.payment_succeeded`,
  `invoice.payment_failed`, `invoice.voided`,
  `invoice.marked_uncollectible`, `payment_intent.created`,
  `payment_intent.succeeded`, `payment_intent.payment_failed`, and
  `payment_intent.canceled`.
- **Documentation.** `docs/billing.md` gains the customer list/update/delete
  and tax sections; a new `docs/invoicing.md` covers invoices, payment
  intents, and the resolve endpoint.

No breaking changes: existing customer, subscription, and checkout behavior is
unchanged, and the new customer fields default to empty.

## Capabilities

### New Capabilities

- `invoicing`: Creating, listing, reading, and staging invoices (lines, tax
  amounts, status transitions) for a customer.
- `payments`: Creating, listing, and reading payment intents, and settling
  them so the invoices they are attached to update.

### Modified Capabilities

- `billing`: The customer surface gains list, update, and delete, plus tax
  identification data and the customer lifecycle events.
- `internal-control-plane`: Gains the payment intent resolve endpoint used to
  settle a payment from a test.
- `api-foundations`: The generated-identifier list gains the `in_` invoice
  prefix and the `txi_` tax id prefix.

## Impact

- `src/db.rs` — schema for `invoices`, new `customers` columns, a new
  `payment_intents.invoice` column, plus the idempotent column migrations for
  pre-existing database files and the `reset_state` table list.
- `src/models.rs` — `CustomerRow`, `InvoiceRow`, and `PaymentIntentRow`
  projections.
- `src/ids.rs` — `invoice_id()` and `tax_id()` generators.
- `src/api/v1.rs` — customer list/update/delete, the invoice endpoints, and
  the payment intent endpoints.
- `src/internal.rs` — `POST /internal/payment_intents/{id}/resolve`.
- `src/lib.rs` — routes for the new endpoints.
- `docs/billing.md`, `docs/invoicing.md`, `README.md` — documentation.
- `tests/` — integration coverage for the customer, invoice, and payment
  intent flows.
- No new dependencies.
