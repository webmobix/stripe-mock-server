# Spec Delta

## ADDED Requirements

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
- **THEN** the account reports `details_submitted=true`, `charges_enabled=true`,
  `payouts_enabled=true`, and its capabilities are `active`

#### Scenario: Seed a pending account

- **WHEN** a client posts `{"name": "Acme", "state": "pending"}`
- **THEN** the account reports `details_submitted=false`, `charges_enabled=false`,
  `payouts_enabled=false`, and its capabilities are `pending`

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
