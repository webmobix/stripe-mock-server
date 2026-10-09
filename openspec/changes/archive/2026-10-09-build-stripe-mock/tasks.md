# Tasks

## 1. Project setup, configuration, and stack spike

- [x] 1.1 Add runtime dependencies (`tokio`, `axum`, `sqlx` with SQLite, `serde`, `serde_json`, `toml`, `hmac`, `sha2`, `uuid`, `time`/`chrono`, `tracing`) to `Cargo.toml`; verify `cargo build` succeeds.
- [x] 1.2 Define the TOML config model and a committed sample `config.toml` (server, database, main account, event destinations) per design D13; verify a unit test parses the sample into the typed model.
- [x] 1.3 Add the SQLite pool with WAL, a migrations runner, and the initial schema (accounts, keys, catalog, billing, checkout, events, targets, deliveries); verify migrations apply cleanly to a fresh database file and are idempotent on re-run.
- [x] 1.4 Implement `src/main.rs` boot (load config → pool → migrate → seed → bind axum router); verify the server starts on the configured address and answers a health probe.
- [x] 1.5 Spike: point the official Stripe SDK at the mock base URL and confirm a request reaches the server and is parsed; verify with a captured request log, and record the SDK/language used in the README. This validates the drop-in guarantee before breadth is built.
- [x] 1.6 Write `README.md` and a config reference documenting install, run, config fields, and the internal endpoints; verify every documented command runs as written from a clean checkout.

## 2. API foundations (spec: `api-foundations`)

- [x] 2.1 Implement `ids.rs` prefixed generators for `acct_`, `prod_`, `price_`, `cus_`, `sub_`, `cs_test_`, `pi_`, `evt_`, `sk_test_`, `pk_test_`, `whsec_`; verify unit tests assert the prefix and uniqueness.
- [x] 2.2 Implement `time.rs` conversion between epoch seconds (v1) and RFC 3339-ms UTC strings (v2); verify round-trip unit tests including millisecond precision.
- [x] 2.3 Implement the v1 error envelope and the v2 error envelope with correct HTTP status mapping; verify handler tests for `invalid_request_error`, authentication, and not-found cases on both versions.
- [x] 2.4 Implement the list-envelope helper with `limit`, `starting_after`, `ending_before`, `has_more`, newest-first ordering; verify tests with more items than the page size.
- [x] 2.5 Implement the form-encoded parser including Stripe bracketed notation (`line_items[0][price]`, nested arrays); verify unit tests cover flat, nested, and repeated keys.
- [x] 2.6 Implement authentication and account-context resolution (`Bearer`/`Basic` key lookup plus `Stripe-Account` authorization); verify tests for a valid key, an unknown key, a valid connected-account header, and an unauthorized header.
- [x] 2.7 Document the protocol layer (versions, envelopes, prefixes, timestamp formats) in `docs/api-foundations.md`; verify the document matches the implemented behavior.

## 3. Accounts (spec: `accounts`)

- [x] 3.1 Persist accounts and configurations/capabilities/requirements; verify a migration-and-read test stores and returns a v2 account record.
- [x] 3.2 Implement `POST /v2/core/accounts` and `GET /v2/core/accounts/{id}` returning `v2.core.account` with `acct_` id and RFC 3339 `created`; verify tests assert the object, id prefix, and requested configurations.
- [x] 3.3 Implement `include` and default-null serialization for v2 reads; verify tests assert `null` when omitted and stored values when included.
- [x] 3.4 Implement `GET /v2/core/accounts` list with `limit`/`page`; verify pagination tests.
- [x] 3.5 Implement the v1 account projection (`/v1/accounts`, `/v1/accounts/{id}`) deriving `charges_enabled`, `payouts_enabled`, `details_submitted`, `capabilities`, `requirements`; verify a test retrieves a v2 account through v1 and asserts derived fields.
- [x] 3.6 Implement `POST /v1/account_links` and `POST /v2/core/account_links` producing mock onboarding URLs; verify tests assert the URL, use case, and timestamps per version.
- [x] 3.7 Implement onboarding state transitions (KYC pass/fail → `details_submitted`, capability activation, v1 derived flags, `account.updated` event); verify tests for both outcomes.
- [x] 3.8 Document the accounts surface and the v1/v2 projection in `docs/accounts.md`; verify the examples run against a live mock.

## 4. Catalog (spec: `catalog`)

- [x] 4.1 Implement `POST /v1/products` and `GET /v1/products/{id}`; verify tests assert `prod_` id, `active`, epoch `created`.
- [x] 4.2 Implement `POST /v1/prices` and `GET /v1/prices/{id}` for one-time and recurring prices, updating the product `default_price`; verify tests for both price types.
- [x] 4.3 Implement product and price lists with filters; verify a filtered price-list test.
- [x] 4.4 Enforce account scoping via the resolved account; verify a test that a connected-account product is not visible from the main account.
- [x] 4.5 Document the catalog endpoints in `docs/catalog.md`; verify examples run as written.

## 5. Billing (spec: `billing`)

