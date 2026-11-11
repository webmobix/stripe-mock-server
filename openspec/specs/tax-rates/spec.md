# tax-rates Specification

## Purpose

Lets an application maintain account-scoped tax rates with a percentage and inclusive or exclusive behavior, so checkout sessions can report and assert taxed totals.

## Requirements

### Requirement: Create tax rates

The server SHALL create tax rates at `POST /v1/tax_rates` with a required `display_name`, `inclusive`, and `percentage` between 0 and 100. The server SHALL assign a `txr_` id and expose `object` of `tax_rate`, `active` (default true), `country`, `state`, `jurisdiction`, `description`, `tax_type`, `metadata`, `livemode`, and `created`.

#### Scenario: Create a tax rate

- **WHEN** a client creates a tax rate with `display_name` of "VAT", `inclusive` false, and `percentage` of 20
- **THEN** the response is a `tax_rate` with a `txr_` id, `active` true, and `percentage` 20

#### Scenario: Create an inclusive tax rate

- **WHEN** a client creates a tax rate with `inclusive` true
- **THEN** the response reports `inclusive` true

#### Scenario: Missing display name

- **WHEN** a client creates a tax rate without a `display_name`
- **THEN** the server responds with an `invalid_request_error` naming `display_name`

#### Scenario: Missing inclusive flag

- **WHEN** a client creates a tax rate without an `inclusive` value
- **THEN** the server responds with an `invalid_request_error` naming `inclusive`

#### Scenario: Percentage out of range

- **WHEN** a client creates a tax rate with a `percentage` below 0 or above 100
- **THEN** the server responds with an `invalid_request_error` naming `percentage`

#### Scenario: Inactive tax rate

- **WHEN** a client creates a tax rate with `active` false
- **THEN** the response reports `active` false

### Requirement: Retrieve and list tax rates

The server SHALL retrieve tax rates at `GET /v1/tax_rates/{id}` and list them at `GET /v1/tax_rates` with the list envelope and pagination, filtering by `active` and `inclusive` where provided.

#### Scenario: Retrieve a tax rate

- **WHEN** a client retrieves a tax rate it created
- **THEN** the response is that tax rate

#### Scenario: List active tax rates

- **WHEN** a client lists tax rates with `active=true`
- **THEN** only active tax rates are returned

#### Scenario: List inclusive tax rates

- **WHEN** a client lists tax rates with `inclusive=false`
- **THEN** only exclusive tax rates are returned

#### Scenario: Unknown tax rate

- **WHEN** a client retrieves a tax rate id that does not exist for the account
- **THEN** the server responds with a `not_found` error

#### Scenario: Tax rate isolation between accounts

- **WHEN** a tax rate is created under a connected account
- **THEN** retrieving it without the `Stripe-Account` header returns `not_found`
