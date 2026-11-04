# payments Specification

## Purpose

Lets an application create a payment intent against a customer or an invoice and settle it from a test, so that the amounts and status of the invoice it pays are updated.

## Requirements

### Requirement: Create and retrieve payment intents

The server SHALL create payment intents at `POST /v1/payment_intents` with an `amount` and a `currency`, optionally referencing a `customer` or an `invoice`, and SHALL retrieve them at `GET /v1/payment_intents/{id}`. A new payment intent has a `pi_` id, `object` of `payment_intent`, and `status` of `requires_payment_method`.

#### Scenario: Create a payment intent

- **WHEN** a client creates a payment intent with `amount` 1000 and `currency` `usd`
- **THEN** the response is a payment intent with a `pi_` id, the stored `amount` and `currency`, and `status` of `requires_payment_method`

#### Scenario: Create a payment intent without an amount

- **WHEN** a client creates a payment intent without an `amount`
- **THEN** the server responds with an `invalid_request_error` naming `amount`

#### Scenario: Attach a payment intent to an invoice

- **WHEN** a client creates a payment intent with `invoice` referencing an invoice
- **THEN** the payment intent records that invoice and the invoice lists the payment intent

#### Scenario: Unknown invoice

- **WHEN** a client creates a payment intent with an `invoice` that does not exist for the account
- **THEN** the server responds with a `not_found` error and creates no payment intent

### Requirement: List payment intents

The server SHALL list payment intents at `GET /v1/payment_intents` with the list envelope, filtering by `customer` and `invoice` where provided.

#### Scenario: List payment intents for an invoice

- **WHEN** a client lists payment intents with `invoice=<in_...>`
- **THEN** only that invoice's payment intents are returned

### Requirement: Resolve a payment intent

The server SHALL settle a payment intent at `POST /internal/payment_intents/{id}/resolve` with a `status` of `succeeded`, `failed`, or `canceled`. The server SHALL set the payment intent `status` accordingly and SHALL create a `payment_intent.succeeded`, `payment_intent.payment_failed`, or `payment_intent.canceled` event.

#### Scenario: Resolve a payment intent as succeeded

- **WHEN** a client resolves a payment intent with `status` of `succeeded`
- **THEN** the payment intent `status` is `succeeded` and a `payment_intent.succeeded` event exists

#### Scenario: Resolve a payment intent as failed

- **WHEN** a client resolves a payment intent with `status` of `failed`
- **THEN** the payment intent `status` is `failed` and a `payment_intent.payment_failed` event exists

#### Scenario: Resolve with an unsupported status

- **WHEN** a client resolves a payment intent with a `status` other than `succeeded`, `failed`, or `canceled`
- **THEN** the server responds with an `invalid_request_error` and the payment intent status is unchanged

#### Scenario: Resolve an unknown payment intent

- **WHEN** a client resolves a payment intent id that does not exist
- **THEN** the server responds with a `not_found` error

### Requirement: Successful payment intent settles its invoice

When a payment intent that references an invoice is resolved as `succeeded`, the server SHALL add its `amount` to the invoice `amount_paid`, SHALL set the invoice `amount_remaining` to `amount_due` minus `amount_paid` (not below zero), and SHALL create an `invoice.payment_succeeded` event.

#### Scenario: Payment covers the invoice

- **WHEN** a payment intent of the invoice's full `amount_due` attached to an invoice is resolved as `succeeded`
- **THEN** the invoice `status` is `paid`, `amount_paid` equals `amount_due`, and `amount_remaining` is 0

#### Scenario: Partial payment

- **WHEN** a payment intent for less than the invoice's `amount_due` is resolved as `succeeded`
- **THEN** the invoice `amount_paid` increases by that amount, `amount_remaining` decreases accordingly, and the invoice `status` remains `open`

### Requirement: Invoice is marked paid when fully covered

When a successful payment intent brings an invoice's `amount_paid` to its `amount_due`, the server SHALL set the invoice `status` to `paid` with `paid_at` set, and SHALL create an `invoice.paid` event.

#### Scenario: Invoice becomes paid

- **WHEN** a payment intent settles the remaining balance of an invoice
- **THEN** the invoice `status` is `paid`, `paid_at` is set, and an `invoice.paid` event exists

### Requirement: Failed payment leaves its invoice unchanged

When a payment intent that references an invoice is resolved as `failed`, the server SHALL leave the invoice `amount_paid`, `amount_remaining`, and `status` unchanged and SHALL create an `invoice.payment_failed` event.

#### Scenario: Failed payment leaves the invoice unchanged

- **WHEN** a payment intent attached to an invoice is resolved as `failed`
- **THEN** the invoice `amount_paid`, `amount_remaining`, and `status` are unchanged and an `invoice.payment_failed` event exists

### Requirement: Payment intent creation event

The server SHALL create a `payment_intent.created` event in the same transaction as the payment intent it describes.

#### Scenario: Payment intent created event

- **WHEN** a client creates a payment intent
- **THEN** a `payment_intent.created` event exists whose `data.object` is that payment intent
