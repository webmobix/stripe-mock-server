# coupons Specification

## Purpose

Lets an application maintain account-scoped coupons that describe a percentage or fixed-amount discount, so checkout sessions can apply a real, inspectable discount instead of a hard-coded total.

## Requirements

### Requirement: Create coupons

The server SHALL create coupons at `POST /v1/coupons`. A coupon SHALL carry exactly one of `percent_off` (greater than 0 and at most 100) or `amount_off` (a positive integer in minor units together with a three-letter `currency`). A client MAY supply its own `id`; otherwise the server SHALL generate a `coupon_` id. A duplicate id SHALL be rejected without creating a coupon.

#### Scenario: Create a percentage coupon

- **WHEN** a client creates a coupon with `percent_off` of 25
- **THEN** the response is a `coupon` with a generated id, `percent_off` 25, `valid` true, and `times_redeemed` 0

#### Scenario: Create an amount coupon

- **WHEN** a client creates a coupon with `amount_off` of 500 and `currency` of `usd`
- **THEN** the response is a `coupon` with `amount_off` 500 and `currency` `usd`

#### Scenario: Both discount forms supplied

- **WHEN** a client creates a coupon with both `percent_off` and `amount_off`
- **THEN** the server responds with an `invalid_request_error` and creates no coupon

#### Scenario: Neither discount form supplied

- **WHEN** a client creates a coupon with neither `percent_off` nor `amount_off`
- **THEN** the server responds with an `invalid_request_error` and creates no coupon

#### Scenario: Percentage out of range

- **WHEN** a client creates a coupon with `percent_off` of 0 or above 100
- **THEN** the server responds with an `invalid_request_error` naming `percent_off`

#### Scenario: Amount without a currency

- **WHEN** a client creates an amount coupon without a `currency`
- **THEN** the server responds with an `invalid_request_error` naming `currency`

#### Scenario: Client-supplied id already exists

- **WHEN** a client creates a coupon whose `id` matches an existing coupon for the account
- **THEN** the server responds with an `invalid_request_error` and the existing coupon is unchanged

### Requirement: Coupon duration and restrictions

The server SHALL accept a `duration` of `once`, `forever`, or `repeating`, defaulting to `once`. A `repeating` coupon SHALL require a positive `duration_in_months`. The server SHALL also accept `max_redemptions`, `redeem_by`, `name`, `metadata`, and an optional `applies_to[products][]` list, and SHALL expose them on the coupon.

#### Scenario: Repeating coupon

- **WHEN** a client creates a coupon with `duration` of `repeating` and `duration_in_months` of 3
- **THEN** the coupon records that duration and month count

#### Scenario: Repeating coupon without months

- **WHEN** a client creates a coupon with `duration` of `repeating` and no `duration_in_months`
- **THEN** the server responds with an `invalid_request_error` naming `duration_in_months`

#### Scenario: Unsupported duration

- **WHEN** a client creates a coupon with a `duration` other than `once`, `forever`, or `repeating`
- **THEN** the server responds with an `invalid_request_error` naming `duration`

#### Scenario: Product-restricted coupon

- **WHEN** a client creates a coupon whose `applies_to[products][]` lists one product
- **THEN** the coupon reports that product under `applies_to.products`

### Requirement: Coupon validity

A coupon SHALL report `valid` true while its `redeem_by` timestamp has not passed and it has fewer redemptions than `max_redemptions` when one is set. The server SHALL reject an invalid or unknown coupon at checkout instead of applying a discount.

#### Scenario: Expired coupon

- **WHEN** a coupon's `redeem_by` is earlier than the current time
- **THEN** the coupon reports `valid` false and checkout creation using it is rejected

#### Scenario: Fully redeemed coupon

- **WHEN** a coupon's `times_redeemed` has reached its `max_redemptions`
- **THEN** the coupon reports `valid` false and checkout creation using it is rejected

### Requirement: Retrieve, list, and delete coupons

The server SHALL retrieve coupons at `GET /v1/coupons/{id}`, list them at `GET /v1/coupons` with the list envelope and pagination, and delete them at `DELETE /v1/coupons/{id}`. A deleted coupon SHALL be removed from later reads and lists and the delete response SHALL be a deleted tombstone.

#### Scenario: Retrieve a coupon

- **WHEN** a client retrieves a coupon it created
- **THEN** the response is that coupon

#### Scenario: List coupons

- **WHEN** a client lists coupons with `limit=2` while more than two exist
- **THEN** two items are returned and `has_more` is true

#### Scenario: Delete a coupon

- **WHEN** a client deletes a coupon
- **THEN** retrieving that coupon returns a `not_found` error and it no longer appears in the list

#### Scenario: Coupon isolation between accounts

- **WHEN** a coupon is created under a connected account
- **THEN** retrieving it without the `Stripe-Account` header returns `not_found`

### Requirement: Checkout redeems coupons

The server SHALL increment a coupon's `times_redeemed` once when a checkout session that applied it completes. The server SHALL reject a coupon whose `applies_to.products` does not include every discounted line's product. Expiry, cancellation, or deletion of an incomplete session SHALL NOT increment the counter.

#### Scenario: Completed session redeems the coupon

- **WHEN** a checkout session created with a coupon completes
- **THEN** that coupon's `times_redeemed` increases by one

#### Scenario: Expired session does not redeem the coupon

- **WHEN** a checkout session created with a coupon expires without completing
- **THEN** that coupon's `times_redeemed` is unchanged

#### Scenario: Coupon restricted to another product

- **WHEN** a checkout session uses a coupon restricted to a product that is not on any line
- **THEN** the server responds with an `invalid_request_error` and creates no session
