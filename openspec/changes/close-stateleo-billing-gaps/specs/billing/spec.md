# Spec Delta

## ADDED Requirements

### Requirement: Subscription billing fields

Subscriptions SHALL report `latest_invoice`, `default_payment_method`, `cancel_at`, `canceled_at`, and `ended_at`. `latest_invoice` SHALL name the most recent invoice raised for the subscription and SHALL be null before one exists. `cancel_at` SHALL be set to the current period end while `cancel_at_period_end` is true and null otherwise. `canceled_at` and `ended_at` SHALL be set when the subscription becomes canceled. `latest_invoice` and `default_payment_method` SHALL support `expand[]`.

#### Scenario: Latest invoice on a subscription

- **WHEN** a client creates a subscription and retrieves it
- **THEN** `latest_invoice` is the id of its `subscription_create` invoice

#### Scenario: Latest invoice in the created event

- **WHEN** a subscription is created
- **THEN** the `customer.subscription.created` event's `data.object.latest_invoice` names its first invoice

#### Scenario: Expanded latest invoice and card

- **WHEN** a client retrieves a subscription with `expand[]=latest_invoice` and `expand[]=default_payment_method`
- **THEN** both fields are full objects rather than ids

#### Scenario: Cancel at period end records cancel_at

- **WHEN** a client updates a subscription with `cancel_at_period_end=true`
- **THEN** `cancel_at` equals the subscription's `current_period_end`

### Requirement: Cancel at period end when the period ends

When `POST /internal/subscriptions/{id}/advance` is called for a subscription with `cancel_at_period_end=true`, the server SHALL cancel it instead of raising a renewal: the status SHALL become `canceled`, `canceled_at` and `ended_at` SHALL be set, a `customer.subscription.deleted` event SHALL be created, and no invoice SHALL be raised.

#### Scenario: Advance cancels instead of renewing

- **WHEN** a test advances a subscription whose `cancel_at_period_end` is true
- **THEN** the subscription is `canceled`, a `customer.subscription.deleted` event exists, and the response `invoice` is null

#### Scenario: No renewal invoice

- **WHEN** an advancing subscription is canceled at the period end
- **THEN** listing its invoices returns no `subscription_cycle` invoice for that period

### Requirement: Billing cycle anchor configuration

`POST /v1/subscriptions` SHALL accept `billing_cycle_anchor_config[day_of_month]` as an integer from 1 to 31 and SHALL echo it back on the subscription's `billing_cycle_anchor_config`. The server SHALL reject a value outside that range with an `invalid_request_error` and create no subscription.

#### Scenario: Anchor day is echoed

- **WHEN** a client creates a subscription with `billing_cycle_anchor_config[day_of_month]=15`
- **THEN** the response reports `billing_cycle_anchor_config.day_of_month=15`

#### Scenario: Invalid anchor day

- **WHEN** a client creates a subscription with `billing_cycle_anchor_config[day_of_month]=32`
- **THEN** the request is rejected with an `invalid_request_error` naming the parameter
