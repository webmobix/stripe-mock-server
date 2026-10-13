# Proposal

## Why

Automated tests need connected accounts in a known state (active or pending)
without first driving the Stripe-compatible create + onboarding flow. Today the
only ways to get a connected account are `POST /v2/core/accounts` (requires a
key) followed by an account link and the hosted onboarding page; the internal
control plane can only read accounts, not seed them. This makes test setup
slower and couples tests to the onboarding UI.

## What Changes

- Add `POST /internal/accounts` to the internal control plane: seed a connected
  account directly and return its generated `acct_` id and keys.
- Accept only the basic fields the hosted onboarding form uses (business name,
  optional email/country) plus an explicit `state` of `active` or `pending`.
- `active` seeds `details_submitted=true`, `charges_enabled=true`,
  `payouts_enabled=true`, and all requested capabilities `active`.
- `pending` seeds `details_submitted=false`, `charges_enabled=false`,
  `payouts_enabled=false`, and capabilities `pending`.
- Each call creates a new account with freshly generated keys (no idempotency,
  no caller-supplied id).
- No events are emitted by the seed (it is a control-plane action, not an
  account lifecycle transition).

## Capabilities

### New Capabilities
<!-- None. Seeding connected accounts extends the existing internal control plane. -->

### Modified Capabilities
- `internal-control-plane`: adds a seeding endpoint for connected accounts with
  an `active`/`pending` state, and exposes the account's generated keys.

## Impact

- `src/internal.rs`: new `create_account` handler.
- `src/lib.rs`: new `POST /internal/accounts` route alongside the existing
  `GET /internal/accounts`.
- Reuses `src/seed.rs` account/key helpers or extracts shared account-row
  construction.
- `docs/internal.md`: document the endpoint.
- No changes to the Stripe-compatible API, config format, or database schema.
