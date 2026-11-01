# Spec Delta

## ADDED Requirements

### Requirement: Payment intent resolution endpoint

The server SHALL expose `POST /internal/payment_intents/{id}/resolve` on the control plane, taking a `status` of `succeeded`, `failed`, or `canceled`, so a test can settle a payment without a hosted page. The endpoint SHALL NOT require Stripe authentication, consistent with the rest of the control plane.

#### Scenario: Resolve from a test

- **WHEN** a client without an Authorization header resolves a payment intent as `succeeded`
- **THEN** the payment intent status becomes `succeeded` and the related invoice reflects the payment

#### Scenario: Invalid resolve status

- **WHEN** a client resolves a payment intent with an unsupported `status`
- **THEN** the server responds with an `invalid_request_error` and the payment intent status is unchanged
