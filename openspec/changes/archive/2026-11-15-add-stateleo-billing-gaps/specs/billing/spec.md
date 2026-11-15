# Spec Delta

## ADDED Requirements

### Requirement: First and renewal subscription invoices

Creating a subscription SHALL raise a paid `subscription_create` invoice with one line per item. `POST /internal/subscriptions/{id}/advance` SHALL end the period with outcome `paid` or `failed` and raise a `subscription_cycle` invoice. Paid marks it paid and reactivates the subscription; failed leaves it open, increments `attempt_count`, and sets `past_due`. An existing open invoice SHALL be retried. Metered lines SHALL bill usage from the ended period.

#### Scenario: First invoice

- **WHEN** a client creates a subscription
- **THEN** its first invoice is paid with `billing_reason=subscription_create`

#### Scenario: Paid renewal

- **WHEN** a test advances a subscription with `outcome=paid`
- **THEN** the period advances, a `subscription_cycle` invoice is paid, and the subscription is `active`

#### Scenario: Failed renewal and recovery

- **WHEN** a test advances with `outcome=failed` and then `outcome=paid`
- **THEN** the first call leaves an open invoice and a `past_due` subscription, and the second pays the same invoice and reactivates the subscription

### Requirement: Default card after checkout

Completing a checkout session for a customer SHALL save a simulated Visa card as a `pm_` payment method and set it as `invoice_settings.default_payment_method` on the customer. The card SHALL be readable from the payment method endpoints and through expansion.

#### Scenario: Card on the customer

- **WHEN** a checkout session completes for a customer
- **THEN** the customer's default payment method is a card ending `4242`

#### Scenario: Expanded default card

- **WHEN** a client retrieves the customer with `expand[]=invoice_settings.default_payment_method`
- **THEN** the field is the full payment method object
