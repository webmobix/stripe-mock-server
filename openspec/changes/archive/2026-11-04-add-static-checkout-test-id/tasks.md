# Tasks

## 1. Seed the static demo session

- [x] 1.1 Add `pub const STATIC_CHECKOUT_ID: &str = "cs_test_static"` in `src/mock.rs` and a `seed_static_checkout_session` helper in `src/seed.rs` that upserts an `open`, `payment`-mode session for the main account with a single `Static demo item` line item (2000 `usd`), no `success_url`/`cancel_url`, and a far-future `expires_at`. Verify with a unit/integration test that after seeding the row exists with `status = "open"`, `amount_total = 2000`, one line item, and the fixed id.
- [x] 1.2 Call the helper from `seed::seed` so it runs at boot and via `POST /internal/reset`, and make the upsert idempotent (re-running leaves exactly one row in a fresh `open` state). Verify with a test that boots, completes or expires the static session, calls `/internal/reset`, and observes an `open` static session again.

## 2. Serve and exercise the static checkout page

- [x] 2.1 In `mock::checkout_page`, recognize `STATIC_CHECKOUT_ID` and reset a `complete` or `expired` row to a fresh `open` demo session before rendering. Verify with an integration test that `GET /mock/checkout/cs_test_static` returns 200 HTML containing the demo amount and the "Successful card"/"Declined card" controls without any prior session creation.
- [x] 2.2 Completion needs no new path: confirm `POST /mock/checkout/cs_test_static` with `card=success` uses `v1::complete_checkout_session`. Verify with an integration test asserting the response redirects, the session is `complete`, a succeeded `pi_` exists, and `checkout.session.completed` plus `payment_intent.succeeded` events are created; then `GET` the static URL again and assert a fresh open form renders.
- [x] 2.3 Document the static URL and its reuse semantics in `docs/hosted-ui.md` (open it directly; it re-opens after completion). Verify the documented example by starting the server and fetching the documented URL.

## 3. Integration verification

- [x] 3.1 Run `cargo test` (or `./scripts/...` if the repo wraps it) to confirm the full suite, including existing random-id checkout flows, still passes; run `cargo fmt --check` and `cargo clippy` clean.
