# Spec Delta

## ADDED Requirements

### Requirement: Hosted invoice page

The mock SHALL serve the hosted page of an invoice at `GET /mock/invoices/{id}` and a PDF of the same summary at `GET /mock/invoices/{id}/pdf`, so an application can send its customer to the invoice's `hosted_invoice_url`. The page SHALL render the invoice's id, its status, its lines, and its totals, and both SHALL answer `not_found` for an invoice that does not exist.

#### Scenario: The page renders the invoice

- **WHEN** a browser opens the hosted page of an invoice
- **THEN** the page shows the invoice's id, its status, its lines, and its totals

#### Scenario: The PDF is served

- **WHEN** a client requests the PDF of an invoice
- **THEN** the response is a PDF document holding the invoice's summary
