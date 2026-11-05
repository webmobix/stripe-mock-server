# Spec Delta

## MODIFIED Requirements

### Requirement: Create and retrieve invoices

The server SHALL create invoices at `POST /v1/invoices` for a `customer`, with or without lines, and SHALL retrieve them at `GET /v1/invoices/{id}`. A new invoice has an `in_` id, `object` of `invoice`, `status` of `draft`, a `currency`, a `subtotal`, a `total`, and a `lines` list. An invoice created without lines is an empty draft: its `lines.data` is empty and its `subtotal`, `tax_total`, and `total` are 0.

#### Scenario: Create invoice with one line

- **WHEN** a client creates an invoice with a `customer` and `lines[0][price]` referencing a price
- **THEN** the response is an invoice with a `in_` id, `status` of `draft`, and `lines.data` containing that line with its `amount`

#### Scenario: Create invoice without any line

- **WHEN** a client creates an invoice with a `customer` and no `lines`
- **THEN** the response is a draft invoice whose `lines.data` is empty and whose `subtotal`, `tax_total`, and `total` are 0

#### Scenario: Create invoice without a customer

- **WHEN** a client creates an invoice without a `customer`
- **THEN** the server responds with an `invalid_request_error` naming `customer`

#### Scenario: Unknown customer

- **WHEN** a client creates an invoice with a `customer` that does not exist for the account
- **THEN** the server responds with a `not_found` error and creates no invoice

### Requirement: Invoice line items and totals

The server SHALL derive each invoice line from a catalog price and a `quantity`, or from an explicit `amount`, and SHALL compute `subtotal` as the sum of the line amounts, `tax_total` as the sum of the line tax amounts, and `total` as `subtotal` plus `tax_total`. Invoice creation SHALL accept either form of line and SHALL NOT require a line.

#### Scenario: Line amount from price and quantity

- **WHEN** an invoice line references a price of `unit_amount` 2000 with `quantity` 3
- **THEN** the line `amount` is 6000 and the invoice `subtotal` is 6000

#### Scenario: Default quantity

- **WHEN** an invoice line omits `quantity`
- **THEN** the line `quantity` is 1

#### Scenario: Ad-hoc line with an amount and a description

- **WHEN** an invoice line carries `amount` 799 and `description` "Consulting"
- **THEN** the line `amount` is 799, its `description` is "Consulting", and it is priced in the invoice's `currency`

#### Scenario: Ad-hoc line without a description

- **WHEN** an invoice line carries an `amount` and no `description`
- **THEN** the line `description` is null and its `amount` is the given amount

#### Scenario: Line with neither a price nor an amount

- **WHEN** an invoice line carries neither a `price` nor an `amount`
- **THEN** the server responds with an `invalid_request_error` and no invoice line is created

#### Scenario: A line priced in another currency

- **WHEN** a client creates an invoice whose currency differs from the currency of a line's price
- **THEN** the server responds with an `invalid_request_error` and creates no invoice

### Requirement: Finalize an invoice

The server SHALL finalize a draft invoice at `POST /v1/invoices/{id}/finalize`, moving it to `status` of `open`, setting `finalized_at`, and setting `amount_due` to the invoice `total`. The server SHALL create an `invoice.finalized` event.

#### Scenario: Finalize a draft invoice

- **WHEN** a client finalizes a draft invoice
- **THEN** the invoice `status` is `open`, `amount_due` equals the `total`, and `finalized_at` is set

#### Scenario: Finalize an invoice with no lines

- **WHEN** a client finalizes a draft invoice that has no lines
- **THEN** the server responds with an `invalid_request_error` and the invoice stays a `draft`

#### Scenario: Finalize a non-draft invoice

- **WHEN** a client finalizes an invoice that is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice status is unchanged

## ADDED Requirements

### Requirement: Invoice currency

The server SHALL fix an invoice's currency when the invoice is created, and SHALL not change it afterwards. The currency is the `currency` parameter of the request when the request carries one, otherwise the currency of the customer the invoice is raised against, otherwise `usd`. The server SHALL reject a `currency` that is not a three-letter ISO code.

#### Scenario: Currency from the request

- **WHEN** a client creates an invoice with a `currency` of `eur`
- **THEN** the invoice `currency` is `eur` and its lines are priced in `eur`

#### Scenario: Currency from the customer

- **WHEN** a client creates an invoice for a customer whose currency is `eur` and names no `currency`
- **THEN** the invoice `currency` is `eur`

#### Scenario: Currency falls back to usd

- **WHEN** a client creates an invoice for a customer with no currency and names no `currency`
- **THEN** the invoice `currency` is `usd`

#### Scenario: Invalid currency

- **WHEN** a client creates an invoice with a `currency` that is not a three-letter ISO code
- **THEN** the server responds with an `invalid_request_error` naming `currency` and creates no invoice

### Requirement: Delete a draft invoice

The server SHALL delete a draft invoice at `DELETE /v1/invoices/{id}`, removing it from subsequent reads and lists, and SHALL return a tombstone holding the invoice `id`, an `object` of `invoice`, and `deleted` of true. The server SHALL reject the deletion of an invoice that is not a draft, because such an invoice has to be voided instead, and SHALL create an `invoice.deleted` event.

#### Scenario: Delete a draft invoice

- **WHEN** a client deletes a draft invoice
- **THEN** the response is a tombstone with the invoice `id` and `deleted` of true, retrieving that invoice returns a `not_found` error, and the invoice is gone from the invoice list

#### Scenario: Delete an invoice that is not a draft

- **WHEN** a client deletes an invoice whose status is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

#### Scenario: Delete an unknown invoice

- **WHEN** a client deletes an invoice that does not exist for the account
- **THEN** the server responds with a `not_found` error
