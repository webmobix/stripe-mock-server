# event-destinations Specification

## Purpose

TBD - created by archiving change unify-event-destinations. Update Purpose after archive.

## Requirements

### Requirement: Single webhook configuration concept

The server SHALL model webhooks exclusively as event destinations. The TOML
configuration SHALL accept `[[event_destinations]]` and SHALL NOT accept
`[[webhook_targets]]`; each destination declares a `name`, an `event_payload` of
`snapshot` or `thin`, an `events_from` list, an `enabled_events` list, an
`enabled` flag, and a `webhook_endpoint.url`, and the server SHALL generate a
`whsec_` signing secret per endpoint URL and persist it across restarts. Every
destination with the same `webhook_endpoint.url` SHALL report the same secret.

#### Scenario: Config destination is seeded

- **WHEN** the configuration declares an event destination and the server starts
- **THEN** the destination exists, is owned by the platform account, and has a `whsec_` secret for its URL

#### Scenario: Restart is idempotent

- **WHEN** the server restarts with the same `[[event_destinations]]` name
- **THEN** the existing destination is updated in place and its secret is unchanged

#### Scenario: Invalid config fails at boot

- **WHEN** a destination omits `webhook_endpoint.url`, or uses an `event_payload`
  other than `snapshot`/`thin`, or a `type` other than `webhook_endpoint`
- **THEN** configuration parsing fails with a descriptive error

### Requirement: Payload format selects the delivered payload shape

A destination SHALL have exactly one payload format, independent of the event
family (v1 vs v2). A `snapshot` destination SHALL receive `event` objects; a
`thin` destination SHALL receive `v2.core.event` notifications. A single
destination SHALL NOT receive the same event in both formats, and a snapshot
destination SHALL NOT receive thin events (nor vice versa). Selection of *which*
events reach a destination is governed by `enabled_events` and `events_from`,
not by the payload format; a thin destination can therefore receive both
v1-derived (`v1.*`) and v2-derived (`v2.core.*`) event types.

#### Scenario: Snapshot destination receives v1 events

- **WHEN** a v1 snapshot event such as `payment_intent.succeeded` or `account.updated` is emitted
- **THEN** it is delivered to matching `event_payload = "snapshot"` destinations only

#### Scenario: Thin destination receives v2 events

- **WHEN** a `v2.core.account.*` event is emitted
- **THEN** it is delivered to matching `event_payload = "thin"` destinations only

#### Scenario: One thin destination receives both event families

- **WHEN** a thin destination is configured with `events_from = ["@self", "@accounts"]` and `enabled_events = ["*"]`
- **AND** a v2 Account change emits `v1.account.updated` and `v2.core.account.updated`
- **THEN** that single destination receives both thin notifications

#### Scenario: Two destinations for both formats

- **WHEN** a snapshot and a thin destination both point at a receiver
- **THEN** the receiver gets the event in both formats

### Requirement: Account scope via `events_from`

Destination selection SHALL be scoped by `events_from`: `@self` matches events
owned by the destination's account ("Your account"), and `@accounts` matches
events emitted for another (connected) account ("Connected accounts"). The
default SHALL be `["@self"]`.

#### Scenario: Connected-scope event

- **WHEN** a connected account's resource change emits an event and a destination lists `@accounts`
- **THEN** the destination receives it

#### Scenario: Self-scope event

- **WHEN** a platform/self event is emitted and a destination lists `@self`
- **THEN** the destination receives it, and an `@accounts`-only destination does not

### Requirement: v2 Account changes emit both event families

Changing a v2 Account SHALL emit both its v1 and v2 event families, with
Stripe's scope split. `POST /v2/core/accounts/{id}` and onboarding completion
SHALL emit a snapshot `account.updated` and a thin `v1.account.updated` (both
Connected accounts scope) in addition to the v2 thin events
(`v2.core.account.updated`, capability_status_updated, requirements.updated)
which use the Your account scope.

#### Scenario: Updating an account emits all three forms

- **WHEN** `POST /v2/core/accounts/{id}` changes the account
- **THEN** snapshot `account.updated`, thin `v1.account.updated`, and thin `v2.core.account.updated` are each emitted once

#### Scenario: Scope routing of the families

- **WHEN** an `@self` thin destination and an `@accounts` thin destination are configured
- **THEN** `v2.core.account.updated` reaches only the `@self` destination, and `v1.account.updated` reaches only the `@accounts` destination

#### Scenario: Onboarding emits both families

- **WHEN** an existing connected account completes onboarding
- **THEN** a snapshot `account.updated` and a thin `v1.account.updated` are emitted alongside the v2 capability events

### Requirement: Runtime API accepts both payloads

`POST /v2/core/event_destinations` SHALL accept `event_payload` of `snapshot` or
`thin`, returning a v2 `invalid_request_error` for any other value, and SHALL
continue to reject transports other than `webhook_endpoint`.

#### Scenario: Snapshot destination via API

- **WHEN** a destination is created with `event_payload = "snapshot"`
- **THEN** it is created with `status = "enabled"` and receives v1 snapshot events

#### Scenario: Unknown payload is rejected

- **WHEN** a destination is created with an unsupported `event_payload`
- **THEN** the request fails with a v2 `invalid_request_error` envelope

### Requirement: Internal control plane exposes destinations only

The internal control plane SHALL expose event destinations with their secrets at
`GET /internal/destinations` and SHALL NOT expose a separate webhook-target
surface. Delivery records SHALL reference a destination.

#### Scenario: Destination secrets are retrievable

- **WHEN** `GET /internal/destinations` is called
- **THEN** each destination includes `webhook_endpoint.url` and a `whsec_` `webhook_endpoint.signing_secret`, with destinations sharing a URL returning the same secret

#### Scenario: Delivery references a destination

- **WHEN** `GET /internal/deliveries` is called
- **THEN** each delivery includes its `destination` id
