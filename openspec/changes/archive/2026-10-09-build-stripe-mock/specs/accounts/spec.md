# Spec Delta

## Purpose

Models Stripe connected accounts through the Accounts v2 API as the source of truth, projects them into the v1 account shape for older endpoints, and manages hosted onboarding links and account state transitions.

## ADDED Requirements

### Requirement: Create account (v2)

The server SHALL create connected accounts at `POST /v2/core/accounts`, returning a `v2.core.account` object with an `acct_` id, `object`, RFC 3339 `created`, `livemode`, and `applied_configurations`.

#### Scenario: Create merchant account

- **WHEN** a client POSTs JSON with `identity`, `configuration.merchant`, and `defaults`
- **THEN** the response has an `acct_` id and `applied_configurations` that includes `merchant`

### Requirement: Account configurations and capabilities

The server SHALL track the `customer`, `merchant`, and `recipient` configurations and the requested and active capabilities within them, and SHALL reflect capability activation across the account lifecycle.

#### Scenario: Requested capability

- **WHEN** an account is created requesting `card_payments` under the merchant configuration
- **THEN** the capability status is reported as pending or active according to onboarding progress

### Requirement: Include and default-null behavior

For v2 account reads the server SHALL return `null` for expandable properties not requested via `include`, and SHALL return their stored values when requested.

#### Scenario: Identity omitted from include

- **WHEN** a client retrieves a v2 account without `include`
- **THEN** `identity`, `requirements`, and configuration detail properties are `null`

#### Scenario: Identity requested

- **WHEN** the same client retrieves the account with `include=["identity","requirements"]`
- **THEN** those properties contain their stored values

### Requirement: v1 account projection

The server SHALL serve `POST /v1/accounts` and `GET /v1/accounts/{id}` and SHALL project v2 account state into the v1 `account` shape, including `charges_enabled`, `payouts_enabled`, `details_submitted`, `capabilities`, and `requirements`.

#### Scenario: Project v2 account to v1

- **WHEN** a client retrieves a v2-created account through `/v1/accounts/{id}`
- **THEN** the response `object` is `account` with v1 fields derived from v2 state

### Requirement: Account list

The server SHALL list accounts at `GET /v2/core/accounts` with `limit`/`page` pagination and at `GET /v1/accounts` with the v1 list envelope.

#### Scenario: List connected accounts

- **WHEN** multiple accounts exist and a client lists them
- **THEN** the response contains the created accounts using the appropriate version's envelope

### Requirement: Account link (v1)

The server SHALL create onboarding links at `POST /v1/account_links`, returning a URL to the mock hosted onboarding page and honoring `type` (`account_onboarding` or `account_update`), `refresh_url`, and `return_url`.

#### Scenario: Create onboarding link

- **WHEN** a client creates an `account_onboarding` link for a connected account
- **THEN** the response contains a `url` pointing at the mock onboarding page

### Requirement: Account link (v2)

The server SHALL create onboarding links at `POST /v2/core/account_links` with a `use_case` of `account_onboarding` or `account_update`, returning a `v2.core.account_link` with `url` and RFC 3339 timestamps.

#### Scenario: Create v2 onboarding link

- **WHEN** a client creates a v2 link with `use_case.account_onboarding.refresh_url`
- **THEN** the response `object` is `v2.core.account_link` and `url` targets the mock onboarding page

### Requirement: Onboarding state transitions

Completing the mock onboarding flow SHALL set `details_submitted`, activate the requested capabilities, update the derived `charges_enabled`/`payouts_enabled` v1 fields, and emit an `account.updated` event.

#### Scenario: Successful onboarding

- **WHEN** the user completes the mock onboarding page and simulates a passing KYC
- **THEN** the account reports `details_submitted=true` with active capabilities and an `account.updated` event is created

#### Scenario: Failed KYC

- **WHEN** the user simulates a failing KYC on the onboarding page
- **THEN** the account remains `details_submitted=false` with unsatisfied requirements and no capability activation
