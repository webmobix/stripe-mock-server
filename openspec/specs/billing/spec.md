# billing Specification

## Purpose

Manages customers and subscriptions, including account-backed customers, so recurring billing flows can be exercised locally.

## Requirements

### Requirement: Create and retrieve customers

The server SHALL create customers at `POST /v1/customers` and retrieve them at `GET /v1/customers/{id}` with a `cus_` id. A customer MAY carry tax identification data supplied as `tax_id_data[][type]` and `tax_id_data[][value]`; the server SHALL store each entry as a `tax_ids` object with a `txi_` id, the given `type` and `value`, and expose them on the customer.

#### Scenario: Create customer

- **WHEN** a client creates a customer with an email
- **THEN** the response is a `customer` with a `cus_` id and the email stored

#### Scenario: Create customer with tax identification

- **WHEN** a client creates a customer with `tax_id_data[0][type]` of `eu_vat` and `tax_id_data[0][value]` of `DE123456789`
- **THEN** the customer `tax_ids` contains one entry with that type and value and a `txi_` id

#### Scenario: Tax identification without a value

- **WHEN** a client creates a customer with a `tax_id_data[][type]` but no matching `value`
- **THEN** the server responds with an `invalid_request_error` naming the missing value and creates no customer

### Requirement: Create subscriptions

The server SHALL create subscriptions at `POST /v1/subscriptions` from one or more `items[][price]` references and SHALL return a `sub_` object with `status`, `items`, `customer` or `customer_account`, and period fields.

#### Scenario: Create subscription for a customer

- **WHEN** a client creates a subscription with a recurring price and a `customer`
- **THEN** the response has a subscription status and the price present in `items.data`

#### Scenario: Create subscription for an account-backed customer

- **WHEN** a client supplies `customer_account` with an `acct_` id
- **THEN** the subscription records that account as its customer

### Requirement: Subscription status lifecycle

The server SHALL transition subscription status on creation, update, and cancellation, and SHALL create `customer.subscription.created`, `.updated`, and `.deleted` events.

#### Scenario: Cancel subscription

- **WHEN** a client cancels a subscription
- **THEN** the status becomes `canceled` and a `customer.subscription.deleted` event is created

### Requirement: Retrieve and list subscriptions

The server SHALL retrieve subscriptions at `GET /v1/subscriptions/{id}` and list them with the list envelope, filtering by `customer` or `customer_account` where provided.

#### Scenario: List by customer

- **WHEN** a client lists subscriptions with `customer=<cus_...>`
- **THEN** only that customer's subscriptions are returned

### Requirement: List customers

The server SHALL list customers at `GET /v1/customers` with the list envelope, filtering by `email` where provided.

#### Scenario: List customers by email

- **WHEN** a client lists customers with `email=buyer@example.com`
- **THEN** only customers with that email are returned

### Requirement: Update customers

The server SHALL update a customer at `POST /v1/customers/{id}`, accepting `email`, `name`, `metadata`, and additional `tax_id_data[]` entries. The server SHALL create a `customer.updated` event.

#### Scenario: Update a customer

- **WHEN** a client updates a customer's name
- **THEN** the response is the customer with the new name and a `customer.updated` event exists

#### Scenario: Add a tax id by update

- **WHEN** a client updates a customer with a new `tax_id_data[][type]` and `tax_id_data[][value]`
- **THEN** the customer `tax_ids` contains the original entries plus the new one

### Requirement: Delete customers

The server SHALL delete a customer at `DELETE /v1/customers/{id}`, removing it from subsequent reads and lists. The server SHALL create a `customer.deleted` event.

#### Scenario: Delete a customer

- **WHEN** a client deletes a customer
- **THEN** retrieving that customer returns a `not_found` error and a `customer.deleted` event exists

### Requirement: Customer creation event

The server SHALL create a `customer.created` event in the same transaction as the customer it describes.

#### Scenario: Customer created event

- **WHEN** a client creates a customer
- **THEN** a `customer.created` event exists whose `data.object` is that customer