- [x] 5.1 Implement `POST /v1/customers` and `GET /v1/customers/{id}`; verify tests assert `cus_` id and stored fields.
- [x] 5.2 Implement `POST /v1/subscriptions` from `items[][price]` with `customer` and `customer_account`; verify tests for both customer references.
- [x] 5.3 Implement the subscription status lifecycle and `customer.subscription.created|updated|deleted` events; verify tests assert status transitions and the created events.
- [x] 5.4 Implement `GET /v1/subscriptions/{id}` and list with `customer`/`customer_account` filters; verify a filtered list test.
- [x] 5.5 Document the billing endpoints in `docs/billing.md`; verify examples run as written.

## 6. Checkout (spec: `checkout`)

- [x] 6.1 Implement `POST /v1/checkout/sessions` for `payment`, `subscription`, and `setup` modes with line items and URLs; verify per-mode tests assert `cs_test_` id, `status=open`, and `url`.
- [x] 6.2 Implement `GET /v1/checkout/sessions/{id}` and `.../line_items`; verify retrieval and line-item envelope tests.
- [x] 6.3 Implement the completion flow (session → complete, payment intent and/or subscription, related events) transactionally; verify a test asserts `complete`, a succeeded payment intent, and the `checkout.session.completed` + `payment_intent.succeeded` events.
- [x] 6.4 Implement `POST /v1/checkout/sessions/{id}/expire` and the `checkout.session.expired` event; verify the expiry test.
- [x] 6.5 Document the checkout endpoints and completion semantics in `docs/checkout.md`; verify examples run as written.

## 7. Hosted simulation UI (spec: `hosted-simulation-ui`)

- [x] 7.1 Implement the shared server-rendered HTML shell loading Tailwind and daisyUI from a CDN with no framework bundle; verify a rendered page contains the CDN references and no framework script.
- [x] 7.2 Implement the shared simulation panel with applicable KYC, IBAN-select, and card controls; verify the panel renders on both pages with the right controls.
- [x] 7.3 Implement the onboarding page wired to account-link tokens with pass/fail KYC and `return_url`/`refresh_url` redirects; verify tests for completion and stale-link redirects.
- [x] 7.4 Implement the checkout page with session summary, card success/failure simulation, and `success_url`/`cancel_url` redirects including `{CHECKOUT_SESSION_ID}` substitution; verify the redirect and completion tests.
- [x] 7.5 Document the mock UI flows and how to drive them headlessly in `docs/hosted-ui.md`; verify the documented steps reproduce a completion.

## 8. Events and webhook dispatch (spec: `webhook-dispatch`)

- [x] 8.1 Implement synchronous event creation (v1 snapshot shape with `api_version` and `account` when connected) committed with the state change; verify a test lists the event immediately after an API call returns.
- [x] 8.2 Seed `event_destinations` from config with generated `whsec_` secrets; verify a boot test creates the configured destinations with secrets.
- [x] 8.3 Implement destination selection by `event_payload` and `events_from` scope and event allow-list (`*` and `prefix.*`); verify tests for payload/scope filtering and event filters.
- [x] 8.4 Implement the `DeliveryEngine` trait and the per-target FIFO engine with a fixed 200 ms delay; verify a timing test that the response returns before the webhook and that two events arrive ordered ~200 ms apart.
- [x] 8.5 Implement HMAC-SHA256 signing (`Stripe-Signature`) and the `X-Stripe-Mock-Delivery-Id` header; verify the signature validates with the target secret.
- [x] 8.6 Implement inline retries (3 attempts) and the delivery journal; verify a test with a failing receiver that ends in a terminal failed status with three recorded attempts.
- [x] 8.7 Document the webhook model, destination config, and the deferred A/B engine difference in `docs/webhooks.md`; verify the document matches observed behavior.

## 9. Internal control plane (spec: `internal-control-plane`)

- [x] 9.1 Implement `GET /internal/main` and `GET /internal/accounts/{id}/secrets` returning generated secret/publishable keys; verify tests assert correct prefixes and stability across restart.
- [x] 9.2 Implement `/internal/accounts` and per-destination signing-secret exposure for configured event destinations; verify tests assert `whsec_` secrets are returned.
- [x] 9.3 Implement `/internal/deliveries`, `wait`, `flush`, and `retry` over the journal; verify a test awaits a delivery to a terminal state and one that times out.
- [x] 9.4 Implement `POST /internal/reset` clearing objects/deliveries and reapplying seeding; verify a test shows only the reseeded main account after reset.
- [x] 9.5 Ensure the control plane requires no Stripe auth and binds only to the configured local address; verify an unauthenticated request is served and the bind address is honored.
- [x] 9.6 Document the internal endpoints in `docs/internal.md`; verify examples run as written.

## 10. End-to-end integration verification

- [x] 10.1 Run a full SDK-driven flow (v2 account + onboarding completion → v1 product/price → customer → subscription → checkout session → mock payment) against a live instance; verify state and emitted events match expectations.
- [x] 10.2 Verify webhook signature acceptance using the destination secret through the SDK verifier for both a `snapshot` and a thin destination; record the result in the README.
- [x] 10.3 Run the full test suite plus `openspec validate "build-stripe-mock"`; verify everything is green and the change validates.
