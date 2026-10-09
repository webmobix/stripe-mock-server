# Design

## Context

The repository is a new Rust crate (`edition 2024`, empty dependency set, hello-world `main.rs`) with an initialized OpenSpec project but no prior specs. `docs/openapi.spec3.json` is the full public Stripe spec (454 paths, 1701 schemas, version `2026-09-30.endive`) and serves as the shape contract for the subset we implement — it is not a code-generation input.

The target consumers are automated tests that use the **official Stripe SDK** against a base-URL override, plus a headless browser that drives mock hosted pages. See `proposal.md` for motivation and the `specs/` delta files for the behavioral requirements.

Two properties of the real system drive the architecture: Stripe exposes **two API conventions** (v1 form-encoded/epoch and v2 JSON/RFC 3339) from one host, and **state changes are synchronous while webhook notification is asynchronous**.

## Goals / Non-Goals

**Goals:**

- A single local process that the official SDK can target unchanged, for the accounts → catalog → subscriptions → checkout → webhook flow.
- Realistic identifiers, keys, timestamps, error envelopes, and webhook signatures so SDK parsing and verification behave exactly as against Stripe.
- Deterministic, reproducible test behavior despite asynchronous webhook delivery.
- A clear seam so deferred behavior (v2 thin events, dynamic destinations, connected-account keys, time advance) can be added without schema or interface churn.

**Non-Goals:**

- Implementing the whole Stripe API. Only the roughly 25 endpoints in the tiers below.
- Emulating Stripe internals (risk, tax, disputes, treasury, issuing) or matching undocumented behavior.
- Running real card networks or external services.
- Generating code from the OpenAPI spec.

## Decisions

### D1. Stack: Rust, tokio + axum, sqlx/SQLite, TOML

Rust with `tokio`/`axum` for HTTP, `sqlx` (SQLite, async) for persistence, `serde`/`serde_json`, `toml` for config, and `hmac`+`sha2` for signing. Config is TOML because it is idiomatic for Rust tooling and supports typed deserialization.

Alternatives: actix-web (comparable; axum chosen for tower ecosystem and simplicity); rusqlite (simpler but sync, awkward inside async handlers); a flat-file store (rejected — the domain needs relational joins and list/pagination semantics).

SQLite runs with WAL, a small connection pool, and a single-writer discipline where a request's state mutation, Event row, and Delivery rows commit in one transaction (see D7).

### D2. Dual-version compatibility layer

