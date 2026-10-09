# Proposal

## Why

Automated end-to-end tests of Stripe-backed flows currently depend on the real Stripe API and dashboard-configured webhooks, which is slow, rate-limited, non-reproducible, and cannot simulate failures. We need a local, stateful Stripe-compatible server that the official Stripe SDK can point at unchanged, plus mock Stripe-hosted pages, so full Connect + billing + checkout flows can run deterministically in CI.

## What Changes

- Add a new Rust service (`stripe-mock`) exposing a **drop-in Stripe-compatible REST API** on a single local process backed by a SQLite file, targeted by the official Stripe SDK via base-URL override. Object IDs use real Stripe prefixes (`acct_`, `prod_`, `price_`, `sub_`, `cus_`, `cs_test_`, `pi_`, `evt_`, `sk_test_`, `pk_test_`, `whsec_`).
- Implement two API-version conventions from the same server: **v1** (form-encoded, epoch timestamps) and **v2 core** (JSON bodies, RFC 3339-ms timestamps, distinct error envelope).
- Implement **Accounts v2 as canonical** (`/v2/core/accounts`, configurations/capabilities/`include`/default-null) with a derived **v1 account projection** (`/v1/accounts`), plus account links (v1 + v2) and a simplified hosted onboarding flow.
- Implement **catalog** (products, prices), **billing** (customers, subscriptions incl. `customer_account`), and **checkout** (checkout sessions in payment/subscription/setup modes) scoped per account via `Stripe-Account`.
- Embed **mock Stripe-hosted HTML pages** (no JS framework; Tailwind + daisyUI via CDN) for onboarding and checkout, each with a shared simulation panel (KYC, IBAN select, card payment) and correct `refresh_url` / `return_url` / `success_url` redirect behavior.
- Add a **config-driven webhook dispatcher** ("the server's dashboard"): event destinations defined in `config.toml` (`event_payload` `snapshot` | `thin`, `events_from` `@self` | `@accounts`, event allow-list or `*`), emitting v1 snapshot events into a per-destination FIFO queue with a fixed 200 ms delay, HMAC `Stripe-Signature`, and up to 3 inline retries; a delivery journal records every attempt.
- Add an **internal control plane** (`/internal/**`) exposing generated secrets (secret key, publishable key, per-target signing secrets), the main account, delivery inspection (`wait` / `flush` / `retry`), and database reset.
- Add a **TOML configuration file** describing the main account, database path, server/public base URL, and event destinations; the server seeds initial state at boot.
- Add unit tests for domain logic and integration tests that boot the server and drive the SDK/HTTP, plus documentation (README + config reference).

**Out of scope for MVP** (captured for later, see design.md): v2 thin events and dynamic event destinations (`/v1/webhook_endpoints`, `/v2/core/event_destinations`), connected-account usable API keys, server-side time advance / billing renewals, full v2 onboarding (`account_tokens` + `persons`), concurrent per-event delivery, invoices/tax, idempotency keys, and broad endpoint coverage beyond the ~25 endpoints listed above.

## Capabilities

### New Capabilities

- `api-foundations`: Stripe protocol compatibility layer — request authentication (`Bearer`/`Basic` API keys) and account resolution (`Stripe-Account`), v1 and v2 error envelopes, list envelopes, ID/key generation with correct prefixes, and per-version serialization (epoch vs RFC 3339).
- `accounts`: Accounts v2 canonical model (configurations, capabilities, requirements, `include`, default-null behavior) with a derived v1 account projection; account links (v1 + v2) and onboarding state transitions.
- `catalog`: products and prices, account-scoped.
- `billing`: customers and subscriptions, including `customer_account` references to account-backed customers.
- `checkout`: checkout sessions across payment/subscription/setup modes and the completion flow that produces payment intents/subscriptions and emits events.
- `webhook-dispatch`: event emission (v1 snapshot) and config-driven destination fan-out by payload format, scope, and event filter, with HMAC signing, a per-destination FIFO queue at a fixed 200 ms delay, retries, and a delivery journal.
- `hosted-simulation-ui`: embedded HTML pages for connected-account onboarding and checkout, with a shared simulation panel and correct redirect handling.
- `internal-control-plane`: `/internal/**` endpoints for generated secrets, main account, delivery inspection/wait/flush/retry, and state reset.
- `server-configuration`: the TOML config schema (main account, database, server/public URL, event destinations) and boot-time seeding.

### Modified Capabilities

- None (greenfield; no existing specs).

## Impact

- New Rust crate/binary `stripe-mock` (currently a hello-world `src/main.rs`); all application code and tests are new.
- New dependencies: `tokio`, `axum`, `sqlx` (SQLite), `serde`/`serde_json`, `toml`, HMAC/SHA-256 (`hmac` + `sha2`), time/ID helpers.
- New runtime artifacts: a SQLite database file and a TOML config file; no external services or network dependencies beyond webhook callbacks to configured targets.
- Consumes `docs/openapi.spec3.json` as the schema contract for the implemented subset; does not generate code from it.
- Establishes project architecture, module layout, and test strategy for subsequent increments.
