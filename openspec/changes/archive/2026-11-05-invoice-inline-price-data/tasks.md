# Tasks

## 1. Parse an inline price

- [x] 1.1 In `src/api/invoicing.rs`, add a `price_data` form to the line request, read alongside `price` and `amount`, and reject a line that carries none of the three with an `invalid_request_error`; verify with unit tests in the module's `tests` block.
- [x] 1.2 In the same parser, read the inline `currency` and validate it as a three-letter ISO code, and read the amount from `unit_amount` or `unit_amount_decimal`, rejecting a missing, negative, or malformed one with an `invalid_request_error` naming the parameter; verify with unit tests.
- [x] 1.3 In the same parser, read the product from `price_data[product]` or `price_data[product_data][name]`, and use the product's name as the line's description; verify with unit tests.
- [x] 1.4 Add a helper that parses a decimal amount such as `20.50` into minor units, rounded to the nearest minor unit; verify with unit tests over whole amounts, decimals, and malformed values.

## 2. Create the product and the price of an inline line

- [x] 2.1 In `src/api/invoicing.rs`, create the product described inline and the price of the line in the catalog, in the same transaction as the invoice or the line change, so the invoice line carries a price that can be read back; verify with an integration test that reads the price and the product back.
- [x] 2.2 In the same code, use a product named by id as it is, and reject an id that does not exist for the account; verify with an integration test that an unknown product is rejected and the invoice is unchanged.

## 3. Check the inline price against the invoice

- [x] 3.1 In `src/api/invoicing.rs`, reject an inline price whose currency differs from the invoice's currency, both at creation and on `add_lines`; verify with an integration test that the invoice is left unchanged.
- [x] 3.2 In the same code, price the line at the inline amount times its quantity, in the invoice's currency; verify with an integration test that a line of 2500 with a quantity of 2 amounts to 5000.

## 4. Documentation

- [x] 4.1 Add an "Inline prices" section to `docs/invoicing.md` with a curl example, the accepted forms, and the rejections; verify by following the documented commands against a running mock.
- [x] 4.2 Extend the SDK end-to-end script in `scripts/sdk-e2e/e2e.js` with an inline price line and the created price; verify with `scripts/sdk-e2e/run.sh`.
