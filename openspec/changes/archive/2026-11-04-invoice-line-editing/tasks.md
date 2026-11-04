# Tasks

## 1. Shared line and totals helpers

- [x] 1.1 In `src/api/invoicing.rs`, add one helper that parses a line item as either catalog-priced (`price`, optional `quantity`, optional inline `tax_rate`) or ad-hoc (`amount`, optional `description`), and renders the same field set for both, with `price` set to null on an ad-hoc line; verify with unit tests in the module's `tests` block asserting the rendered line's `amount`, `description`, `price`, and `currency` for both forms.
- [x] 1.2 In `src/api/invoicing.rs`, add a helper that derives an invoice's `subtotal`, `tax_total`, and `total` from a list of line objects by summing the line amounts and their `tax_amounts` amounts; verify with unit tests over an empty list and a mixed list of catalog and ad-hoc lines.
- [x] 1.3 In `src/api/invoicing.rs`, add a helper that recomputes a line's `tax_amounts` from the line amount and the percentage held in its `tax_rates`; verify with a unit test that a 10 percent rate on an amount of 1000 yields a tax amount of 100.
- [x] 1.4 Rewire `create_invoice` to parse its lines through the shared line helper and to compute the invoice totals through the shared totals helper, keeping its current rule that at least one line is required; verify with the existing invoicing unit and integration tests still passing under `cargo test`.

## 2. Add lines to a draft invoice

- [x] 2.1 In `src/api/invoicing.rs`, add a handler for `POST /v1/invoices/{id}/add_lines` that loads the account's invoice, rejects it unless its status is `draft`, requires a `lines` array, and builds one line per item through the shared line helper; verify with unit tests that a non-draft invoice and a missing `lines` array each produce an `invalid_request_error`.
- [x] 2.2 In the same handler, reject a line whose catalog price is unknown, whose price currency differs from the invoice currency, or whose `amount` is not a non-negative integer; verify with integration tests in `tests/invoicing.rs` that each rejection leaves the invoice unchanged.
- [x] 2.3 In the same handler, append the new lines, write the line list and the recomputed `subtotal`, `tax_total`, `total`, `amount_due`, and `amount_remaining` with a compare-and-swap on the previous line list and status `draft`, and move the invoice's `updated` timestamp; verify with an integration test that adds a catalog line and an ad-hoc line to a draft invoice and asserts the returned invoice's lines and totals.
- [x] 2.4 In `src/lib.rs`, register `POST /v1/invoices/{id}/add_lines`; verify with an integration test that the route is reachable and that the handler's response is the updated invoice.

## 3. Remove lines from a draft invoice

- [x] 3.1 In `src/api/invoicing.rs`, add a handler for `POST /v1/invoices/{id}/remove_lines` that loads the account's invoice, rejects it unless its status is `draft`, requires a `lines` array whose items name a line `id`, and accepts an optional `behavior` of `delete` or `unassign`; verify with a unit test that any other `behavior` is rejected with an `invalid_request_error`.
- [x] 3.2 In the same handler, detach the named lines from the invoice, reject a line id the invoice does not carry with an `invalid_request_error`, and write the remaining line list with the recomputed totals; verify with integration tests in `tests/invoicing.rs` that removing one line and removing every line leave the invoice totals consistent with its lines.
- [x] 3.3 In `src/lib.rs`, register `POST /v1/invoices/{id}/remove_lines`; verify with an integration test that the route is reachable and that removing lines from a non-draft invoice is rejected.

## 4. Update lines on a draft invoice

- [x] 4.1 In `src/api/invoicing.rs`, add a handler for `POST /v1/invoices/{id}/update_lines` that loads the account's invoice, rejects it unless its status is `draft`, and requires a `lines` array whose items name a line `id`; verify with a unit test that an item with no updatable field is rejected with an `invalid_request_error`.
- [x] 4.2 In the same handler, apply `quantity` (recomputing the amount from the line's price), `amount`, `description`, `price`, and `tax_rate` to the named lines, and recompute the line tax and the invoice totals; verify with integration tests in `tests/invoicing.rs` that updating a quantity changes the line amount and the invoice total, and that updating a tax rate changes the invoice `tax_total`.
- [x] 4.3 In the same handler, reject a line id the invoice does not carry with an `invalid_request_error` and leave the invoice unchanged; verify with an integration test that the invoice still holds its original lines and totals after the rejection.
- [x] 4.4 In `src/lib.rs`, register `POST /v1/invoices/{id}/update_lines`; verify with an integration test that the route is reachable and that updating lines on a non-draft invoice is rejected.

## 5. Invoice line change event

- [x] 5.1 In `src/api/invoicing.rs`, emit an `invoice.updated` event with the updated invoice as its data object in the same transaction as each successful line change, and dispatch it after the commit; verify with integration tests in `tests/invoicing.rs` that an add, a remove, and an update each leave one `invoice.updated` event describing the changed invoice.
- [x] 5.2 Verify with an integration test that a rejected line change emits no `invoice.updated` event; verify with the same test that the invoice's `updated` timestamp is unchanged after the rejection.

## 6. Documentation

- [x] 6.1 Document the ad-hoc line form (`lines[][amount]` with an optional `lines[][description]`) and note that it is accepted both at invoice creation and on `add_lines` in `docs/invoicing.md`; verify by following the documented commands against a running mock.
- [x] 6.2 Add an `add_lines` section to `docs/invoicing.md` with a curl example for a catalog-priced line and for an ad-hoc line, and the error cases; verify by following the documented commands against a running mock.
- [x] 6.3 Add `remove_lines` and `update_lines` sections to `docs/invoicing.md` with a curl example each and the note that `behavior` accepts `delete` or `unassign`; verify by following the documented commands against a running mock.
- [x] 6.4 Add `invoice.updated` to the event list in `docs/invoicing.md` and note that the three endpoints only work on draft invoices; verify that the list matches the event types the handlers emit.

## Workflow follow-up

- Archive the `add-customer-invoicing` change first: it is implemented but not archived, and this change's delta modifies requirements of the `invoicing` capability it introduces.
- Archive this change once its tasks are complete and `openspec validate invoice-line-editing` passes.
