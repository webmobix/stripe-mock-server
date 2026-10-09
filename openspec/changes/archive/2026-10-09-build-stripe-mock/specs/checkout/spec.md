# Spec Delta

## Purpose

Creates Stripe checkout sessions and drives the mock hosted payment flow that completes payments and subscriptions.

## ADDED Requirements

### Requirement: Create checkout session

The server SHALL create checkout sessions at `POST /v1/checkout/sessions` supporting `mode` of `payment`, `subscription`, and `setup`, with `line_items`, `success_url`, `cancel_url`, and optional `customer` or `customer_account`, returning a `cs_test_` session with `status` of `open` and a `url`.

#### Scenario: Payment-mode session

- **WHEN** a client creates a `payment` session with a one-time price line item and a `success_url`
- **THEN** the response is `open` and `url` points at the mock checkout page

#### Scenario: Subscription-mode session

- **WHEN** a client creates a `subscription` session with a recurring price
- **THEN** completion creates a subscription and the session records it

### Requirement: Retrieve session and line items

The server SHALL retrieve sessions at `GET /v1/checkout/sessions/{id}` and list line items at `GET /v1/checkout/sessions/{id}/line_items`.

#### Scenario: Retrieve line items

- **WHEN** a client requests line items for a session
- **THEN** the session's line items are returned in the list envelope

### Requirement: Completion flow

Completing the mock checkout page SHALL synchronously mark the session `complete`, create the corresponding payment intent and/or subscription, and create the associated events.

#### Scenario: Successful card payment

- **WHEN** the user submits the mock checkout page with a successful card simulation
- **THEN** the session becomes `complete`, a succeeded payment intent exists, and `checkout.session.completed` and `payment_intent.succeeded` events are created

### Requirement: Redirect behavior

On completion the mock page SHALL redirect to `success_url` with `{CHECKOUT_SESSION_ID}` substituted; on cancel or expiry the user SHALL be directed to `cancel_url`.

#### Scenario: Success redirect

- **WHEN** a checkout session completes
- **THEN** the browser is redirected to the `success_url` containing the session id

### Requirement: Expiration

The server SHALL support expiring sessions at `POST /v1/checkout/sessions/{id}/expire`, setting `status` to `expired` and creating a `checkout.session.expired` event.

#### Scenario: Expire session

- **WHEN** a client expires an open session
- **THEN** the status becomes `expired`
