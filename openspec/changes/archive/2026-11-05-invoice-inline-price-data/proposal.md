# Proposal

## Why

The Stripe SDK's invoice line accepts an inline `price_data` next to a
`price` and an `amount`:

```
price_data: { currency, product | product_data, unit_amount | unit_amount_decimal }
```

The mock does not understand it. `parse_line_request` only looks for a `price`
and an `amount`, so a line carrying an inline price is rejected with *"Each
line must have either a `price` or an `amount`"`. An application that prices
its invoice lines inline — the way the Stripe SDK 22.6 `InvoiceAddLinesParams`
allows — cannot create an invoice at all against the mock.

## What Changes

- **Invoice lines accept an inline price.** A line may carry `price_data` with
  its own `currency` and amount, given as `unit_amount` or as
  `unit_amount_decimal`, and naming a product either by id
  (`price_data[product]`) or by description (`price_data[product_data][name]`).
  The amount is the inline amount times the line's `quantity`, and the line is
  described with the name of the product.
- **The product and the price are created for the line.** As on Stripe, an
  inline price that describes its product creates that product, and a price is
  created either way. Both are stored in the catalog, so the price the
  invoice line carries can be read back from `/v1/prices/{id}` and the product
  from `/v1/products/{id}`.
- **The inline price is checked against the invoice.** Its `currency` has to
  be the invoice's currency, a `price_data[product]` that does not exist is
  rejected, and a missing or malformed `currency` or amount is rejected with
  an `invalid_request_error` naming the parameter.

An inline price is accepted at invoice creation and on `add_lines`, and on
`remove_lines` and `update_lines` a line that carries one is treated like any
other line.

No other invoice behavior changes: catalog-priced and ad-hoc lines keep
working exactly as before, and the totals, the tax, and the lifecycle are
untouched.

## Capabilities

### New Capabilities

None. This change extends the `invoicing` capability.

### Modified Capabilities

- `invoicing`: an invoice line may carry an inline `price_data` instead of a
  catalog price, and the product and price it needs are created for the line.

## Impact

- **Code.** `src/api/invoicing.rs` gains the `price_data` line form, the
  decimal amount parser, and the creation of the inline product and price;
  `src/api/v1.rs` gains a product loader.
- **Database.** No schema change: the inline product and price use the
  existing `products` and `prices` tables.
- **API surface.** No new route; `POST /v1/invoices` and
  `POST /v1/invoices/{id}/add_lines` accept a new line form.
- **Events.** No new event type.
- **Tests.** `tests/invoicing.rs` covers the inline price on `add_lines` and
  at creation, and the rejections.
- **Docs.** `docs/invoicing.md` gains an "Inline prices" section.
