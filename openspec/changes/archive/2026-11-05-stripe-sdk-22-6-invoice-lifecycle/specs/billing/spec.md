# Spec Delta

## MODIFIED Requirements

### Requirement: Create and retrieve customers

The server SHALL create customers at `POST /v1/customers` and retrieve them at `GET /v1/customers/{id}` with a `cus_` id. A customer MAY carry tax identification data supplied as `tax_id_data[][type]` and `tax_id_data[][value]`; the server SHALL store each entry as a `tax_ids` object with a `txi_` id, the given `type` and `value`, and expose them on the customer. A customer MAY carry a default `currency`; the server SHALL store it and expose it on the customer.

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

### Requirement: Update customers

The server SHALL update a customer at `POST /v1/customers/{id}`, accepting `email`, `name`, `metadata`, `currency`, and additional `tax_id_data[]` entries. The server SHALL create a `customer.updated` event.

#### Scenario: Update a customer

- **WHEN** a client updates a customer's name
- **THEN** the response is the customer with the new name and a `customer.updated` event exists

#### Scenario: Add a tax id by update

- **WHEN** a client updates a customer with a new `tax_id_data[][type]` and `tax_id_data[][value]`
- **THEN** the customer `tax_ids` contains the original entries plus the new one

#### Scenario: Change a customer's currency

- **WHEN** a client updates a customer with a `currency` of `gbp`
- **THEN** the customer `currency` is `gbp`

#### Scenario: A malformed currency

- **WHEN** a client creates or updates a customer with a `currency` that is not a three-letter ISO code
- **THEN** the server responds with an `invalid_request_error` naming `currency` and the customer is unchanged
