# Spec Delta

## ADDED Requirements

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
