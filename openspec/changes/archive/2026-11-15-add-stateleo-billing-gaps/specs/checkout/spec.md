# Spec Delta

## ADDED Requirements

### Requirement: Embedded checkout

`POST /v1/checkout/sessions` SHALL accept `ui_mode=embedded_page` (and the `embedded` alias), requiring `return_url`. An embedded session SHALL report `ui_mode`, `return_url`, and a `client_secret`, and SHALL have `url=null`. Hosted sessions SHALL default to `ui_mode=hosted_page`, keep `url`, and report `client_secret=null`. Completion of an embedded session SHALL redirect to `return_url` with `{CHECKOUT_SESSION_ID}` substituted.

#### Scenario: Create embedded session

- **WHEN** a client creates a session with `ui_mode=embedded_page` and a `return_url`
- **THEN** the response carries a `client_secret`, no `url`, and the supplied `return_url`

#### Scenario: Missing return url

- **WHEN** a client creates an embedded session without `return_url`
- **THEN** the request is rejected with a missing-parameter error

#### Scenario: Embedded completion redirects to return url

- **WHEN** an embedded session completes
- **THEN** the browser is redirected to `return_url`

### Requirement: Subscription metadata on checkout

A `subscription`-mode session SHALL accept `subscription_data[metadata]` and copy it to the subscription created on completion.

#### Scenario: Metadata copied

- **WHEN** a session is created with `subscription_data[metadata][plan]=team` and completes
- **THEN** the created subscription has `metadata.plan=team`

### Requirement: Saved card after completion

Completing a session for a customer SHALL create a saved card payment method and make it the customer's default. A `subscription`-mode completion SHALL also create the subscription's paid first invoice.

#### Scenario: Card saved

- **WHEN** a session completes for a customer
- **THEN** the customer's `invoice_settings.default_payment_method` is a Visa card ending `4242`

#### Scenario: Subscription first invoice

- **WHEN** a `subscription`-mode session completes
- **THEN** the subscription has a paid `subscription_create` invoice
