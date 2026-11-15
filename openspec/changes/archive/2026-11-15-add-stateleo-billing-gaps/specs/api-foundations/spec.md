# Spec Delta

## ADDED Requirements

### Requirement: Idempotency keys

The server SHALL honor an `Idempotency-Key` header on `POST`, `DELETE`, and `PATCH` requests under `/v1/`. Responses SHALL be cached by key, credential, method, path, and body, and replaying the same parameters SHALL return the cached response with `Idempotent-Replay: true`. A different body with the same key SHALL return an `idempotency_error` without running the handler. Server errors SHALL NOT be cached and reset SHALL clear keys.

#### Scenario: Replay a request

- **WHEN** a client repeats a mutation with the same `Idempotency-Key` and body
- **THEN** the same object is returned and no second object is created

#### Scenario: Conflicting parameters

- **WHEN** a client reuses an `Idempotency-Key` with a different body
- **THEN** the response is an `idempotency_error` and no handler runs

#### Scenario: Reset clears keys

- **WHEN** a client calls reset and then replays a previously used key
- **THEN** the request is executed as a new request
