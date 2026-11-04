# Proposal

## Why

The mock hosted pages currently frame themselves as a developer tool: the shared
shell shows a **TEST MODE** badge and a **"Local simulation tool. Never use in
production."** footer, and the shared simulation panel carries a **"Simulation
panel"** heading plus a **"developer-mode simulator"** disclaimer. These labels
add noise, and automation cannot reliably target the payment controls because
they have no stable ids. The checkout button labels also over-specify "card"
even though the page simulates payment generally.

## What Changes

- Rename the checkout control label **"Simulate card payment"** to
  **"Simulate payment"**.
- Remove the word "card" from both checkout action buttons: **"Successful card"**
  → **"Successful"** and **"Declined card"** → **"Declined"**. The form field
  (`name="card"`, values `success`/`fail`) is unchanged.
- Add stable `id` attributes for automation:
  - `id="payment-success"` on the successful-payment button.
  - `id="payment-decline"` on the declined-payment button.
  - `id="payment-cancel"` on the cancel control.
- Turn the checkout cancel link into a button-styled control placed next to the
  simulate-payment buttons (still navigating to the cancel URL, not submitting
  the completion form).
- Remove the **TEST MODE** badge from the shared shell on all pages.
- Remove the **"Local simulation tool. Never use in production."** footer text
  from the shared shell on all pages.
- Remove the **"Simulation panel"** heading and the **"Controls behave like a
  developer-mode simulator; no real data is processed."** description from the
  shared simulation panel on all pages (onboarding and checkout).

Explicitly out of scope: changing payment/simulation behavior, form field names
or values, onboarding control semantics, the cancel URL/route, styling beyond
placing the cancel control, and any new API or dependency.

## Capabilities

### New Capabilities
<!-- None. -->

### Modified Capabilities
- `hosted-simulation-ui`: the shared shell no longer shows developer-tooling
  chrome, the simulation panel no longer shows a heading/disclaimer, and the
  checkout form exposes stable ids plus a renamed, button-styled cancel control.

## Impact

- `src/mock.rs` — `shell()` (remove badge and footer), `simulation_panel()`
  (remove heading/description, rename checkout label and button text, add button
  ids and the cancel control).
- `tests/flows.rs` — assertions that reference `"Successful card"` /
  `"Declined card"` must be updated to the new labels; add coverage for the new
  ids and the absence of the removed text.
- `docs/hosted-ui.md` — update the description of the checkout controls.
- No API, route, config, dependency, or database changes; no breaking changes.
