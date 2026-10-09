# catalog Specification

## Purpose

Manages the account-scoped product and price catalog that checkout sessions and subscriptions reference.

## Requirements

### Requirement: Create and retrieve products

The server SHALL create products at `POST /v1/products` and retrieve them at `GET /v1/products/{id}`, returning a `product` object with a `prod_` id, `object`, `active`, epoch `created`, `name`, `default_price`, and `metadata`.

#### Scenario: Create product

- **WHEN** a client creates a product with a name
- **THEN** the response is a `product` with a `prod_` id and `active` true

### Requirement: Create and retrieve prices

The server SHALL create prices at `POST /v1/prices` referencing a product, and retrieve them at `GET /v1/prices/{id}`, returning a `price` object with a `price_` id, `currency`, `unit_amount`, and an optional `recurring` interval.

#### Scenario: Create one-time price

- **WHEN** a client creates a price with a product and `unit_amount`
- **THEN** the response has `type` of `one_time` and the product's `default_price` is updated

#### Scenario: Create recurring price

- **WHEN** a client creates a price with `recurring.interval=month`
- **THEN** the response has `type` of `recurring` and the interval recorded

### Requirement: List products and prices

The server SHALL list products and prices at their collection endpoints using the list envelope.

#### Scenario: List prices for a product

- **WHEN** a client lists prices filtered by `product`
- **THEN** only prices belonging to that product are returned

### Requirement: Account scoping

The server SHALL own products and prices by the resolved account, isolating catalog entries created for the main account from those created for connected accounts.

#### Scenario: Connected-account catalog isolation

- **WHEN** a product is created with `Stripe-Account` set to a connected account
- **THEN** retrieving that product id without the header does not return it
