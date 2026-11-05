# Spec Delta

## MODIFIED Requirements

### Requirement: Invoice line items and totals

The server SHALL derive each invoice line from a catalog price and a `quantity`, from an inline `price_data`, or from an explicit `amount`, and SHALL compute `subtotal` as the sum of the line amounts, `tax_total` as the sum of the line tax amounts, and `total` as `subtotal` plus `tax_total`. Invoice creation SHALL accept any of the three forms of line and SHALL NOT require a line.

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

- **WHEN** an invoice line carries neither a `price`, nor a `price_data`, nor an `amount`
- **THEN** the server responds with an `invalid_request_error` and no invoice line is created

#### Scenario: A line priced in another currency

- **WHEN** a client creates an invoice whose currency differs from the currency of a line's price
- **THEN** the server responds with an `invalid_request_error` and creates no invoice

### Requirement: Add lines to a draft invoice

The server SHALL add line items to a draft invoice at `POST /v1/invoices/{id}/add_lines`, given a `lines` array whose items are catalog-priced, inline-priced, or ad-hoc, and SHALL return the updated invoice. The server SHALL recompute the invoice totals from the resulting lines.

#### Scenario: Add a catalog-priced line

- **WHEN** a client adds a line with `lines[0][price]` referencing a price of `unit_amount` 1000 and `lines[0][quantity]` 2 to a draft invoice
- **THEN** the invoice `lines.data` holds the new line with an `amount` of 2000 and the invoice `subtotal` and `total` include it

#### Scenario: Add a line with an inline price

- **WHEN** a client adds a line with `lines[0][price_data][currency]` of `usd`, `lines[0][price_data][unit_amount]` of 2500, `lines[0][price_data][product_data][name]` of "Consulting", and `lines[0][quantity]` of 2 to a draft invoice
- **THEN** the invoice `lines.data` holds the new line with an `amount` of 5000, a `description` of "Consulting", and the price created for the line

## ADDED Requirements

### Requirement: Inline prices on invoice lines

The server SHALL accept an invoice line that carries its own price in `price_data`, at invoice creation and on `add_lines`. The inline price SHALL hold a `currency` and an amount, given as `unit_amount` or as `unit_amount_decimal`, and SHALL name a product by id (`price_data[product]`) or by description (`price_data[product_data][name]`). The server SHALL price the line at the inline amount times its `quantity` and SHALL describe it with the name of the product.

#### Scenario: A line with an inline price

- **WHEN** a client adds a line with `price_data` of a `unit_amount` of 2500 and a `quantity` of 2
- **THEN** the line `amount` is 5000 and the line carries the price created for it

#### Scenario: An inline price with a decimal amount

- **WHEN** a client adds a line with `price_data` of a `unit_amount_decimal` of `20.50`
- **THEN** the line `amount` is 2050 and the price created for the line has a `unit_amount` of 2050

#### Scenario: An inline price describing its product

- **WHEN** a client adds a line with `price_data[product_data][name]` of "Consulting"
- **THEN** the line `description` is "Consulting" and the product created for the line is named "Consulting"

#### Scenario: An inline price naming an existing product

- **WHEN** a client adds a line with `price_data[product]` naming a product of the account
- **THEN** the price created for the line carries that product

### Requirement: Inline price objects and checks

The server SHALL create the product and the price of an inline `price_data` line in the catalog, so the price the invoice line carries can be read back from `GET /v1/prices/{id}` and the product from `GET /v1/products/{id}`. The server SHALL reject an inline price whose `currency` differs from the invoice's currency, whose `currency` or amount is missing or malformed, and a `price_data[product]` that does not exist for the account.

#### Scenario: The price of an inline line can be read back

- **WHEN** a client adds a line with an inline price
- **THEN** the price the line carries can be read from `GET /v1/prices/{id}` and the product from `GET /v1/products/{id}`

#### Scenario: An inline price in another currency

- **WHEN** a client adds a line whose `price_data[currency]` differs from the invoice's currency
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged

#### Scenario: An inline price without a currency

- **WHEN** a client adds a line whose `price_data` carries no `currency`
- **THEN** the server responds with an `invalid_request_error` naming `lines[][price_data][currency]`

#### Scenario: An inline price without an amount

- **WHEN** a client adds a line whose `price_data` carries neither `unit_amount` nor `unit_amount_decimal`
- **THEN** the server responds with an `invalid_request_error` naming `lines[][price_data][unit_amount]`

#### Scenario: An inline price naming an unknown product

- **WHEN** a client adds a line whose `price_data[product]` does not exist for the account
- **THEN** the server responds with an `invalid_request_error` and the invoice is unchanged
