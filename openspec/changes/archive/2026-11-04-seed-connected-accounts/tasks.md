# Tasks

## 1. Seed helper

- [x] 1.1 Add `seed::seed_connected_account(pool, name, email, country, active)` in `src/seed.rs`: insert a non-main `accounts` row with `business_type='company'`, merchant configuration requesting `card_payments`/`transfers`, and `active`/`pending` flags + capability statuses, then call `ensure_keys`; verify with a `#[tokio::test]` that asserts the flags, capability JSON, and presence of one `secret` and one `publishable` key.

## 2. Internal endpoint

- [x] 2.1 Add `internal::create_account` in `src/internal.rs` and register `POST /internal/accounts` in `build_router` (`src/lib.rs`); parse `name`/`email`/`country`/`state`, default `name` to `"Mock Business"`, `country` to `"US"`, and `state` to `pending`, call the seed helper, and respond with `account_with_keys`; verify with an integration test that a valid body returns an `acct_` id plus `sk_test_`/`pk_test_` keys.
- [x] 2.2 Reject an unknown `state` with an invalid-parameter error and create no account; verify active vs pending seeding produces the expected `details_submitted`/`charges_enabled`/`payouts_enabled` and capability statuses, and that two calls return distinct ids.
- [x] 2.3 Verify seeded keys are usable: `/internal/accounts/{id}/secrets` returns the same keys, `GET /v1/accounts/{id}` reflects the seeded state, and a Stripe API request bearing the seeded secret key authenticates.
- [x] 2.4 Document `POST /internal/accounts` in `docs/internal.md` with an active and a pending example; verify the documented curl command runs as written and returns an `acct_` id.

## 3. Integration verification

- [x] 3.1 Run `cargo test` and `cargo clippy --all-targets` on the full suite and confirm no regressions; verify the seed-then-use flow works after `POST /internal/reset`.
