# Spec Delta

## ADDED Requirements

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
