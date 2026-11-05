# Tasks

## 1. Create an invoice without lines

- [x] 1.1 In `src/api/invoicing.rs`, remove the `missing_param` rejection of an empty `lines` array from `create_invoice`, so that a request with only a `customer` creates a draft invoice with an empty `lines.data` and a `subtotal`, `tax_total`, and `total` of 0; verify with an integration test in `tests/invoicing.rs` that the invoice is created and empty.
- [x] 1.2 Keep parsing a `lines` array when the request carries one, so the older one-shot creation flow still prices its lines; verify with the existing invoice creation tests still passing under `cargo test`.

## 2. Resolve the invoice currency

- [x] 2.1 In `src/api/invoicing.rs`, add a helper that resolves an invoice's currency from the request's `currency`, then the customer's currency, then `usd`, and rejects a `currency` that is not a three-letter ISO code; verify with unit tests in the module's `tests` block.
- [x] 2.2 In `create_invoice`, use that helper instead of deriving the currency from the first catalog-priced line, and reject a line whose price is in another currency than the invoice; verify with an integration test that an invoice created with `currency` of `eur` and a `eur` price is priced in `eur`, and that a mismatching price is rejected.
- [x] 2.3 In `src/api/v1.rs`, accept a `currency` on `POST /v1/customers` and `POST /v1/customers/{id}`, store it, and expose it on the customer; verify with an integration test that a customer created with `currency` of `eur` reports it and that an invoice raised against it without a `currency` is priced in `eur`.
- [x] 2.4 In `src/db.rs`, add the nullable `currency` column to the `customers` table and to the idempotent column migration, so an existing database gains it too; verify with the schema migration tests still passing under `cargo test`.

## 3. Delete a draft invoice

- [x] 3.1 In `src/api/invoicing.rs`, add a handler for `DELETE /v1/invoices/{id}` that loads the account's invoice, rejects it unless its status is `draft`, removes it, and returns the tombstone Stripe returns (`id`, `object` of `invoice`, `deleted` of true); verify with integration tests that the invoice is gone from subsequent reads and lists.
- [x] 3.2 In the same handler, emit an `invoice.deleted` event with the deleted invoice as its data object, and reject the deletion of an invoice that is not a draft with an `invalid_request_error`; verify with an integration test that the event is delivered and that a finalized invoice cannot be deleted.
- [x] 3.3 In `src/lib.rs`, register `DELETE /v1/invoices/{id}`; verify with an integration test that the route is reachable.

## 4. Finalize an invoice that has lines added later

- [x] 4.1 In `src/api/invoicing.rs`, reject the finalization of an invoice with no lines, since the lines may only have been added after the invoice was created; verify with an integration test that the invoice stays a draft.
- [x] 4.2 Verify with an integration test that an invoice created empty, given its lines through `add_lines`, can be finalized and paid; verify with the existing lifecycle tests still passing under `cargo test`.

## 5. Documentation

- [x] 5.1 Document the creation of an invoice without lines and the currency precedence in `docs/invoicing.md`; verify by following the documented commands against a running mock.
- [x] 5.2 Add a `DELETE /v1/invoices/{id}` section to `docs/invoicing.md`, note that only a draft invoice can be deleted, and add `invoice.deleted` to the event list; verify that the list matches the event types the handlers emit.

## Workflow follow-up

- Archive this change once its tasks are complete and the `invoicing` and
  `billing` specs carry the new requirements.
