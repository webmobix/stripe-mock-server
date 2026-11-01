# Spec Delta

## ADDED Requirements

### Requirement: Minimum customer and invoice event set

For the customer and invoice flow the server SHALL emit the event types `customer.created`, `customer.updated`, `customer.deleted`, `invoice.created`, `invoice.finalized`, `invoice.paid`, `invoice.payment_succeeded`, `invoice.payment_failed`, `invoice.voided`, `invoice.marked_uncollectible`, `payment_intent.created`, `payment_intent.succeeded`, `payment_intent.payment_failed`, and `payment_intent.canceled`, and SHALL deliver them to snapshot destinations that allow them.

#### Scenario: Event types reach an allowing destination

- **WHEN** a client creates a customer and a destination enables `customer.*` and `invoice.*`
- **THEN** the destination receives `customer.created` and every invoice event emitted by the flow it triggers

#### Scenario: Payment failure event

- **WHEN** a payment intent attached to an invoice is resolved as `failed`
- **THEN** a `payment_intent.payment_failed` event is emitted and delivered
