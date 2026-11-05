# Tasks

## 1. Give a finalized invoice a hosted page

- [x] 1.1 In `src/models.rs`, render the invoice with the configured base url and derive its `hosted_invoice_url` and its `invoice_pdf` from it, leaving both null while the invoice is a draft; verify with unit tests and with an integration test that a draft invoice has neither.
- [x] 1.2 In `src/api/invoicing.rs`, render every invoice with the base url of the running mock, in the responses and in the events; verify with the existing invoicing tests still passing under `cargo test`.

## 2. Serve the hosted page and its PDF

- [x] 2.1 In `src/mock.rs`, add a handler for `GET /mock/invoices/{id}` that renders the invoice — its id, its status, its lines, and its totals — in the shared page shell, and answers `404` for an invoice that does not exist; verify with an integration test that the page renders and names the invoice.
- [x] 2.2 In `src/mock.rs`, add a handler for `GET /mock/invoices/{id}/pdf` that serves a minimal one-page PDF with the invoice's summary, and register both routes in `src/lib.rs`; verify with an integration test that the PDF is served as `application/pdf` and starts with the PDF header.

## 3. Documentation

- [x] 3.1 Document the hosted page and the PDF in `docs/invoicing.md`, and drop the note that the mock has no hosted invoice page; verify by following the documented commands against a running mock.
- [x] 3.2 Document the hosted invoice page in `docs/hosted-ui.md`; verify that the page is reachable from the running mock.
- [x] 3.3 Extend the SDK end-to-end script in `scripts/sdk-e2e/e2e.js` with the hosted page and the PDF; verify with `scripts/sdk-e2e/run.sh`.
