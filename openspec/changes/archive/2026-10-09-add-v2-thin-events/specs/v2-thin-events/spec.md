# Spec Delta

## ADDED Requirements

### Requirement: Event destination resource

The server SHALL expose `v2.core.event_destination` objects with `ed_test_` identifiers, supporting `webhook_endpoint` destinations that carry a URL, an `enabled_events` list, an `event_payload` of `snapshot` or `thin`, a `status` of `enabled` or `disabled`, and a `whsec_` signing secret issued per endpoint URL (destinations sharing a URL share the secret).

#### Scenario: Create a webhook event destination

- **WHEN** `POST /v2/core/event_destinations` is called with `name`, `enabled_events`, `event_payload = "thin"`, `type = "webhook_endpoint"`, and `webhook_endpoint.url`
- **THEN** the response is a `v2.core.event_destination` with an `ed_test_` id, `status = "enabled"`, the supplied `enabled_events`, and RFC 3339 `created`/`updated`

#### Scenario: Signing secret is include-gated

- **WHEN** a destination is created without `webhook_endpoint.signing_secret` in `include`
- **THEN** `webhook_endpoint.signing_secret` is `null`; when the include is supplied it contains a `whsec_` value

#### Scenario: Unsupported transport

- **WHEN** a destination is created with `type` other than `webhook_endpoint`, or an `event_payload` other than `snapshot`/`thin`
- **THEN** the request fails with a v2 `invalid_request_error` envelope

### Requirement: Event destination lifecycle

The server SHALL support retrieving, listing, updating, deleting, enabling, disabling, and pinging event destinations.

#### Scenario: Disable and enable

- **WHEN** `POST /v2/core/event_destinations/{id}/disable` is called
- **THEN** the destination `status` becomes `disabled` and `status_details.disabled.reason` is `user`; a subsequent `/enable` returns it to `enabled`

#### Scenario: Deleted destinations do not receive events

- **WHEN** a destination is deleted
- **THEN** it no longer appears in lists and does not receive deliveries

### Requirement: Thin event emission on v2 account creation
The server SHALL create a `v2.core.event` with type `v2.core.account.created` in the same transaction as every account created through `POST /v2/core/accounts`, with `related_object` referencing the new account.

#### Scenario: Account creation emits a thin event

- **WHEN** an account is created via `POST /v2/core/accounts`
- **THEN** a `v2.core.account.created` event exists whose `related_object` is `{"id": "<acct_id>", "type": "v2.core.account", "url": "/v2/core/accounts/<acct_id>"}`

### Requirement: Config-declared event destinations

The server SHALL allow event destinations to be declared in the TOML config under `[[event_destinations]]` (`name`, `description`, `type`, `event_payload`, `events_from`, `enabled_events`, `enabled`, and `webhook_endpoint.url`), seeding them at boot with a `whsec_` secret keyed by endpoint URL and updating them idempotently by name on restart. Snapshot and thin destinations are both supported and coexist.

#### Scenario: Config destination is seeded

- **WHEN** the config declares an event destination and the server starts
- **THEN** the destination exists, is owned by the platform account, and receives matching events

#### Scenario: Restart is idempotent

- **WHEN** the server restarts with the same `[[event_destinations]]` name
- **THEN** the existing destination is updated in place and its secret is unchanged

### Requirement: Thin events on account updates

The server SHALL emit v2 thin events when an existing account changes: `v2.core.account.updated` for any change, `v2.core.account[configuration.<section>].capability_status_updated` for each newly requested capability, and `v2.core.account[requirements].updated` when onboarding requirements change.

#### Scenario: Update an existing account

- **WHEN** `POST /v2/core/accounts/{id}` changes `metadata`, `identity`, `defaults`, or `configuration`
- **THEN** a `v2.core.account.updated` thin event is emitted, plus a capability-status event for each newly requested capability

#### Scenario: Onboard an existing account

- **WHEN** an existing connected account completes onboarding with KYC pass
- **THEN** `v2.core.account[configuration.<section>].capability_status_updated` and `v2.core.account.updated` thin events are emitted

#### Scenario: Failed onboarding

- **WHEN** onboarding fails KYC
- **THEN** `v2.core.account[requirements].updated` and `v2.core.account.updated` thin events are emitted

### Requirement: Thin event shape

V2 events SHALL be serialized as `v2.core.event` with `id` (`evt_test_`), `object`, `type`, RFC 3339 `created`, `livemode`, and `related_object`; the delivered thin payload SHALL omit the fetched-only envelope fields.

#### Scenario: Delivered thin payload

- **WHEN** a thin event is delivered to an event destination
- **THEN** its JSON body contains `id`, `object = "v2.core.event"`, `type`, `created`, `livemode`, and `related_object`, and does not contain `data`, `changes`, or `reason`

#### Scenario: Fetched event includes envelope fields

- **WHEN** `GET /v2/core/events/{id}` is called for a v2 event
- **THEN** the response additionally contains `data`, `reason`, `changes`, and `context` keys (values may be `null`)

### Requirement: Destination-based matching and signed delivery

The dispatcher SHALL deliver a thin event to every enabled destination of the owning account whose `enabled_events` matches the event type (exact name, `prefix.*`, or `*`), signing the body with that destination's secret using HMAC-SHA256 over `<timestamp>.<payload>` in a `Stripe-Signature` header, preserving the per-destination FIFO queue and retry journal.

#### Scenario: Matching destination receives the event

- **WHEN** a `v2.core.account.created` event is emitted and an enabled destination lists that type (or `v2.core.account.*`, or `*`)
- **THEN** the destination receives a signed thin delivery

#### Scenario: Non-matching destination is skipped

- **WHEN** an enabled destination's `enabled_events` does not match the event type
- **THEN** no delivery row is created for it

#### Scenario: Disabled destination is skipped

- **WHEN** a destination is disabled
- **THEN** emitting a matching event creates no delivery for it

### Requirement: Payload format isolation

A destination has one payload format. Snapshot destinations SHALL NOT receive thin events, and thin destinations SHALL NOT receive snapshot events. This isolation is by payload shape only, not by event family: a thin destination may receive thin renderings of v1-derived events (for example `v1.account.updated`) as well as `v2.core.*` events.

#### Scenario: Snapshot destination does not receive v2 events

- **WHEN** a snapshot destination is configured with `enabled_events = ["*"]` and a `v2.core.account.created` event is emitted
- **THEN** the snapshot destination receives no delivery

### Requirement: v2 events read API

The server SHALL expose `GET /v2/core/events` (newest-first, paginated, filterable by `types` and `object_id`) and `GET /v2/core/events/{id}` (full event, 404 when unknown).

#### Scenario: List and retrieve

- **WHEN** accounts have been created through the v2 API
- **THEN** `GET /v2/core/events` returns the thin events newest-first and `GET /v2/core/events/{id}` returns the matching event

### Requirement: Internal destination secret exposure

The internal control plane SHALL expose event destinations with their signing secrets so a local receiver can verify deliveries.

#### Scenario: Secret retrieval

- **WHEN** `GET /internal/destinations` is called
- **THEN** each destination includes its `whsec_` `secret`
