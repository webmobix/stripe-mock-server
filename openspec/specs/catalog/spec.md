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

### Requirement: Price lookup keys

`POST /v1/prices` SHALL accept a `lookup_key` and the price object SHALL expose it, as `null` when absent. `GET /v1/prices` SHALL filter by repeated `lookup_keys[]` values, returning only prices whose lookup key matches one of them.

#### Scenario: Create a price with a lookup key

- **WHEN** a client creates a price with `lookup_key` of `pro_monthly`
- **THEN** the response price reports `lookup_key` of `pro_monthly`

#### Scenario: Create a price without a lookup key

- **WHEN** a client creates a price without `lookup_key`
- **THEN** the response price reports `lookup_key` as null

#### Scenario: Retrieve a price with a lookup key

- **WHEN** a client retrieves a price that has a lookup key
- **THEN** the response still reports that lookup key

#### Scenario: List prices by lookup keys

- **WHEN** a client lists prices with `lookup_keys[]=pro_monthly` and `lookup_keys[]=team_monthly`
- **THEN** only prices carrying either lookup key are returned

#### Scenario: A lookup key with no matching price

- **WHEN** a client lists prices with a lookup key that no price carries
- **THEN** the response is an empty list

### Requirement: Lookup key uniqueness

Lookup keys SHALL be unique among an account's active prices. Creating an active price with a key already held by another active price SHALL be rejected unless `transfer_lookup_key=true`, which SHALL clear the key from the previous price and assign it to the new one.

#### Scenario: Duplicate lookup key rejected

- **WHEN** a client creates a second active price with a lookup key already in use
- **THEN** the server responds with an `invalid_request_error` and creates no price

#### Scenario: Transfer a lookup key

- **WHEN** a client creates an active price with an existing lookup key and `transfer_lookup_key=true`
- **THEN** the new price carries the key and the previous price reports `lookup_key` as null

#### Scenario: Lookup keys are account-scoped

- **WHEN** two connected accounts each create an active price with the same lookup key
- **THEN** both prices are created and each is visible only to its own account

### Requirement: Price product expansion

Price responses SHALL support `expand[]=product`, replacing the product id with the full product object. Expansion SHALL apply to creation, retrieval, listing, and prices embedded in subscription and checkout responses.

#### Scenario: Expanded product on a retrieved price

- **WHEN** a client retrieves a price with `expand[]=product`
- **THEN** the price's `product` is the full product object

#### Scenario: Expanded product in a price list

- **WHEN** a client lists prices with `expand[]=product`
- **THEN** every returned price carries its full product object

#### Scenario: Unknown product reference

- **WHEN** a price's product cannot be read for the account during expansion
- **THEN** the price's `product` remains the original product id
