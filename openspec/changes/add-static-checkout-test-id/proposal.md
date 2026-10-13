# Proposal

## Why

Today, seeing the mock checkout page requires first creating a product, a price,
and a checkout session through the API, then copying a random `cs_test_...` URL
out of the response. For quick manual inspection of the hosted page — and for
smoke tests that only care that the page renders — that setup is unnecessary
friction. A well-known, always-available checkout URL lets anyone open
`http://127.0.0.1:12111/mock/checkout/cs_test_static` and immediately see the
page.

## What Changes

- Seed a single **static demo checkout session** under the main account at boot,
  using a fixed, documented id (`cs_test_static`).
- Serve the existing mock checkout page for that id without requiring a prior
  `POST /v1/checkout/sessions`; the page renders a fixed demo line item and
  total.
- Keep the static session reusable: opening its URL when it is `complete` or
  `expired` resets it to a fresh `open` demo session, so the page is always
  testable.
- Completing the static page follows the normal completion flow (succeeded
  payment intent, `checkout.session.completed` and `payment_intent.succeeded`
  events), scoped to the main account.
- Document the static URL in `docs/hosted-ui.md`.

Explicitly out of scope: making the id or demo amount/currency configurable, a
static onboarding link, and any change to programmatically created sessions
(random ids keep working exactly as before).

## Capabilities

### New Capabilities
<!-- None. -->

### Modified Capabilities
- `hosted-simulation-ui`: add a requirement that a well-known static demo
  checkout session exists so the checkout page can be opened and exercised
  without creating a session first.

## Impact

- `src/seed.rs` — seed the static demo session idempotently at boot.
- `src/mock.rs` — recognize the static id in the checkout page handler and
  re-open it when it is no longer `open`.
- `config.toml` / `tests/` — no new config; add integration coverage.
- `docs/hosted-ui.md` — document the URL and its reuse semantics.
- No new API endpoints, dependencies, or breaking changes; existing checkout
  sessions are unaffected.
