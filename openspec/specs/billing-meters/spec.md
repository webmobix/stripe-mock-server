# billing-meters Specification

## Purpose

Adds Stripe billing meters, the customer usage events they collect, and the aggregated event summaries that metered prices bill from.

## Requirements

### Requirement: Manage billing meters

The server SHALL create, retrieve, update, list, deactivate, and reactivate billing meters under `/v1/billing/meters`. A meter SHALL have an `mtr_` id, `display_name`, `event_name`, an aggregation formula, a customer mapping key, a value key, and an `active`/`inactive` status. Only one active meter MAY exist per `event_name`; deactivation records `deactivated_at` and reactivation clears it. A deactivated meter SHALL reject events and SHALL NOT attach to a price.

#### Scenario: Create a sum meter

- **WHEN** a client creates a meter with `default_aggregation.formula=sum`
- **THEN** the response has an `mtr_` id and `status=active`

#### Scenario: Deactivate and reactivate

- **WHEN** a client deactivates a meter and then reactivates it
- **THEN** the status becomes `inactive` with a `deactivated_at`, then `active` with `deactivated_at` null

#### Scenario: Event name conflict

- **WHEN** a client creates a second active meter with an existing `event_name`
- **THEN** the request is rejected with an `invalid_request_error`

### Requirement: Report meter events

`POST /v1/billing/meter_events` SHALL accept `event_name`, a string-valued `payload`, and optional `identifier` and `timestamp`, returning a `billing.meter_event`. An active meter SHALL be required. A missing identifier SHALL be generated. Identifiers SHALL be unique per account: a duplicate SHALL return `duplicate_meter_event`, an unknown event name `no_meter`, and an out-of-range timestamp SHALL be rejected.

#### Scenario: Report usage

- **WHEN** a client reports an event for an active meter with a valid customer and value
- **THEN** the response is a `billing.meter_event` carrying the identifier and payload

#### Scenario: Duplicate identifier

- **WHEN** a client reports the same identifier twice
- **THEN** the second request is rejected with `duplicate_meter_event` and no second event is stored

#### Scenario: No meter

- **WHEN** a client reports an event name with no active meter
- **THEN** the request is rejected with `no_meter`

### Requirement: Aggregate event summaries

`GET /v1/billing/meters/{id}/event_summaries` SHALL require `customer`, `start_time`, and `end_time` and SHALL accept `value_grouping_window` of `hour` or `day`. Each summary SHALL report a stable `mtrusg_` id, the meter, window bounds, and `aggregated_value` computed with the meter's formula. Without grouping one summary SHALL cover the range; grouped summaries SHALL be newest-first. Times SHALL align to minute boundaries and to hour or UTC day boundaries when grouped.

#### Scenario: Sum usage

- **WHEN** a client reports values 10 and 5 and requests a summary over their range
- **THEN** one summary reports `aggregated_value=15`

#### Scenario: Group hourly

- **WHEN** a client requests `value_grouping_window=hour` over hour-aligned times
- **THEN** each hourly bucket with events is returned separately

#### Scenario: Required customer

- **WHEN** a client omits `customer`
- **THEN** the request is rejected with a missing-parameter error
