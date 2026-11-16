# Spec Delta

## ADDED Requirements

### Requirement: Inspect raw meter events

The server SHALL expose `GET /internal/meter_events` on the control plane, without Stripe authentication, returning accepted meter events in a v1 list envelope. The endpoint SHALL filter by `event_name` and by the meter's mapped customer id when `customer` is supplied. Each item SHALL carry its `identifier`, `event_name`, the mapped customer id, the reported `value` as a string, and `timestamp`.

#### Scenario: List accepted events

- **WHEN** a client reports two meter events and requests `/internal/meter_events?event_name=api_call&customer=cus_123`
- **THEN** both events are returned with their identifiers, values, and timestamps

#### Scenario: Filter by event name

- **WHEN** a client requests `/internal/meter_events?event_name=documents.used`
- **THEN** only events for that event name are returned

#### Scenario: No API key required

- **WHEN** a client calls `/internal/meter_events` without an Authorization header
- **THEN** the request is served

## MODIFIED Requirements

### Requirement: End a subscription billing period

The server SHALL expose `POST /internal/subscriptions/{id}/advance` accepting `outcome` of `paid` or `failed`. With no open invoice it SHALL advance the period, raise a renewal invoice, and set the subscription `active` or `past_due`; with an open invoice it SHALL retry that invoice instead. A subscription with `cancel_at_period_end=true` SHALL be canceled instead of renewed, with no invoice. The response SHALL include the updated subscription and the invoice, which SHALL be null when the subscription is canceled.

#### Scenario: Failed then recovered renewal

- **WHEN** a test advances with `failed` and then `paid`
- **THEN** the subscription becomes `past_due` with an open invoice, then `active` with the same invoice paid

#### Scenario: Missing outcome

- **WHEN** `outcome` is absent or not `paid`/`failed`
- **THEN** the request is rejected with an `invalid_request_error`

#### Scenario: Cancel at the period boundary

- **WHEN** a test advances a subscription with `cancel_at_period_end=true`
- **THEN** the subscription is canceled, `customer.subscription.deleted` is created, and the response invoice is null
