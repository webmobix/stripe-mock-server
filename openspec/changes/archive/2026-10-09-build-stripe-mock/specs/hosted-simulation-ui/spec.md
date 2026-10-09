# Spec Delta

## Purpose

Serves the embedded mock Stripe-hosted HTML pages for connected-account onboarding and checkout, including controls to simulate the actions a real user would perform.

## ADDED Requirements

### Requirement: Embedded pages without a JS framework

The server SHALL serve the onboarding and checkout pages as server-rendered HTML with no client-side framework, loading Tailwind and daisyUI from a CDN.

#### Scenario: Checkout page renders

- **WHEN** a browser opens a checkout session URL
- **THEN** an HTML page renders with CDN-provided styling and no framework bundle

### Requirement: Onboarding page

The mock onboarding page SHALL collect minimal business information and provide controls to simulate KYC verification outcomes, and SHALL redirect to the link's `return_url` on completion or `refresh_url` when the link is stale.

#### Scenario: Complete onboarding

- **WHEN** the user submits the onboarding form and simulates verified KYC
- **THEN** the account is updated and the browser is redirected to `return_url`

### Requirement: Checkout page

The mock checkout page SHALL render the session summary and a minimal payment form, and SHALL provide controls to simulate card payment success and failure plus alternative payment methods.

#### Scenario: Simulate successful card

- **WHEN** the user activates the successful card simulation
- **THEN** the session completes and the browser is redirected to `success_url`

### Requirement: Shared simulation panel

Every mock page SHALL present a consistent simulation panel offering the applicable controls among KYC verification, IBAN selection, and card payment.

#### Scenario: Controls present

- **WHEN** a user views a mock hosted page
- **THEN** the applicable simulation controls are visible and labeled without development tooling
