# hosted-simulation-ui Specification

## Purpose

Serves the embedded mock Stripe-hosted HTML pages for connected-account onboarding and checkout, including controls to simulate the actions a real user would perform.

## Requirements

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

### Requirement: Hosted invoice page

The mock SHALL serve the hosted page of an invoice at `GET /mock/invoices/{id}` and a PDF of the same summary at `GET /mock/invoices/{id}/pdf`, so an application can send its customer to the invoice's `hosted_invoice_url`. The page SHALL render the invoice's id, its status, its lines, and its totals, and both SHALL answer `not_found` for an invoice that does not exist.

#### Scenario: The page renders the invoice

- **WHEN** a browser opens the hosted page of an invoice
- **THEN** the page shows the invoice's id, its status, its lines, and its totals

#### Scenario: The PDF is served

- **WHEN** a client requests the PDF of an invoice
- **THEN** the response is a PDF document holding the invoice's summary

### Requirement: Static demo checkout session

The server SHALL expose a well-known checkout session id so the mock checkout
page can be opened and completed without first creating a session through the
API. The id SHALL be `cs_test_static`, the page SHALL render a fixed demo line
item and total, and requesting the page while the session is `complete` or
`expired` SHALL reset it to a fresh `open` demo session so it is always
available for testing.

#### Scenario: Open the page without creating a session

- **WHEN** a browser opens `/mock/checkout/cs_test_static` on a freshly started server
- **THEN** an HTML checkout page renders with a demo line item, a total, and the simulation controls

#### Scenario: Complete the static page

- **WHEN** the static page is submitted with a successful card simulation
- **THEN** the session becomes `complete`, a succeeded payment intent exists, and the completion events are created

#### Scenario: Reusable after completion

- **WHEN** the browser opens `/mock/checkout/cs_test_static` again after the session was completed or expired
- **THEN** a fresh `open` demo session renders instead of the redirect or error page

### Requirement: Billing portal page

The mock SHALL serve the hosted page of a billing portal session at `GET /mock/billing-portal/{id}`. The page SHALL render the session's customer, list that customer's subscriptions with their id, status, item quantities, and current period end, and provide a link back to the session's `return_url`. A session id that does not exist SHALL render the mock's not-found page.

#### Scenario: Portal page renders

- **WHEN** a browser opens the URL of a billing portal session
- **THEN** an HTML page renders the customer and that customer's subscriptions

#### Scenario: Return to the application

- **WHEN** the billing portal page is rendered
- **THEN** it contains a link to the session's `return_url`

#### Scenario: Unknown portal session

- **WHEN** a browser opens a billing portal URL whose session does not exist
- **THEN** the mock's not-found page is returned

#### Scenario: No subscriptions

- **WHEN** the customer has no subscriptions
- **THEN** the page renders without an error and shows an empty subscription list
