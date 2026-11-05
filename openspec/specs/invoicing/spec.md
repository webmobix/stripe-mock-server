# invoicing Specification

## Purpose

Lets an application raise invoices against a customer, price them from the catalog with optional tax, and move each invoice through its real lifecycle (draft, open, paid, void, uncollectible).

## Requirements

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

### Requirement: Add lines to a draft invoice

The server SHALL add line items to a draft invoice at `POST /v1/invoices/{id}/add_lines`, given a `lines` array whose items are catalog-priced or ad-hoc, and SHALL return the updated invoice. The server SHALL recompute the invoice totals from the resulting lines.

#### Scenario: Add a catalog-priced line

- **WHEN** a client adds a line with `lines[0][price]` referencing a price of `unit_amount` 1000 and `lines[0][quantity]` 2 to a draft invoice
- **THEN** the invoice `lines.data` holds the new line with an `amount` of 2000 and the invoice `subtotal` and `total` include it

#### Scenario: Add an ad-hoc line

- **WHEN** a client adds a line with `lines[0][amount]` 500 and `lines[0][description]` "Setup fee" to a draft invoice
- **THEN** the invoice `lines.data` holds the new line with an `amount` of 500 and the invoice `total` grows by 500

#### Scenario: Add a line with an inline tax rate

- **WHEN** a client adds a line with `lines[0][price]` and `lines[0][tax_rate][percentage]` 10 to a draft invoice
- **THEN** the new line has a `tax_amounts` entry and the invoice `tax_total` grows by that tax

#### Scenario: Add lines to a non-draft invoice

- **WHEN** a client adds lines to an invoice whose `status` is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

#### Scenario: Add lines without a lines parameter

- **WHEN** a client calls the endpoint without a `lines` array
- **THEN** the server responds with an `invalid_request_error` naming `lines`

#### Scenario: Add a line with an unknown price

- **WHEN** a client adds a line whose `price` does not exist for the account
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

#### Scenario: Add a line in another currency

- **WHEN** a client adds a catalog-priced line whose price currency differs from the invoice currency
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

### Requirement: Remove lines from a draft invoice

The server SHALL remove line items from a draft invoice at `POST /v1/invoices/{id}/remove_lines`, given a `lines` array whose items name a line `id`, and SHALL return the updated invoice. The server SHALL recompute the invoice totals from the remaining lines.

#### Scenario: Remove a line

- **WHEN** a client removes the line whose `id` is on a draft invoice
- **THEN** that line is gone from the invoice `lines.data` and the invoice `subtotal` and `total` no longer include its amount

#### Scenario: Remove every line

- **WHEN** a client removes every line of a draft invoice
- **THEN** the invoice `lines.data` is empty and the invoice `subtotal`, `tax_total`, and `total` are 0

#### Scenario: Remove a line that is not on the invoice

- **WHEN** a client removes a line `id` that the invoice does not carry
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

#### Scenario: Remove lines from a non-draft invoice

- **WHEN** a client removes lines from an invoice whose `status` is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

### Requirement: Update lines on a draft invoice

The server SHALL update line items on a draft invoice at `POST /v1/invoices/{id}/update_lines`, given a `lines` array whose items name a line `id` and the fields to change, and SHALL return the updated invoice.

#### Scenario: Update the quantity of a line

- **WHEN** a client updates a catalog-priced line with `lines[0][id]` and `lines[0][quantity]` 3
- **THEN** the line `quantity` is 3, its `amount` is recomputed from the price, and the invoice `total` follows

#### Scenario: Update the description of a line

- **WHEN** a client updates a line with `lines[0][id]` and `lines[0][description]` "Revised"
- **THEN** the line `description` is "Revised" and the invoice `total` is unchanged

#### Scenario: Update the amount of a line

- **WHEN** a client updates a line with `lines[0][id]` and `lines[0][amount]` 250
- **THEN** the line `amount` is 250 and the invoice `subtotal` and `total` follow

#### Scenario: Update the tax rate of a line

- **WHEN** a client updates a line with `lines[0][id]` and `lines[0][tax_rate][percentage]` 20
- **THEN** the line `tax_amounts` is recomputed from the new percentage and the invoice `tax_total` follows

#### Scenario: Update a line that is not on the invoice

- **WHEN** a client updates a line `id` that the invoice does not carry
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

#### Scenario: Update lines on a non-draft invoice

- **WHEN** a client updates lines on an invoice whose `status` is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

### Requirement: Draft invoice totals follow its lines

The server SHALL derive the `subtotal`, `tax_total`, `total`, `amount_due`, and `amount_remaining` of a draft invoice from its lines after every line change, and SHALL move the invoice `updated` timestamp.

#### Scenario: Totals after adding a line

- **WHEN** a line of amount 1000 is added to a draft invoice whose `total` was 500
- **THEN** the invoice `total` is 1500, its `amount_due` is 1500, and its `amount_remaining` is 1500

#### Scenario: Totals after removing a line

- **WHEN** a line of amount 1000 is removed from a draft invoice whose `total` was 1500
- **THEN** the invoice `total` is 500, its `amount_due` is 500, and its `amount_remaining` is 500

#### Scenario: Totals after updating a line

- **WHEN** the quantity of a line of amount 1000 is updated so that the line `amount` becomes 2000
- **THEN** the invoice `subtotal` and `total` grow by 1000

#### Scenario: Tax total follows the lines

- **WHEN** a line carrying a 10 percent tax rate of amount 1000 is added to a draft invoice
- **THEN** the invoice `tax_total` is 100 and its `total` is `subtotal` plus 100

### Requirement: Invoice updated event

The server SHALL create an `invoice.updated` event, in the same transaction as the change that caused it, whenever a client adds, removes, or updates the lines of an invoice.

#### Scenario: Lines added

- **WHEN** a client adds lines to a draft invoice
- **THEN** an `invoice.updated` event exists whose `data.object` is the invoice with the added lines

#### Scenario: Lines removed

- **WHEN** a client removes lines from a draft invoice
- **THEN** an `invoice.updated` event exists whose `data.object` is the invoice without the removed lines

#### Scenario: Lines updated

- **WHEN** a client updates lines on a draft invoice
- **THEN** an `invoice.updated` event exists whose `data.object` is the invoice with the updated lines

#### Scenario: Rejected line change

- **WHEN** a client attempts a line change on a non-draft invoice and the server rejects it
- **THEN** no `invoice.updated` event is created

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

#### Scenario: Finalize an invoice with no lines

- **WHEN** a client finalizes a draft invoice that has no lines
- **THEN** the server responds with an `invalid_request_error` and the invoice stays a `draft`

#### Scenario: Finalize a non-draft invoice

- **WHEN** a client finalizes an invoice that is not `draft`
- **THEN** the server responds with an `invalid_request_error` and the invoice status is unchanged

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
