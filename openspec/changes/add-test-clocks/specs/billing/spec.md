# Spec Delta

## MODIFIED Requirements

### Requirement: Create and retrieve customers

The server SHALL create customers at `POST /v1/customers` and retrieve them at `GET /v1/customers/{id}` with a `cus_` id. A customer MAY carry tax identification data supplied as `tax_id_data[][type]` and `tax_id_data[][value]`; the server SHALL store each entry as a `tax_ids` object with a `txi_` id, the given `type` and `value`, and expose them on the customer. A customer MAY carry a default `currency`; the server SHALL store it and expose it on the customer. A customer MAY be created with a `test_clock`; the server SHALL attach it, render `test_clock`, and stamp `created` with the clock's `frozen_time`.

#### Scenario: Create customer

- **WHEN** a client creates a customer with an email
- **THEN** the response is a `customer` with a `cus_` id and the email stored

#### Scenario: Create customer with tax identification

- **WHEN** a client creates a customer with `tax_id_data[0][type]` of `eu_vat` and `tax_id_data[0][value]` of `DE123456789`
- **THEN** the customer `tax_ids` contains one entry with that type and value and a `txi_` id

#### Scenario: Tax identification without a value

- **WHEN** a client creates a customer with a `tax_id_data[][type]` but no matching `value`
- **THEN** the server responds with an `invalid_request_error` naming the missing value and creates no customer

#### Scenario: Create customer with a currency

- **WHEN** a client creates a customer with a `currency` of `eur`
- **THEN** the customer `currency` is `eur`

#### Scenario: Create customer on a test clock

- **WHEN** a client creates a customer with `test_clock` naming a ready clock
- **THEN** the customer reports that `test_clock` and `created` equals the clock's `frozen_time`

#### Scenario: Unknown test clock

- **WHEN** a client creates a customer with a `test_clock` that does not exist for the account
- **THEN** the request is rejected with an `invalid_request_error` and no customer is created

### Requirement: Create subscriptions

The server SHALL create subscriptions at `POST /v1/subscriptions` from one or more `items[][price]` references and SHALL return a `sub_` object with `status`, `items`, `customer` or `customer_account`, and period fields. A subscription for a customer that has a `test_clock` SHALL inherit that clock, render `test_clock`, and stamp `created`, `current_period_start`, and its item periods with the clock's `frozen_time`. Its first period SHALL follow the price interval.

#### Scenario: Create subscription for a customer

- **WHEN** a client creates a subscription with a recurring price and a `customer`
- **THEN** the response has a subscription status and the price present in `items.data`

#### Scenario: Create subscription for an account-backed customer

- **WHEN** a client supplies `customer_account` with an `acct_` id
- **THEN** the subscription records that account as its customer

#### Scenario: Subscription inherits the test clock

- **WHEN** a client creates a subscription for a customer attached to a test clock
- **THEN** the subscription reports `test_clock`, `created` equals the clock's `frozen_time`, and its period ends one price interval later
