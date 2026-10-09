# Spec Delta

## Purpose

Emits Stripe events for state changes and delivers them to configured event destinations with signing, ordering, and retries.

## ADDED Requirements

### Requirement: Synchronous event creation

On a state change the server SHALL create the `evt_` Event object and its delivery records within the same transaction as the state mutation, before responding to the request.

#### Scenario: Event visible immediately

- **WHEN** a checkout session completes and the response returns
- **THEN** the corresponding event already exists and can be listed without waiting for delivery

### Requirement: v1 snapshot event shape

Events SHALL be serialized as v1 snapshot events containing `id`, `object` of `event`, `type`, `data.object`, epoch `created`, `api_version`, and `account` when the source account is connected.

#### Scenario: Connected-account event

- **WHEN** an event originates from a connected account
- **THEN** the delivered event includes `account` set to that `acct_` id

### Requirement: Event destination fan-out

The dispatcher SHALL deliver events to event destinations defined in configuration or created at runtime, selecting destinations by payload format (`snapshot` vs `thin`), by `events_from` scope (`@self` / `@accounts`), and by an `enabled_events` allow-list supporting exact names and `prefix.*` patterns, defaulting to all events when `*` is configured.

#### Scenario: Scope filtering

- **WHEN** a connected-account event is emitted and only an `@self` destination is configured
- **THEN** no delivery is made to that destination

#### Scenario: Event filter

- **WHEN** an `@accounts` destination lists `customer.subscription.*`
- **THEN** it receives subscription events and not unrelated events

#### Scenario: Payload format filtering

- **WHEN** a thin event is emitted and only a `snapshot` destination is configured
- **THEN** no delivery is made to that destination

### Requirement: Signed delivery

The dispatcher SHALL sign each delivery using HMAC-SHA256 over `<timestamp>.<payload>` with the destination's signing secret, send the result in a `Stripe-Signature` header, and include a unique delivery identifier header.

#### Scenario: Signature verification

- **WHEN** a receiver verifies a delivery with the target's signing secret using the Stripe SDK
- **THEN** verification succeeds

### Requirement: Asynchronous ordered delivery

Deliveries SHALL be asynchronous and SHALL be sent per destination through a FIFO queue with a fixed 200 ms delay between deliveries, preserving emit order per destination.

#### Scenario: Response precedes webhook

- **WHEN** an API call triggers an event
- **THEN** the API response returns before the webhook is received

#### Scenario: Ordering

- **WHEN** two events are emitted in order for the same destination
- **THEN** they are delivered in that order, spaced approximately 200 ms apart

### Requirement: Simple retries and delivery journal

The dispatcher SHALL retry a failed delivery up to three attempts and SHALL record every attempt in a delivery journal with status, HTTP response status, and timestamps.

#### Scenario: Receiver failure

- **WHEN** a destination returns a non-2xx response
- **THEN** the delivery is retried up to the configured attempt limit and its journal entry ends in a terminal failed status
