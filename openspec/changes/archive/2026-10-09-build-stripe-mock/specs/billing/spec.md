# Spec Delta

## Purpose

Manages customers and subscriptions, including account-backed customers, so recurring billing flows can be exercised locally.

## ADDED Requirements

### Requirement: Create and retrieve customers

The server SHALL create customers at `POST /v1/customers` and retrieve them at `GET /v1/customers/{id}` with a `cus_` id.

#### Scenario: Create customer

- **WHEN** a client creates a customer with an email
- **THEN** the response is a `customer` with a `cus_` id and the email stored

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
