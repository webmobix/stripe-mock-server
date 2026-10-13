# Design

## Context

See `proposal.md` — Why. The mock already persists every checkout session as a
`checkout_sessions` row and renders it through `mock::checkout_page`, and
completion runs entirely through `v1::complete_checkout_session`, which loads the
row, resolves the owning account, and creates the payment intent, subscription,
and events transactionally. Boot-time state is created by `seed::seed`, which is
also called by `POST /internal/reset`. Programmatic sessions get random
`cs_test_...` ids from `ids::checkout_session_id()`.

## Goals / Non-Goals

**Goals:**

- A checkout page reachable at a fixed, documented URL with no prior API calls.
- Completion of the static page behaves exactly like a normal session (real
  payment intent and events), so it exercises the full path.
- The URL stays usable across repeated manual checks.

**Non-Goals:**

- Configuring the id, amount, currency, or line-item content.
- A static onboarding link (only checkout is requested).
- Changing random-id session behavior in any way.

## Decisions

### Persist a real seeded row, not an in-memory page

Seed the static session into `checkout_sessions` as a normal row under the main
account instead of synthesizing a page-only session. `complete_checkout_session`
requires a persisted row and uses `account_id` to scope the payment intent and
events; reusing it means zero new completion logic and real webhook deliveries.

- Alternative rejected: render an in-memory session and special-case completion.
  That would need a parallel completion path that either skips state creation
  (unrealistic) or fabricates an account, duplicating logic for no benefit.

### Fixed id and fixed demo content

Use a `pub const STATIC_CHECKOUT_ID: &str = "cs_test_static"` and seed one
payment-mode session with a single line item (`Static demo item`, 2000 `usd`),
no `success_url`/`cancel_url`, and a far-future `expires_at`. Payment mode needs
no `prices` row at completion, so the embedded price object is display-only.

- Alternative rejected: a `[mock]` config section for the id/amount. Kept out of
  scope to keep the change small; can be added later without changing behavior.
- Collision with generated ids is impossible in practice: the random generator
  never emits the literal `static` segment.

### Re-open the static session on request when it is no longer open

`mock::checkout_page` recognizes `STATIC_CHECKOUT_ID`; if the row is `complete`
or `expired`, it resets it to a fresh `open` demo session (same content, refreshed
`expires_at`) before rendering. Seeding at boot and on `/internal/reset` leaves
the row in the same fresh state, so every entry point agrees.

- Alternative rejected: seed once at boot only. After one completion the URL
  would show the success/error page until a restart or reset, defeating the
  "quick look" purpose.

## Risks / Trade-offs

- [GET mutates the static session back to `open`, so a caller that completes it
  and then re-opens the page sees a form again] → Documented in
  `docs/hosted-ui.md`; the static id is for manual/visual checks, while
  programmatic tests keep using random session ids with normal single-use
  semantics.
- [A completed static session's payment intent and events accumulate on each
  completion] → Expected for a test tool and consistent with repeated normal
  completions; `/internal/reset` clears them.
- [The embedded demo price id does not exist in `prices`] → Harmless because the
  session is `payment` mode; completion never looks up prices for payment mode.
