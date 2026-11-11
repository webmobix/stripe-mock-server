# billing Specification

## Purpose

Manages customers and subscriptions, including account-backed customers, so recurring billing flows can be exercised locally.

## Requirements

### Requirement: Create and retrieve customers

The server SHALL create customers at `POST /v1/customers` and retrieve them at `GET /v1/customers/{id}` with a `cus_` id. A customer MAY carry tax identification data supplied as `tax_id_data[][type]` and `tax_id_data[][value]`; the server SHALL store each entry as a `tax_ids` object with a `txi_` id, the given `type` and `value`, and expose them on the customer. A customer MAY carry a default `currency`; the server SHALL store it and expose it on the customer.

#### Scenario: Create customer

- **WHEN** a client creates a customer with an email
- **THEN** the response is a `customer` with a `cus_` id and the email stored

#### Scenario: Create customer with tax identification

- **WHEN** a client creates a customer with `tax_id_data[0][type]` of `eu_vat` and `tax_id_data[0][value]` of `DE123456789`
- **THEN** the customer `tax_ids` contains one entry with that type and value and a `txi_` id

#### Scenario: Tax identification without a value

- **WHEN** a client creates a customer with a `tax_id_data[][type]` but no matching `value`
- **THEN** the server responds with an `invalid_request_error` naming the missing value and creates no customer

#### Scenario: Create customer with a currency

- **WHEN** a client creates a customer with a `currency` of `eur`
- **THEN** the customer `currency` is `eur`

### Requirement: Create subscriptions

The server SHALL create subscriptions at `POST /v1/subscriptions` from one or more `items[][price]` references and SHALL return a `sub_` object with `status`, `items`, `customer` or `customer_account`, and period fields.

#### Scenario: Create subscription for a customer

- **WHEN** a client creates a subscription with a recurring price and a `customer`
- **THEN** the response has a subscription status and the price present in `items.data`

#### Scenario: Create subscription for an account-backed customer

- **WHEN** a client supplies `customer_account` with an `acct_` id
- **THEN** the subscription records that account as its customer

### Requirement: Subscription status lifecycle

The server SHALL transition subscription status on creation, update, and cancellation, and SHALL create `customer.subscription.created`, `.updated`, and `.deleted` events.

#### Scenario: Cancel subscription

- **WHEN** a client cancels a subscription
- **THEN** the status becomes `canceled` and a `customer.subscription.deleted` event is created

### Requirement: Retrieve and list subscriptions

The server SHALL retrieve subscriptions at `GET /v1/subscriptions/{id}` and list them with the list envelope, filtering by `customer` or `customer_account` where provided.

#### Scenario: List by customer

- **WHEN** a client lists subscriptions with `customer=<cus_...>`
- **THEN** only that customer's subscriptions are returned

### Requirement: List customers

The server SHALL list customers at `GET /v1/customers` with the list envelope, filtering by `email` where provided.

#### Scenario: List customers by email

- **WHEN** a client lists customers with `email=buyer@example.com`
- **THEN** only customers with that email are returned

### Requirement: Update customers

The server SHALL update a customer at `POST /v1/customers/{id}`, accepting `email`, `name`, `metadata`, `currency`, and additional `tax_id_data[]` entries. The server SHALL create a `customer.updated` event.

#### Scenario: Update a customer

- **WHEN** a client updates a customer's name
- **THEN** the response is the customer with the new name and a `customer.updated` event exists

#### Scenario: Add a tax id by update

- **WHEN** a client updates a customer with a new `tax_id_data[][type]` and `tax_id_data[][value]`
- **THEN** the customer `tax_ids` contains the original entries plus the new one

#### Scenario: Change a customer's currency

- **WHEN** a client updates a customer with a `currency` of `gbp`
- **THEN** the customer `currency` is `gbp`

#### Scenario: A malformed currency

- **WHEN** a client creates or updates a customer with a `currency` that is not a three-letter ISO code
- **THEN** the server responds with an `invalid_request_error` naming `currency` and the customer is unchanged

### Requirement: Delete customers

The server SHALL delete a customer at `DELETE /v1/customers/{id}`, removing it from subsequent reads and lists. The server SHALL create a `customer.deleted` event.

#### Scenario: Delete a customer

- **WHEN** a client deletes a customer
- **THEN** retrieving that customer returns a `not_found` error and a `customer.deleted` event exists

### Requirement: Customer creation event

The server SHALL create a `customer.created` event in the same transaction as the customer it describes.

#### Scenario: Customer created event

- **WHEN** a client creates a customer
- **THEN** a `customer.created` event exists whose `data.object` is that customer

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
