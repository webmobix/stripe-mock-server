# Spec Delta

## ADDED Requirements

### Requirement: Subscription invoice parent details

A subscription invoice SHALL report `parent` as a `subscription_details` object carrying the subscription id and the subscription's metadata. Each line priced from a subscription item SHALL report `parent.subscription_item_details` with the subscription and item ids. The existing `subscription` and `subscription_item` fields SHALL remain present.

#### Scenario: Parent on a subscription invoice

- **WHEN** a client retrieves a subscription invoice
- **THEN** `parent.type=subscription_details`, `parent.subscription_details.subscription` names the subscription, and its metadata is copied

#### Scenario: Parent on an invoice line

- **WHEN** a subscription invoice line is priced from a subscription item
- **THEN** `lines.data[].parent.subscription_item_details` carries the subscription and item ids

#### Scenario: Legacy fields remain

- **WHEN** a client retrieves a subscription invoice
- **THEN** `subscription` and each line's `subscription_item` are still reported

### Requirement: Line pricing details

Each invoice line that carries a price SHALL report `pricing` with `type=price_details`, the `price` and `product` ids, and `unit_amount_decimal` as a decimal string for the line's unit amount. Lines without a price SHALL report `pricing=null`.

#### Scenario: Pricing on a subscription line

- **WHEN** a subscription invoice line is priced at 4900 minor units
- **THEN** `pricing.price_details.price` names the price and `pricing.unit_amount_decimal` is `"4900"`

#### Scenario: Pricing on an ad-hoc line

- **WHEN** an invoice line carries only an `amount`
- **THEN** its `pricing` is null

### Requirement: Invoice total taxes

Every invoice SHALL report `total_taxes` as an array. It SHALL be empty when the invoice has no tax, and SHALL contain one entry per applied tax rate with the rate, taxed amount, and taxable amount otherwise.

#### Scenario: No tax

- **WHEN** a client retrieves a subscription invoice with no tax rates
- **THEN** `total_taxes` is an empty array

#### Scenario: Taxed invoice

- **WHEN** an invoice carries a 10 percent tax rate on an amount of 1000
- **THEN** `total_taxes` contains one entry with an amount of 100 and a taxable amount of 1000

### Requirement: Preview invoice details

`POST /v1/invoices/create_preview` SHALL render the preview with the same `parent`, line `parent`, line `pricing`, and `total_taxes` fields as the persisted invoice it forecasts.

#### Scenario: Preview subscription details

- **WHEN** a client previews the upcoming invoice for a subscription
- **THEN** `parent.subscription_details.subscription` names the subscription and its lines carry `pricing.price_details`