One router serves both `/v1/**` and `/v2/**`. A shared layer resolves authentication and the target account, while version-specific serializers produce epoch vs RFC 3339 timestamps and the v1 vs v2 error envelopes. v1 bodies are form-encoded (including Stripe's bracketed nested notation such as `line_items[0][price]`), which requires a small custom parser that expands bracket keys into nested structures; v2 bodies are JSON.

Alternatives: separate servers per version (rejected — must share one base URL and one datastore); a request-body shim that treats everything as JSON (rejected — the SDK sends form encoding to v1).

### D3. Accounts v2 canonical, v1 projection

There is a single `accounts` table. A v2 account record (configurations `customer`/`merchant`/`recipient`, capabilities, requirements, identity, defaults) is canonical, and `/v1/accounts` requests compute a v1 `account` projection from it (`charges_enabled`, `payouts_enabled`, `details_submitted`, `capabilities`, `requirements`). Account ids use the shared `acct_` prefix, so a v2-created account is usable in v1 calls and vice versa.

v2 reads honor `include` and default-null semantics: stored values are omitted (`null`) unless named in `include`. This matches real behavior and is required for honest tests, even though it is less convenient than always returning everything.

Alternatives: a separate v2 model (rejected — the docs and spec describe v1 and v2 as two views of one account); always returning full data (rejected — would let tests pass that fail against Stripe).

### D4. Webhooks are configuration, not API

The mock has no webhook registration endpoints. Receivers are configured in `config.toml` as Stripe-style **event destinations** (`[[event_destinations]]`), mirroring how teams configure webhooks once in the Stripe dashboard. A destination declares a `webhook_endpoint.url`, an `event_payload` (`snapshot` | `thin`), an `events_from` scope (`@self` | `@accounts`), `enabled`, and an `enabled_events` allow-list or `*` with `prefix.*` patterns. The server issues one `whsec_` signing secret per endpoint URL at boot (destinations sharing a URL share it), retrievable through the internal control plane.

The proposal's original tiers mentioned `/v1/webhook_endpoints` and `/v2/core/event_destinations`; both are dropped from the MVP because the app under test never registers them. (v2 thin event destinations were later added by `add-v2-thin-events`, and the configuration was unified onto event destinations by `unify-event-destinations`; the `[[webhook_targets]]` / `scope` model originally described here was removed with no compatibility shim.)

Alternatives: full dynamic registration API (rejected — unused surface, more state); a single hard-coded target (rejected — the user needs platform vs connected routing and per-target filtering).

### D5. Event representation: v1 snapshot now, v2 thin deferred

The MVP emits **v1 snapshot `Event` objects** (`object: "event"`, `data.object` inline, epoch `created`, `api_version`, `account` when the source is a connected account) for every event, including `account.updated` on onboarding completion. This gives one signing path and works with the SDK's `Webhook.constructEvent`.

The events row carries `api_version` and a `version` discriminator so v2 thin events can be added later without migration. See **Deferred / future** below.

### D6. Synchronous state and events, asynchronous delivery

A request that changes state (session completion, subscription activation, payment intent success) commits the state mutation, the Event object, and a `pending` Delivery row **in one transaction before responding**. Only the outbound HTTP call is deferred. This is deliberately the real Stripe model and means a test can assert state and list events immediately after the call returns.

### D7. Delivery engine: per-destination FIFO, fixed 200 ms, three attempts

Each target owns a FIFO queue. Deliveries are sent in emit order, spaced a fixed 200 ms apart, each with `Stripe-Signature` (`t=<ts>,v1=HMAC_SHA256(secret, "<ts>.<payload>")`) and a unique `X-Stripe-Mock-Delivery-Id` header for deduplication. A failed attempt is retried inline up to three attempts, then marked `failed_exhausted`; the queue then advances. Every attempt is written to the delivery journal.

The engine is behind a `DeliveryEngine` trait (`enqueue(target_id, delivery_id)`) so the strategy can be swapped; both strategies share the journal, signatures, and internal endpoints.

**Alternatives considered — the A/B difference:**

```
 (A) concurrent per-event              (B) per-target FIFO   <-- chosen
 ----------------------------------    --------------------------------
 spawn a task per event, sleep 200ms   enqueue onto the target's queue
 arrivals ~simultaneous                arrivals ordered, +200ms each
 order arbitrary                       order == emit order (per target)
 no cross-event blocking               a slow receiver blocks that target only
 retries independent                   retries inline, then next item
 closer to real Stripe                 mock convenience
 higher throughput                     predictable for tests
```

(A) matches real Stripe more closely — Stripe does not guarantee event ordering and delivers concurrently — and isolates a slow receiver. (B) is chosen because deterministic per-target ordering and spacing materially simplify tests while delivery remains genuinely asynchronous; it is intentionally *more* deterministic than Stripe. Revisit (A) if unordered delivery must be exercised or throughput per target matters.

### D8. Deterministic awaiting of async deliveries

Because delivery is async, tests must not sleep-and-hope. The internal control plane exposes `/internal/deliveries` (journal), `/internal/deliveries/wait` (block until a given event's delivery reaches a terminal state or times out), `/internal/deliveries/flush` (block until all pending deliveries settle), and manual retry. Together with the synchronously created journal row, this removes the race without changing the async behavior under test.

### D9. Internal control plane for non-returnable secrets

Real Stripe deliberately does not return some secrets (v2 event destinations' signing secrets are returned only at creation). The mock instead exposes all generated material at `/internal/**`: the main account, per-account secret/publishable keys, per-endpoint-URL signing secrets, the delivery journal, wait/flush/retry, and reset. These endpoints require no Stripe auth and bind only to the configured local address. Connected accounts do not get directly usable API keys in the MVP.

### D10. Hosted UI: embedded, server-rendered, CDN styling

The onboarding and checkout pages are server-rendered HTML emitted by Rust handlers, with no client-side framework, loading Tailwind and daisyUI from a CDN. A shared simulation panel component offers the applicable controls (KYC, IBAN select, card payment). Pages are addressed by opaque tokens/session ids and perform correct `refresh_url`/`return_url` (onboarding) and `success_url`/`cancel_url` (checkout) redirects, substituting `{CHECKOUT_SESSION_ID}`.

### D11. Modular layout

```
src/
  main.rs          boot: config -> pool -> migrate -> seed -> router
  config.rs        TOML model
  db/              sqlx pool + migrations
  ids.rs           prefixed id/key generation
  time.rs          epoch <-> RFC 3339-ms helpers
  auth.rs          key -> account, Stripe-Account, internal bypass
  api/v1/          form-encoded handlers (accounts, products, prices,
                   customers, subscriptions, checkout, payment_intents)
  api/v2/          JSON handlers (accounts, account_links, persons)
  mock/            hosted HTML (onboarding, checkout, shared components)
  internal/        /internal/** control plane
  events.rs        event creation (sync)
  dispatch.rs      DeliveryEngine, signing, retries, journal
```

### D12. Endpoint surface (MVP)

| Tier | Endpoints |
|------|-----------|
| Foundations | auth + account resolution, v1/v2 error and list envelopes, ids, time |
| Accounts | `POST/GET /v2/core/accounts`, `/v2/core/accounts/{id}`, `POST /v2/core/account_links`, `POST/GET /v1/accounts`, `/v1/accounts/{id}`, `POST /v1/account_links` |
| Catalog | `POST/GET /v1/products`, `/v1/products/{id}`, `POST/GET /v1/prices`, `/v1/prices/{id}` |
| Billing | `POST/GET /v1/customers`, `/v1/customers/{id}`, `POST/GET /v1/subscriptions`, `/v1/subscriptions/{id}` |
| Checkout | `POST/GET /v1/checkout/sessions`, `/v1/checkout/sessions/{id}`, `.../line_items`, `.../expire` |
| Events | sync creation + `GET /v1/events/{id}` (read) |
| Delivery | config targets, dispatcher, journal, internal wait/flush/retry |
| Mock UI | `GET /mock/connect/{token}`, `GET /mock/checkout/{session_id}`, simulate actions |
| Internal | `/internal/main`, `/internal/accounts`, `/internal/accounts/{id}/secrets`, `/internal/deliveries*`, `/internal/reset` |

### D13. Event destination configuration shape

```toml
[[event_destinations]]
name             = "snapshot-app"
event_payload    = "snapshot"             # snapshot (v1) | thin (v2)
events_from      = ["@self", "@accounts"] # @self | @accounts
enabled_events   = ["*"]                  # or exact names / "prefix.*"
enabled          = true
webhook_endpoint = { url = "http://localhost:4000/stripe/webhook" }
```

## Risks / Trade-offs

- **SDK base-URL override may vary by language/version** → Validate early with a spike against the intended SDK(s) before building breadth; the `host`/`port`/`protocol` (or `base_url`) config plus a v2-capable SDK is a prerequisite.
- **`Stripe-Version` mismatch for v2 calls (SDK defaults may pin a preview version)** → Accept any version, ignore unknown params, and echo the spec `api_version` on events rather than validating it.
- **v2 default-null / `include` fidelity is easy to get subtly wrong** → Encode it once in the v2 serializer with tests asserting null-vs-included; read the spec schema for each include group.
- **Stripe bracketed form parsing** → Dedicated parser with unit tests for `items[0][price]`, `line_items[data][0][price]`, and nested arrays.
- **Async tests are flaky if they sleep** → Mandate the wait/flush endpoints in the test harness; keep the journal row transactional with the event.
- **At-least-once delivery causes duplicates on retry** → `X-Stripe-Mock-Delivery-Id` plus idempotent receivers in tests.
- **FIFO head-of-line blocking by a slow/hung receiver** → Bound each attempt with a delivery timeout; contained per target; consider engine (A) later.
- **Internal endpoints expose secrets** → Bind to configured local address only; document that the server is a test tool, never production.
- **SQLite write contention under parallel tests** → WAL, single-writer transaction for state+event+delivery, and reset between suites.
- **Spec drift (`2026-09-30.endive` is forward-dated)** → Treat the spec as a shape reference, not a version to match exactly; pin behavior to the implemented subset.

## Deferred / future

Recorded so later increments can pick them up without re-deciding:

- **Event payload B/C:** v2 thin events and dynamic `event_destinations`; the `events` row already carries `api_version` + `version`.
- **Delivery engine (A):** concurrent per-event, unordered, higher throughput; swap behind `DeliveryEngine`.
- **Connected-account keys:** directly usable secret/publishable keys; MVP exposes none.
- **Server-side time advance:** subscription renewal/period rollover without wall-clock waits.
- **Full v2 onboarding:** `account_tokens` + `persons` instead of the simplified hosted form.
- **Invoices/tax, idempotency keys, broader endpoint coverage, `expand[]` beyond line items.**

## Migration Plan

Greenfield: no data or API to migrate. Delivery is phased by tier (Foundations → Accounts → Catalog/Billing → Checkout → Delivery → Mock UI → Internal), with `internal-control-plane` and reset landing early to support testing from the first tier. Rollback is deleting the change directory and generated database; no external state is touched. `docs/openapi.spec3.json` is read-only.

## Open Questions

- Exact retry backoff beyond the fixed 200 ms cadence (fixed-inline chosen; exponential backoff is a config swap).
- Whether to expose read-only `GET /v1/events` list and `/v2/core/events` in the MVP or only `GET /v1/events/{id}`.
- Which SDK language(s) are first-class for the drop-in guarantee.
