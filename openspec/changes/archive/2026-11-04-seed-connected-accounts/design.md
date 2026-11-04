# Design

## Context

See proposal.md — Why. The internal control plane (`src/internal.rs`) currently
reads accounts and keys and reset/reseeds config state. Account creation lives in
the Stripe handlers: `src/api/v2.rs::create_account` (JSON, requires auth) and
`src/api/v1.rs::create_account` (form, requires auth), while
`src/seed.rs::seed_main_account` builds the platform account row from config. Of
these, only the v2 handler populates the richer `identity`/`configuration`/
`defaults` fields; none of the Stripe create paths mint `api_keys` for the new
account.

## Goals / Non-Goals

**Goals:**
- One no-auth internal call that yields an immediately usable connected account.
- Deterministic, minimal input: business name, optional email/country, and state.
- Seeded account is indistinguishable from one created through the Stripe API for
  read/projection purposes, plus it has keys.

**Non-Goals:**
- No `failed`/`restricted`/`rejected` state (only `active` and `pending`).
- No events emitted and no webhooks triggered by seeding.
- No change to the Stripe create endpoints, config schema, or DB schema.
- No caller-supplied id and no idempotency/upsert semantics.

## Decisions

**1. Endpoint is `POST /internal/accounts`, sharing the path with the existing
`GET`.** Axum routes method + path independently, and the plural collection path
matches the existing read endpoint. Response reuses `internal::account_with_keys`,
the same shape as `GET /internal/accounts[/{id}]`, so it includes `id` and both
keys. Alternative: a dedicated `/internal/seed` path; rejected because it would
fragment the account resource and duplicate response shaping.

**2. Put row construction in `src/seed.rs` as `seed_connected_account`.** Seeding
is already `seed.rs`'s responsibility and it owns `ensure_keys`. The handler stays
thin: validate input, call the seed helper, reload the row, shape the response.
Alternative: inline the SQL in `internal.rs`; rejected to avoid a third copy of
the account INSERT (v1, v2, main already exist).

**3. Mint keys via the existing `seed::ensure_keys`.** This is what makes the
seeded account usable with the Stripe API (any `kind='secret'` key authenticates
per `src/auth.rs`) and keeps `/internal/accounts/{id}/secrets` consistent with
the seed response. The seed helper inserts the account, then calls `ensure_keys`.
Alternative: reuse the v2 create path; rejected because it requires auth, emits
events, and does not mint keys.

**4. State maps to the same fields the onboarding page sets.** `active` sets
`details_submitted/charges_enabled/payouts_enabled = 1` and capability statuses
`active`; `pending` sets the flags to `0` and statuses `pending`. Capabilities
mirror the main account's shape: `merchant` configuration requesting
`card_payments` and `transfers`. This keeps `to_v1()`/`to_v2()` projections and
the onboarding-derived state consistent. `state` defaults to `pending` and any
other value is an invalid-parameter error. Alternative: a boolean `active`; a
named state reads better and leaves room for future states.

**5. No events.** Seeding is a control-plane fixture action, not an account
lifecycle transition; emitting `account.updated`/`v2.core.account.created` would
enqueue deliveries and make test setup non-deterministic. Tests that need events
should exercise the real create/onboarding flows.

**6. `name` is optional, defaulting to `"Mock Business"`** (the value the hosted
onboarding form pre-fills), so an empty/omitted name still yields a valid account.
`country` defaults to `"US"`, `email` defaults to null, `business_type` is
`"company"`. Inputs are trimmed; empty strings fall back to defaults.

## Risks / Trade-offs

- **Divergence from v2-created accounts (they lack keys)** → Acceptable and
  documented; this endpoint is explicitly a keyed test fixture. Reconciling the
  v2 path is out of scope.
- **Seeded accounts bypass onboarding events** → A test relying on
  `v2.core.account.created` will not see it; documented in the spec/response and
  in `docs/internal.md`.
- **Reset wipes seeded accounts** → Expected; `/internal/reset` clears all state,
  so tests seed after reset. No mitigation needed.

## Migration Plan

Additive endpoint and helper only; no data migration. Rollback is removing the
route and helper. Existing endpoints and specs are unchanged.
