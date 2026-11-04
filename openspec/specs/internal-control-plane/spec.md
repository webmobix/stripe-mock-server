# internal-control-plane Specification

## Purpose

Exposes a local, non-Stripe control plane for retrieving generated secrets, inspecting delivery state, and resetting the server so automated tests can drive the mock deterministically.

## Requirements

### Requirement: Main account endpoint

The server SHALL expose the main account and its keys at `GET /internal/main`.

#### Scenario: Read main account

- **WHEN** a client requests the main account
- **THEN** the response includes the main account id and its generated secret and publishable keys

### Requirement: Secret retrieval

The server SHALL expose generated secrets the Stripe API does not return, including each account's secret and publishable keys and event destination signing secrets. There SHALL be one signing secret per endpoint URL, shared by all destinations that use that URL.

#### Scenario: Retrieve webhook secrets

- **WHEN** a client requests internals for the configured or API-created event destinations
- **THEN** each destination's `whsec_` signing secret and `webhook_endpoint.url` are returned, and destinations sharing a URL return the same secret

### Requirement: Delivery inspection and waiting

The server SHALL expose the delivery journal at `/internal/deliveries`, a blocking wait that resolves when a given event's delivery reaches a terminal state, a flush that resolves when all pending deliveries settle, and a manual retry.

#### Scenario: Await a delivery

- **WHEN** a test triggers an event and calls the wait endpoint for that event
- **THEN** the call returns when the delivery reaches a terminal status or a timeout elapses

### Requirement: Reset

The server SHALL reset state on request at `/internal/reset`, clearing all objects and deliveries and reapplying configuration seeding.

#### Scenario: Reset between tests

- **WHEN** a client calls reset after creating accounts and products
- **THEN** subsequent reads show only the reseeded main account

### Requirement: Local-only exposure

The internal control plane SHALL NOT require Stripe authentication and SHALL be reachable only on the configured bind address.

#### Scenario: No API key required

- **WHEN** a client calls an `/internal` endpoint without an Authorization header
- **THEN** the request is served

### Requirement: Payment intent resolution endpoint

The server SHALL expose `POST /internal/payment_intents/{id}/resolve` on the control plane, taking a `status` of `succeeded`, `failed`, or `canceled`, so a test can settle a payment without a hosted page. The endpoint SHALL NOT require Stripe authentication, consistent with the rest of the control plane.

#### Scenario: Resolve from a test

- **WHEN** a client without an Authorization header resolves a payment intent as `succeeded`
- **THEN** the payment intent status becomes `succeeded` and the related invoice reflects the payment

#### Scenario: Invalid resolve status

- **WHEN** a client resolves a payment intent with an unsupported `status`
- **THEN** the server responds with an `invalid_request_error` and the payment intent status is unchanged

### Requirement: Seed connected account endpoint

The internal control plane SHALL expose `POST /internal/accounts` to create a
connected (`is_main = 0`) account from minimal fields, without Stripe
authentication. The JSON request body SHALL accept `name` (business name),
optional `email`, optional `country` (default `US`), and `state`. Each request
SHALL create a new account and the response SHALL include the generated `acct_`
id.

#### Scenario: Seed with only a business name

- **WHEN** a client posts `{"name": "Acme"}` to `/internal/accounts`
- **THEN** a connected account is created and the response includes its `acct_` id

#### Scenario: Each call creates a distinct account

- **WHEN** a client posts the same body twice
- **THEN** two different `acct_` ids are returned and both accounts exist

#### Scenario: No API key required

- **WHEN** a client posts to `/internal/accounts` without an `Authorization` header
- **THEN** the request is served

### Requirement: Seed account state

The seed request SHALL accept a `state` field of `active` or `pending`, defaulting
to `pending` when omitted. `active` SHALL set `details_submitted`,
`charges_enabled`, and `payouts_enabled` to true and all requested capabilities
to `active`. `pending` SHALL set those flags to false and all requested
capabilities to `pending`.

#### Scenario: Seed an active account

- **WHEN** a client posts `{"name": "Acme", "state": "active"}`
- **THEN** the account reports `details_submitted=true`, `charges_enabled=true`, `payouts_enabled=true`, and its capabilities are `active`

#### Scenario: Seed a pending account

- **WHEN** a client posts `{"name": "Acme", "state": "pending"}`
- **THEN** the account reports `details_submitted=false`, `charges_enabled=false`, `payouts_enabled=false`, and its capabilities are `pending`

#### Scenario: State defaults to pending

- **WHEN** a client posts a body without `state`
- **THEN** the account is seeded in the `pending` state

#### Scenario: Unknown state is rejected

- **WHEN** a client posts `{"name": "Acme", "state": "frozen"}`
- **THEN** the request fails with an invalid-parameter error and no account is created

### Requirement: Seeded account keys

The seed response SHALL include a generated secret key and publishable key for
the account, SHALL persist them so the existing internal secret endpoints return
the same values, and the secret key SHALL authenticate Stripe API requests.

#### Scenario: Keys returned and retrievable

- **WHEN** a client seeds an account and then calls `/internal/accounts/{id}/secrets`
- **THEN** both responses contain the same `sk_test_` and `pk_test_` keys

#### Scenario: Seeded key authenticates

- **WHEN** a client uses a seeded account's secret key as a Bearer token on a Stripe API request
- **THEN** the request is authenticated as that connected account
