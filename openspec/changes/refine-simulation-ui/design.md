# Design

## Context

See `proposal.md` — Why. The mock hosted pages are rendered entirely in
`src/mock.rs`: `shell()` wraps every page (including the header badge and
footer), and `simulation_panel(PanelKind)` builds the shared control block used
by both the onboarding and checkout pages. The checkout page embeds that panel
inside a `<form method="post" action="/mock/checkout/{id}">`; `checkout_submit`
reads the `card` form field, and `checkout_cancel` is a separate `GET
/mock/checkout/{id}/cancel` route reached by an anchor below the form. Pages ship
no client-side JavaScript. The delta in
`specs/hosted-simulation-ui/spec.md` defines the required behavior.

## Goals / Non-Goals

**Goals:**

- Remove developer-tooling chrome and simulation framing from shared output
  (all pages) with a change confined to `src/mock.rs`.
- Give automation stable `id` hooks on the checkout payment and cancel controls.
- Make the cancel control visually a button next to the simulate-payment buttons
  while preserving the existing cancel navigation and completion behavior.

**Non-Goals:**

- No change to form field names/values, routes, handlers, or payment semantics.
- No per-page variants of the panel, and no new configuration or dependencies.
- No restyling beyond placing the cancel control.

## Decisions

### Remove the badge, footer, heading, and disclaimer at their single shared sources

The badge and footer exist only in `shell()`; the heading and disclaimer exist
only in `simulation_panel()`. Editing those two functions removes all four from
every page at once, which matches the "all pages" decision without threading a
per-page flag through both call sites.

- Alternative rejected: a parameter or separate checkout/onboarding panel
  variants. It would add branching for output that should be identical
  everywhere, and the user explicitly confirmed removal on all pages.

### Keep the `name="card"` field; only change the visible button labels

`checkout_submit` dispatches on `params.get("card")` with values `success`/`fail`.
"Remove card from both buttons" is a label change ("Successful card" →
"Successful", "Declined card" → "Declined"); the field name and values are
untouched so the existing completion and decline paths keep working.

- Alternative rejected: renaming the field to match the new wording. It would
  change the request contract for no benefit and force handler/test churn.

### Add ids as plain attributes on the existing controls

`id="payment-success"` and `id="payment-decline"` are added to the existing
submit buttons, and the label becomes "Simulate payment". Ids are stable and
independent of the visible text, which is the point of the automation hooks.

### Cancel control stays a navigation anchor styled as a button

The cancel control must remain a navigation to `GET /mock/checkout/{id}/cancel`.
A native `<button>` inside the completion `<form>` would submit it (or need
`formaction`/JavaScript) and could accidentally complete the session. Rendering
an anchor as `<a id="payment-cancel" class="btn" href=".../cancel">Cancel</a>`
inside the same daisyUI `.join` as the payment buttons gives the button
appearance and the "next to" placement without changing navigation or adding
JavaScript.

- Alternative rejected: `<button type="button" onclick="location=...">` adds
  client-side JS to a deliberately JS-free page.
- Alternative rejected: `<button formaction="/mock/checkout/{id}/cancel"
  formmethod="get">` works without JS but is easy to conflate with the payment
  submits; an anchor keeps intent unambiguous.

## Risks / Trade-offs

- [Existing tests assert the old labels `"Successful card"` /
  `"Declined card"` in `tests/flows.rs`] → Update those assertions to the new
  labels and add assertions for the new ids; the suite guards the change.
- [Removing the panel heading on onboarding leaves the KYC/IBAN controls without
  a section title] → Accepted and explicitly confirmed; controls remain labeled
  and the page still renders its own card title.
- [A screen reader user loses the `<a>` affordance if the styled anchor is
  mistaken for a submit] → Keep the accessible link semantics (an anchor) and a
  clear "Cancel" label; no role override.
