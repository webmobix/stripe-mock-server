# Spec Delta

## ADDED Requirements

### Requirement: End a subscription billing period

The server SHALL expose `POST /internal/subscriptions/{id}/advance` accepting `outcome` of `paid` or `failed`. With no open invoice it SHALL advance the period, raise a renewal invoice, and set the subscription `active` or `past_due`; with an open invoice it SHALL retry that invoice instead. The response SHALL include the updated subscription and the invoice.

#### Scenario: Failed then recovered renewal

- **WHEN** a test advances with `failed` and then `paid`
- **THEN** the subscription becomes `past_due` with an open invoice, then `active` with the same invoice paid

#### Scenario: Missing outcome

- **WHEN** `outcome` is absent or not `paid`/`failed`
- **THEN** the request is rejected with an `invalid_request_error`

## MODIFIED Requirements

### Requirement: Reset

The server SHALL reset state on request at `/internal/reset`, clearing all objects, events, deliveries, idempotency keys, and saved payment methods and reapplying configuration seeding. Configured main-account keys and webhook signing secrets SHALL be reapplied and therefore survive a reset.

#### Scenario: Reset between tests

- **WHEN** a client calls reset after creating accounts and products
- **THEN** subsequent reads show only the reseeded main account

#### Scenario: Configured secrets survive

- **WHEN** the configuration fixes the secret key and a signing secret and a client calls reset
- **THEN** `/internal/main` and `/internal/destinations` report the configured values again
