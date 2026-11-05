# Spec Delta

## ADDED Requirements

### Requirement: Hosted invoice page and PDF

The server SHALL give an invoice a `hosted_invoice_url` and an `invoice_pdf` once the invoice is no longer a `draft`, built from the configured `public_base_url`, and SHALL leave both null while the invoice is a draft. The server SHALL serve the hosted page at `GET /mock/invoices/{id}` and the PDF at `GET /mock/invoices/{id}/pdf`, and SHALL answer `not_found` for an invoice that does not exist.

#### Scenario: A finalized invoice has a hosted page

- **WHEN** a client finalizes an invoice
- **THEN** the invoice `hosted_invoice_url` is `{public_base_url}/mock/invoices/{id}` and its `invoice_pdf` is `{public_base_url}/mock/invoices/{id}/pdf`

#### Scenario: A draft invoice has no hosted page

- **WHEN** a client creates an invoice
- **THEN** the invoice `hosted_invoice_url` and its `invoice_pdf` are null

#### Scenario: The hosted page describes the invoice

- **WHEN** a browser opens the hosted page of an invoice
- **THEN** an HTML page renders with the invoice's id, its status, its lines, and its totals

#### Scenario: The PDF is a document

- **WHEN** a client requests the PDF of an invoice
- **THEN** the response is a PDF document holding the invoice's summary
