# Spec Delta

## ADDED Requirements

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
