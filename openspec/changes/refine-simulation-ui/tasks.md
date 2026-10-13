# Tasks

## 1. Remove developer-tooling chrome and panel framing

- [x] 1.1 In `src/mock.rs` `shell()`, remove the `TEST MODE` `<span class="badge badge-warning">` badge and the `<footer>` containing "Local simulation tool. Never use in production.". Verify by adding assertions in `tests/flows.rs` that a rendered page (e.g. `GET /mock/checkout/cs_test_static`) contains neither "TEST MODE" nor "Local simulation tool. Never use in production.".
- [x] 1.2 In `src/mock.rs` `simulation_panel()`, remove the `Simulation panel` `<h2>` and the "Controls behave like a developer-mode simulator; no real data is processed." `<p>` for every `PanelKind`, leaving the controls. Verify with integration assertions that both a checkout page and an onboarding page render the applicable controls while containing neither removed string.

## 2. Checkout control labels, ids, and cancel button

- [x] 2.1 In `src/mock.rs` `simulation_panel()` (checkout branch), rename the label "Simulate card payment" to "Simulate payment", change the button labels to "Successful" and "Declined", and add `id="payment-success"` / `id="payment-decline"` while keeping `name="card"` with values `success` / `fail`. Verify with an integration test asserting the new labels and ids are present and "Successful card" / "Declined card" are absent.
- [x] 2.2 Replace the "Cancel and return" anchor with `<a id="payment-cancel" class="btn" href="/mock/checkout/{id}/cancel">Cancel</a>` placed in the same button group as the payment buttons (do not submit the completion form). Verify with an integration test that `id="payment-cancel"` is present and `GET /mock/checkout/{id}/cancel` still redirects to `cancel_url` (or renders the in-page cancel notice).
- [x] 2.3 Update the existing `tests/flows.rs` assertions that match "Successful card" / "Declined card" to the new labels, and add assertions for `payment-success`, `payment-decline`, and `payment-cancel`. Verify the full `cargo test` suite passes.
- [x] 2.4 Update `docs/hosted-ui.md` to describe the "Simulate payment" controls, the stable ids, and the button-styled cancel control, then verify the documented `curl` flow still works as written against a running server.

## 3. Integration verification

- [x] 3.1 Run `cargo fmt --check`, `cargo clippy`, and the full `cargo test` suite to confirm the checkout and onboarding flows pass end to end with the new markup.
