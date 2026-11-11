# Spec Delta

## ADDED Requirements

### Requirement: Update subscriptions

The server SHALL update subscriptions at `POST /v1/subscriptions/{id}`. The request SHALL accept `items[]` entries carrying an item `id` and a new `quantity`, and MAY accept `cancel_at_period_end` and `metadata`. A request MAY omit the item id and name a `price` instead when the subscription has exactly one item.

#### Scenario: Change an item quantity

- **WHEN** a client updates a subscription with an existing item id and a `quantity` of 5
- **THEN** that item's `quantity` is 5 and the subscription status is unchanged

#### Scenario: Change a single item by price

- **WHEN** a client updates a one-item subscription naming its price and a new quantity
- **THEN** the item's quantity is updated

#### Scenario: Cancel at period end

- **WHEN** a client updates a subscription with `cancel_at_period_end=true`
- **THEN** the subscription reports `cancel_at_period_end` true and keeps its status

#### Scenario: Unknown item

- **WHEN** a client updates a subscription with an item id that does not belong to it
- **THEN** the server responds with an `invalid_request_error` and changes nothing

#### Scenario: Invalid quantity

- **WHEN** a client updates an item with a quantity that is not a positive integer
- **THEN** the server responds with an `invalid_request_error` naming `quantity`

#### Scenario: Update creates an event

- **WHEN** a subscription update succeeds
- **THEN** a `customer.subscription.updated` event exists whose `data.object` is the updated subscription

### Requirement: Subscription update prorations

The server SHALL accept `proration_behavior` of `create_prorations`, `none`, or `always_invoice`, defaulting to `create_prorations`. When an item quantity changes and prorations are enabled, the update response SHALL include a `prorations` array with an entry per changed item holding the item id, price id, quantity delta, prorated amount in minor units, currency, and the period it covers. No invoice SHALL be created.

#### Scenario: Proration for an increased quantity

- **WHEN** a client raises an item quantity with `proration_behavior=create_prorations`
- **THEN** the response `prorations` contains one entry with a positive amount and the quantity delta

#### Scenario: Proration for a decreased quantity

- **WHEN** a client lowers an item quantity with prorations enabled
- **THEN** the proration amount is negative

#### Scenario: Prorations disabled

- **WHEN** a client changes a quantity with `proration_behavior=none`
- **THEN** the response `prorations` is empty and no amount is computed

#### Scenario: Always invoice does not create an invoice

- **WHEN** a client changes a quantity with `proration_behavior=always_invoice`
- **THEN** the proration figures are returned and no invoice is created

#### Scenario: Unsupported proration behavior

- **WHEN** a client sends a `proration_behavior` other than the three supported values
- **THEN** the server responds with an `invalid_request_error` and changes nothing

### Requirement: Subscription item period fields

Every subscription item in `items.data` SHALL carry its own `current_period_start` and `current_period_end` epoch fields, together with its item id, `object` of `subscription_item`, price, and quantity. The periods SHALL be present on create, retrieve, list, and update responses.

#### Scenario: Item periods on creation

- **WHEN** a subscription is created
- **THEN** each item reports `current_period_start` and `current_period_end` matching the subscription period

#### Scenario: Item periods after an update

- **WHEN** a subscription item's quantity is updated
- **THEN** the item still reports its period fields

### Requirement: Retrieve canceled subscriptions

`GET /v1/subscriptions/{id}` SHALL return a canceled subscription with `status` of `canceled` instead of responding `not_found`. Listing subscriptions SHALL continue to include canceled subscriptions.

#### Scenario: Retrieve a canceled subscription

- **WHEN** a client retrieves a subscription it canceled
- **THEN** the response is that subscription with `status` of `canceled`

#### Scenario: Canceled subscription in the list

- **WHEN** a client lists subscriptions after canceling one
- **THEN** the canceled subscription appears in the list

### Requirement: Billing portal sessions

The server SHALL create billing portal sessions at `POST /v1/billing_portal/sessions` from a `customer` and a `return_url`. The response SHALL be a `billing_portal.session` with a `bps_` id, `created`, the customer, the return url, and a `url` pointing at the mock billing portal page. The server SHALL NOT create an event.

#### Scenario: Create a portal session

- **WHEN** a client creates a portal session for an existing customer with a return url
- **THEN** the response is a `bps_` session whose `url` points at the mock billing portal

#### Scenario: Unknown customer

- **WHEN** a client creates a portal session for a customer that does not exist for the account
- **THEN** the server responds with a `not_found` error and creates no session

#### Scenario: Missing return url

- **WHEN** a client creates a portal session without a `return_url`
- **THEN** the server responds with an `invalid_request_error` naming `return_url`

#### Scenario: No event for a portal session

- **WHEN** a client creates a billing portal session
- **THEN** no event is created
