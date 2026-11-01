# Spec Delta

## Purpose

Lets an application raise invoices against a customer, price them from the catalog with optional tax, and move each invoice through its real lifecycle (draft, open, paid, void, uncollectible).

## ADDED Requirements

### Requirement: Create and retrieve invoices

The server SHALL create invoices at `POST /v1/invoices` for a `customer` with at least one line, and SHALL retrieve them at `GET /v1/invoices/{id}`. A new invoice has an `in_` id, `object` of `invoice`, `status` of `draft`, a `currency`, a `subtotal`, a `total`, and a `lines` list.

#### Scenario: Create invoice with one line

- **WHEN** a client creates an invoice with a `customer` and `lines[0][price]` referencing a price
- **THEN** the response is an invoice with a `in_` id, `status` of `draft`, and `lines.data` containing that line with its `amount`

#### Scenario: Create invoice without a customer

- **WHEN** a client creates an invoice without a `customer`
- **THEN** the server responds with an `invalid_request_error` naming `customer`

#### Scenario: Unknown customer

- **WHEN** a client creates an invoice with a `customer` that does not exist for the account
- **THEN** the server responds with a `not_found` error and creates no invoice

### Requirement: Invoice line items and totals

The server SHALL derive each invoice line from a catalog price and a `quantity`, and SHALL compute `subtotal` as the sum of the line amounts, `tax_total` as the sum of the line tax amounts, and `total` as `subtotal` plus `tax_total`.

#### Scenario: Line amount from price and quantity

- **WHEN** an invoice line references a price of `unit_amount` 2000 with `quantity` 3
- **THEN** the line `amount` is 6000 and the invoice `subtotal` is 6000

#### Scenario: Default quantity

- **WHEN** an invoice line omits `quantity`
- **THEN** the line `quantity` is 1

### Requirement: Invoice tax amounts

A client MAY attach an inline tax rate to an invoice line with `lines[][tax_rate][percentage]`. The server SHALL compute the line `tax_amounts` from that percentage, SHALL expose `tax_total` on the invoice, and SHALL include the tax rate details with each amount.

#### Scenario: Line with a tax rate

- **WHEN** an invoice line of amount 1000 carries a tax rate of `10` percent
- **THEN** the line has a `tax_amounts` entry of amount 100 and the invoice `tax_total` is 100

#### Scenario: Tax total in the invoice total

- **WHEN** an invoice has a subtotal of 1000 and a tax total of 100
- **THEN** the invoice `total` is 1100

### Requirement: Customer tax identifiers on invoices

The server SHALL copy the tax identifiers of the invoice's customer to the invoice `customer_tax_ids`, so a rendered invoice carries the customer tax details.

#### Scenario: Invoice carries customer tax ids

- **WHEN** a customer has a tax id of type `eu_vat` and an invoice is created for that customer
- **THEN** the invoice `customer_tax_ids` contains that tax id with its `type` and `value`

### Requirement: List invoices and lines

The server SHALL list invoices at `GET /v1/invoices` with the list envelope, filtering by `customer` and `status` where provided, and SHALL list the lines of one invoice at `GET /v1/invoices/{id}/lines`.

#### Scenario: List invoices for a customer

- **WHEN** a client lists invoices with `customer=<cus_...>`
- **THEN** only that customer's invoices are returned

#### Scenario: Invoice lines envelope

- **WHEN** a client requests the lines of an invoice
- **THEN** the response is a list envelope whose `data` holds the invoice's lines

### Requirement: Finalize an invoice

The server SHALL finalize a draft invoice at `POST /v1/invoices/{id}/finalize`, moving it to `status` of `open`, setting `finalized_at`, and setting `amount_due` to the invoice `total`. The server SHALL create an `invoice.finalized` event.

#### Scenario: Finalize a draft invoice

- **WHEN** a client finalizes a draft invoice
- **THEN** the invoice `status` is `open`, `amount_due` equals the `total`, and `finalized_at` is set

#### Scenario: Finalize a non-draft invoice

- **WHEN** a client finalizes an invoice that is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice status is unchanged

### Requirement: Pay an invoice

The server SHALL pay an open invoice at `POST /v1/invoices/{id}/pay` by creating a payment intent for the invoice `amount_due` and settling it as `succeeded`. The server SHALL set the invoice `status` to `paid`, `amount_paid` to the `amount_due`, `amount_remaining` to 0, and `paid_at`, and SHALL create `invoice.payment_succeeded` and `invoice.paid` events.

#### Scenario: Pay an open invoice

- **WHEN** a client pays an open invoice
- **THEN** the invoice `status` is `paid`, `amount_paid` equals `amount_due`, and a succeeded `pi_` payment intent references the invoice

#### Scenario: Pay an invoice that is not open

- **WHEN** a client pays an invoice that is not `open`
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

### Requirement: Void an invoice

The server SHALL void a draft or open invoice at `POST /v1/invoices/{id}/void`, moving it to `status` of `void` and setting `voided_at`. The server SHALL create an `invoice.voided` event.

#### Scenario: Void an open invoice

- **WHEN** a client voids an open invoice
- **THEN** the invoice `status` is `void` and `voided_at` is set

### Requirement: Mark an invoice uncollectible

The server SHALL mark an open invoice uncollectible at `POST /v1/invoices/{id}/mark_uncollectible`, moving it to `status` of `uncollectible`. The server SHALL create an `invoice.marked_uncollectible` event.

#### Scenario: Mark an open invoice uncollectible

- **WHEN** a client marks an open invoice uncollectible
- **THEN** the invoice `status` is `uncollectible`

#### Scenario: Mark a paid invoice uncollectible

- **WHEN** a client marks an invoice that is not `open` uncollectible
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

### Requirement: Invoice creation event

The server SHALL create an `invoice.created` event in the same transaction as the invoice it describes.

#### Scenario: Invoice created event

- **WHEN** a client creates an invoice
- **THEN** an `invoice.created` event exists whose `data.object` is that invoice
