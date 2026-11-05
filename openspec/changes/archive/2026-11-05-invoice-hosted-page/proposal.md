# Proposal

## Why

A finalized invoice has no hosted page: the mock hardcodes
`"hosted_invoice_url": null` on every invoice, so an application that sends its
customer to the invoice's hosted page — the usual way to collect payment — has
nothing to redirect to. Real Stripe publishes the page and a PDF of the
invoice once the invoice is open, and an application written against Stripe
treats the missing URL as a failure rather than as a draft invoice.

## What Changes

- **A finalized invoice has a hosted page.** Once the invoice leaves the
  `draft` status, its `hosted_invoice_url` and its `invoice_pdf` are built from
  the configured `public_base_url`: `{public_base_url}/mock/invoices/{id}` and
  `{public_base_url}/mock/invoices/{id}/pdf`. A draft invoice keeps both
  fields null, as on Stripe, where the page is published on finalization.
- **The hosted page is served.** `GET /mock/invoices/{id}` renders the invoice
  the way the other mock pages render: its id, its status, its lines, and its
  totals. `GET /mock/invoices/{id}/pdf` serves a minimal one-page PDF with the
  same summary. Both answer `404` for an invoice that does not exist.
- **The invoice's rendering takes the base url.** The invoice object is
  rendered with the configured base url, so the two links it carries point at
  the running mock.

No other invoice behavior changes: the invoice's amounts, its lines, its
lifecycle, and its events are untouched.

## Capabilities

### New Capabilities

None. This change extends the `invoicing` and `hosted-simulation-ui`
capabilities.

### Modified Capabilities

- `invoicing`: a finalized invoice carries a `hosted_invoice_url` and an
  `invoice_pdf`.
- `hosted-simulation-ui`: the mock serves the hosted page of an invoice and
  its PDF.

## Impact

- **Code.** `src/models.rs` renders the invoice with the base url and derives
  the two links; `src/mock.rs` gains the hosted page and the PDF;
  `src/lib.rs` registers the two routes.
- **Database.** No schema change.
- **API surface.** Two new routes under `/mock/invoices`; the invoice object
  gains the `invoice_pdf` field and a `hosted_invoice_url` that is no longer
  always null.
- **Events.** No new event type.
- **Tests.** `tests/invoicing.rs` covers the hosted url, the page, and the
  PDF.
- **Docs.** `docs/invoicing.md` and `docs/hosted-ui.md`.
