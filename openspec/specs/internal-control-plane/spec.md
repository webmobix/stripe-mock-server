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
