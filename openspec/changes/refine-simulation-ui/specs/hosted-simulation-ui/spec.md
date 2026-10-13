# Spec Delta

## MODIFIED Requirements

### Requirement: Checkout page

The mock checkout page SHALL render the session summary and a minimal payment form, and SHALL provide labeled controls to simulate payment success and failure plus alternative payment methods, together with a cancel control adjacent to the payment simulation controls.

#### Scenario: Simulate successful card

- **WHEN** the user activates the successful payment simulation
- **THEN** the session completes and the browser is redirected to `success_url`

#### Scenario: Simulate declined payment

- **WHEN** the user activates the declined payment simulation
- **THEN** the session stays open and the page re-renders with a decline notice

### Requirement: Shared simulation panel

Every mock page SHALL present the applicable simulation controls among KYC verification, IBAN selection, and payment simulation, without a panel heading or a developer-mode disclaimer.

#### Scenario: Controls present

- **WHEN** a user views a mock hosted page
- **THEN** the applicable simulation controls are visible

#### Scenario: No developer-mode messaging

- **WHEN** a user views a mock hosted page
- **THEN** the page does not show a "Simulation panel" heading or a "developer-mode simulator" disclaimer

## ADDED Requirements

### Requirement: No development-tooling chrome

The shared page shell SHALL NOT display a "TEST MODE" badge or a statement that the tool must not be used in production.

#### Scenario: Tooling chrome absent

- **WHEN** any mock hosted page is rendered
- **THEN** the HTML contains no "TEST MODE" badge and no "Local simulation tool. Never use in production." text

### Requirement: Automation-stable checkout controls

The checkout payment and cancel controls SHALL expose stable element identifiers so automation can address them independently of their labels.

#### Scenario: Payment control ids

- **WHEN** the checkout page renders
- **THEN** the successful payment control has id `payment-success` and the declined payment control has id `payment-decline`

#### Scenario: Cancel control id

- **WHEN** the checkout page renders
- **THEN** the cancel control has id `payment-cancel` and navigates to the session cancel URL
