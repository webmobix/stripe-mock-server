# Spec Delta

## Purpose

Adds Stripe-compatible test clocks so a test can freeze a customer's clock,
create subscriptions at that time, and advance once to drive every renewal,
invoice, and cancellation deterministically.

## ADDED Requirements

### Requirement: Manage test clocks

The server SHALL expose `/v1/test_helpers/test_clocks` for create, retrieve,
list, and delete. A clock SHALL have a `clock_` id, object
`test_helpers.test_clock`, `created`, `deletes_after`, `frozen_time`,
`livemode=false`, `name`, `status`, and `status_details`. Create SHALL require
`frozen_time` and accept `name` (at most 300 characters) and an existing
`customer` to attach. Delete SHALL return a deleted tombstone.

#### Scenario: Create a test clock

- **WHEN** a client posts `frozen_time=1577836800` and `name=Annual renewal`
- **THEN** the response is a `test_helpers.test_clock` with a `clock_` id,
  `frozen_time` 1577836800, `name` "Annual renewal", and `status=ready`

#### Scenario: List test clocks

- **WHEN** a client lists test clocks
- **THEN** the response is a list envelope whose `data` holds the account's
  clocks newest first

#### Scenario: Delete a test clock

- **WHEN** a client deletes a clock
- **THEN** the response is `{id, object: "test_helpers.test_clock", deleted: true}`
  and a later retrieval responds `not_found`

#### Scenario: Missing frozen time

- **WHEN** a client creates a clock without `frozen_time`
- **THEN** the request is rejected with a missing-parameter error

### Requirement: Attach a clock to customers and subscriptions

Creating a customer with `test_clock`, or creating a clock with `customer`,
SHALL attach them. A customer SHALL have at most one clock and the association
SHALL NOT change after creation. A subscription created for a clock customer
SHALL inherit the clock. Customer, subscription, and invoice objects SHALL
render `test_clock` and support `expand[]=test_clock`.

#### Scenario: Create a customer at frozen time

- **WHEN** a client creates a customer with `test_clock=clock_...`
- **THEN** the customer reports `test_clock` and its `created` equals the
  clock's `frozen_time`

#### Scenario: Attach a clock to an existing customer

- **WHEN** a client creates a clock with an existing `customer`
- **THEN** that customer reports the new clock and its `created` equals the
  clock's `frozen_time`

#### Scenario: Reject an attached customer with subscriptions

- **WHEN** a client creates a clock with a `customer` that has a subscription
  that is not `canceled`
- **THEN** the request is rejected with an `invalid_request_error` and no clock
  is created

#### Scenario: Subscription inherits the clock

- **WHEN** a client creates a subscription for a clock customer
- **THEN** the subscription reports `test_clock`, and `created`,
  `current_period_start`, and its item periods use the frozen time

#### Scenario: Reject a second clock

- **WHEN** a client attaches a second clock to a customer that already has one
- **THEN** the request is rejected with an `invalid_request_error`

#### Scenario: Expand the clock

- **WHEN** a client retrieves a clock customer with `expand[]=test_clock`
- **THEN** `test_clock` is the full test clock object

### Requirement: Advance a test clock

`POST /v1/test_helpers/test_clocks/{id}/advance` SHALL require a `frozen_time`
later than the current one. The target SHALL be at most two intervals of the
clock's shortest subscription, or two years when the clock has no
subscriptions. The response SHALL report status `advancing` with
`status_details.advancing.target_frozen_time`, and a later retrieval SHALL
report `ready` and the new frozen time.

#### Scenario: Advance to a later time

- **WHEN** a client advances a ready clock to a later `frozen_time`
- **THEN** the response has `status=advancing` and
  `status_details.advancing.target_frozen_time`, and retrieval then reports
  `status=ready` with the new `frozen_time`

#### Scenario: Reject a backwards advance

- **WHEN** a client advances to a time at or before the current frozen time
- **THEN** the request is rejected with an `invalid_request_error` naming
  `frozen_time`

#### Scenario: Limit a clock with subscriptions

- **WHEN** a client advances a clock with a monthly subscription more than two
  months ahead
- **THEN** the request is rejected with an `invalid_request_error`

#### Scenario: Limit a clock without subscriptions

- **WHEN** a client advances an empty clock more than two years ahead
- **THEN** the request is rejected with an `invalid_request_error`

### Requirement: Clock-wide renewal

Advancing a clock SHALL synchronously process every clock-scoped subscription.
An open invoice SHALL be paid first. Then, for each period ending at or before
the target, the server SHALL raise a paid `subscription_cycle` invoice at the
boundary, bill metered usage for the ended period, and advance the period.
`cancel_at_period_end` SHALL cancel at the boundary without a renewal. Each
change SHALL emit its standard subscription and invoice events.

#### Scenario: Advance one period

- **WHEN** a clock with an active monthly subscription advances one month
- **THEN** the subscription is `active` with the next period and one paid
  `subscription_cycle` invoice exists for the ended period

#### Scenario: Catch up several periods

- **WHEN** a clock advances across two renewal boundaries in one call
- **THEN** two paid renewal invoices exist and the subscription period is the
  latest period

#### Scenario: Cancel at period end

- **WHEN** a clock subscription has `cancel_at_period_end` and the clock
  passes its period end
- **THEN** the subscription becomes `canceled` and no renewal invoice is raised

#### Scenario: Recover an open invoice

- **WHEN** a clock subscription has an open invoice from a failed collection
  and the clock advances
- **THEN** that invoice is paid before the next renewal is considered

#### Scenario: Metered usage is billed in arrears

- **WHEN** metered usage was reported during a period and the clock advances
  past its end
- **THEN** the renewal invoice bills that usage for the ended period

### Requirement: Delete a clock with its objects

Deleting a clock SHALL cancel its subscriptions, delete its customers and their
saved payment methods, emit `test_helpers.test_clock.deleted` plus the standard
subscription and customer deletion events, and make the clock unreadable. Its
invoices SHALL remain readable.

#### Scenario: Delete a clock

- **WHEN** a client deletes a clock that has a customer and an active
  subscription
- **THEN** the subscription is `canceled`, the customer is gone, and the clock
  retrieval responds `not_found`

### Requirement: Clock lifecycle events

Creating, advancing, and deleting a clock SHALL emit
`test_helpers.test_clock.created`, `test_helpers.test_clock.advancing`,
`test_helpers.test_clock.ready`, and `test_helpers.test_clock.deleted` events
with the clock as `data.object`, in the same transaction as the state change.

#### Scenario: Emit clock events

- **WHEN** a client creates and then advances a clock
- **THEN** `test_helpers.test_clock.created`, `.advancing`, and `.ready` events
  exist, each carrying that clock

### Requirement: Reset clears clocks

`POST /internal/reset` SHALL clear all test clocks along with their
associations on customers, subscriptions, and invoices.

#### Scenario: Reset removes clocks

- **WHEN** a client resets the server after creating a clock and a clock
  customer
- **THEN** listing test clocks is empty and the seeded main account is the only
  account
