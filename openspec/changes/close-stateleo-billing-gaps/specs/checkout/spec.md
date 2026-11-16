# Spec Delta

## ADDED Requirements

### Requirement: Checkout parameter echoes

`POST /v1/checkout/sessions` SHALL accept `payment_method_collection` of `always` or `if_required`, defaulting to `always`, and SHALL echo it on the session. It SHALL accept `automatic_tax[enabled]` as a boolean, defaulting to false, and SHALL report `automatic_tax` as an object carrying the enabled flag and `status=null`. Both values SHALL survive retrieval and completion.

#### Scenario: Echo payment method collection

- **WHEN** a client creates a session with `payment_method_collection=if_required`
- **THEN** the session reports `payment_method_collection=if_required`

#### Scenario: Default payment method collection

- **WHEN** a client creates a session without `payment_method_collection`
- **THEN** the session reports `payment_method_collection=always`

#### Scenario: Echo automatic tax

- **WHEN** a client creates a session with `automatic_tax[enabled]=true`
- **THEN** the session reports `automatic_tax.enabled=true` and `automatic_tax.status=null`

#### Scenario: Echoes survive completion

- **WHEN** a session created with both parameters completes
- **THEN** retrieving it still reports the supplied values

## MODIFIED Requirements

### Requirement: Saved card after completion

Completing a session for a customer SHALL create a saved card payment method and make it the customer's default. A `subscription`-mode completion SHALL also create the subscription's paid first invoice and set the saved card as that subscription's `default_payment_method`.

#### Scenario: Card saved

- **WHEN** a session completes for a customer
- **THEN** the customer's `invoice_settings.default_payment_method` is a Visa card ending `4242`

#### Scenario: Subscription first invoice

- **WHEN** a `subscription`-mode session completes
- **THEN** the subscription has a paid `subscription_create` invoice

#### Scenario: Subscription default card

- **WHEN** a `subscription`-mode session completes for a customer
- **THEN** the created subscription's `default_payment_method` is the saved card
