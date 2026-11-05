# Invoicing

Invoices are priced from the account's catalog, carry optional inline tax
rates, and move through a small status machine:

```
draft ──finalize──▶ open ──pay / payment intent──▶ paid
  │                   │
  │                   ├──void──▶ void
  │                   └──mark_uncollectible──▶ uncollectible
  └──void──▶ void
```

`paid`, `void`, and `uncollectible` are terminal. Any other transition is
rejected with an `invalid_request_error` and leaves the invoice unchanged.

## Create an invoice

An invoice needs a `customer`. Its lines are optional: an invoice may be
created empty and given its lines afterwards with
[`add_lines`](#add-lines), which is how the Stripe SDK 22.6 creates one. A
line is either priced from the catalog (`lines[][price]`, with an optional
`lines[][quantity]`) or written directly (`lines[][amount]`, with an optional
`lines[][description]`):

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

CID=cus_...   # see billing.md
PRICE_ID=price_...

INVOICE=$(curl -s -X POST $BASE/v1/invoices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "lines[0][price]=$PRICE_ID" \
  --data-urlencode "lines[0][quantity]=2")
IID=$(echo "$INVOICE" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
```

A new invoice has an `in_` id, `status` of `draft`, and the `subtotal`, `total`,
and `lines` computed from the catalog prices. `quantity` defaults to 1.

An invoice created without any line is an empty draft: its `lines.data` is
empty and its `subtotal`, `tax_total`, and `total` are 0.

```bash
curl -s -X POST $BASE/v1/invoices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID"
```

### Currency

The invoice's currency is fixed when the invoice is created and never changes
afterwards. It is:

1. the `currency` of the request, when the request carries one;
2. otherwise the currency of the customer the invoice is raised against;
3. otherwise `usd`.

```bash
curl -s -X POST $BASE/v1/invoices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "currency=eur"
```

A `currency` that is not a three-letter ISO code is rejected. A line priced
from the catalog in another currency than the invoice is rejected too, both at
creation and on [`add_lines`](#add-lines).

### Ad-hoc lines

A line may also be written directly instead of being priced from the catalog:
`lines[][amount]` with an optional `lines[][description]`. An ad-hoc line is
priced in the invoice's currency and carries a null `price`:

```bash
curl -s -X POST $BASE/v1/invoices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "lines[0][amount]=799" \
  --data-urlencode "lines[0][description]=Consulting"
```

The ad-hoc line form is accepted both at invoice creation and on
[`add_lines`](#add-lines). A line that carries neither a `price` nor an
`amount` is rejected.

### Inline prices

A line may carry its own price in `price_data`, instead of naming a price from
the catalog. The inline price has its own `currency` and its own amount, given
as `unit_amount` or as `unit_amount_decimal`, and it names a product either
by id (`price_data[product]`) or by description
(`price_data[product_data][name]`):

```bash
curl -s -X POST $BASE/v1/invoices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "lines[0][price_data][currency]=usd" \
  --data-urlencode "lines[0][price_data][unit_amount]=2500" \
  --data-urlencode "lines[0][price_data][product_data][name]=Consulting" \
  --data-urlencode "lines[0][quantity]=2"
```

The line is priced at 2500 per unit, so it amounts to 5000, and it is
described with the name of the product. As on Stripe, the mock creates the
product and the price for the line: the line carries the price, and both
objects can be read back from `/v1/products/{id}` and `/v1/prices/{id}`.

The inline price is accepted both at invoice creation and on
[`add_lines`](#add-lines). Its currency has to be the invoice's currency, and
a `price_data[product]` that does not exist is rejected.

### Tax rates

A line may carry an inline tax rate. The percentage is required; the name is
optional:

```bash
curl -s -X POST $BASE/v1/invoices \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "lines[0][price]=$PRICE_ID" \
  --data-urlencode "lines[0][tax_rate][percentage]=10" \
  --data-urlencode "lines[0][tax_rate][name]=VAT"
```

The tax is computed once, at creation: the line gets a `tax_amounts` entry, the
invoice gets `tax_total`, and `total` is `subtotal` plus `tax_total`. Amounts
are in minor units and the per-line tax is rounded to the nearest minor unit
(halves away from zero). A percentage outside 0–100 is rejected.

The customer's tax ids (see [`billing.md`](billing.md)) are copied to the
invoice's `customer_tax_ids`, so an invoice carries the tax details on its own.

## Read invoices

```bash
curl -s "$BASE/v1/invoices/$IID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/invoices?customer=$CID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/invoices?status=open" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/invoices/$IID/lines" -H "Authorization: Bearer $KEY"
```

## Edit the lines of a draft invoice

The three line endpoints below only work on a `draft` invoice. A non-draft
invoice is rejected with an `invalid_request_error` and is left untouched.

After every successful change the invoice's `subtotal`, `tax_total`, `total`,
`amount_due`, and `amount_remaining` are recomputed from its lines, its
`updated` timestamp moves, and an `invoice.updated` event is emitted.

### Add lines

`POST /v1/invoices/{id}/add_lines` appends lines to a draft invoice. A line is
either catalog-priced or ad-hoc, exactly as at creation:

```bash
# A catalog-priced line.
curl -s -X POST $BASE/v1/invoices/$IID/add_lines \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "lines[0][price]=$PRICE_ID" \
  --data-urlencode "lines[0][quantity]=2"

# An ad-hoc line.
curl -s -X POST $BASE/v1/invoices/$IID/add_lines \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "lines[0][amount]=500" \
  --data-urlencode "lines[0][description]=Setup fee"
```

The response is the updated invoice. The call is rejected when:

- the invoice is not a draft;
- the `lines` array is missing;
- a line carries neither a `price` nor an `amount`;
- the price does not exist;
- the price is in another currency than the invoice;
- an `amount` is not a non-negative integer.

A rejected call leaves the invoice exactly as it was.

### Remove lines

`POST /v1/invoices/{id}/remove_lines` detaches lines from a draft invoice.
Each item names the `id` of the line to remove:

```bash
LINE_ID=$(curl -s "$BASE/v1/invoices/$IID/lines" -H "Authorization: Bearer $KEY" \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"][0]["id"])')

curl -s -X POST $BASE/v1/invoices/$IID/remove_lines \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "lines[0][id]=$LINE_ID"
```

An item may carry a `behavior` of `delete` or `unassign`; both detach the
line, because the mock has no invoice item for `unassign` to leave behind.
Any other behavior is rejected. Removing a line the invoice does not carry is
rejected too, and leaves the invoice unchanged.

### Update lines

`POST /v1/invoices/{id}/update_lines` changes the lines of a draft invoice.
Each item names the `id` of the line to change and the fields to change:
`quantity`, `amount`, `description`, `price`, or `tax_rate[percentage]`.

```bash
curl -s -X POST $BASE/v1/invoices/$IID/update_lines \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "lines[0][id]=$LINE_ID" \
  --data-urlencode "lines[0][quantity]=3"
```

Updating a `quantity` recomputes the line's amount from its price, and
updating a `tax_rate` recomputes the line's tax. An item that changes no field
is rejected, as is a line id the invoice does not carry.

## Lifecycle

```bash
# draft -> open: sets finalized_at and amount_due.
curl -s -X POST $BASE/v1/invoices/$IID/finalize -H "Authorization: Bearer $KEY"

# open -> paid.
curl -s -X POST $BASE/v1/invoices/$IID/pay -H "Authorization: Bearer $KEY"

# draft|open -> void.
curl -s -X POST $BASE/v1/invoices/$IID/void -H "Authorization: Bearer $KEY"

# open -> uncollectible.
curl -s -X POST $BASE/v1/invoices/$IID/mark_uncollectible -H "Authorization: Bearer $KEY"
```

Paying an invoice creates a payment intent for the outstanding balance and
settles it as `succeeded`. The response is the invoice, with `payment_intent`
holding the id of the payment intent that settled it.

Finalizing an invoice that has no line is rejected, because the lines of an
invoice created empty are only added afterwards.

## Delete an invoice

A draft invoice can be deleted, which is how an application rolls back a draft
it no longer needs:

```bash
curl -s -X DELETE $BASE/v1/invoices/$IID -H "Authorization: Bearer $KEY"
```

The response is a tombstone, as on Stripe:

```json
{"id": "in_...", "object": "invoice", "deleted": true}
```

The invoice is removed: it can no longer be read and it is gone from the
invoice list. An invoice that is not a draft cannot be deleted — a finalized
invoice has to be voided instead — and the call is rejected with an
`invalid_request_error`.

## Payment intents

Payment intents can also be created directly, optionally attached to a
customer and an invoice:

```bash
PI=$(curl -s -X POST $BASE/v1/payment_intents \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "amount=2000" \
  --data-urlencode "currency=usd" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "invoice=$IID")
PIID=$(echo "$PI" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

curl -s "$BASE/v1/payment_intents/$PIID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/payment_intents?invoice=$IID" -H "Authorization: Bearer $KEY"
```

A new payment intent has a `pi_` id and a `status` of
`requires_payment_method`.

### Resolving a payment intent

The mock has no payment methods, so a test settles a payment intent through
the control plane:

```bash
curl -s -X POST $BASE/internal/payment_intents/$PIID/resolve \
  -H 'Content-Type: application/json' \
  -d '{"status":"succeeded"}'
```

`status` is one of `succeeded`, `failed`, or `canceled`; anything else is
rejected with an `invalid_request_error`. The endpoint needs no Stripe
authentication, like the rest of `/internal/**`.

When a payment intent that references an invoice is resolved as `succeeded`,
its amount is added to the invoice's `amount_paid`, `amount_remaining` shrinks
by the same amount, and the invoice becomes `paid` once the balance is covered.
A partial payment leaves the invoice `open` with the remaining balance visible.
A `failed` resolution leaves the invoice amounts and status unchanged.

## Events

The customer and invoice flow emits:

- `customer.created`, `customer.updated`, `customer.deleted`
- `invoice.created`, `invoice.finalized`, `invoice.paid`,
  `invoice.payment_succeeded`, `invoice.payment_failed`, `invoice.voided`,
  `invoice.marked_uncollectible`, `invoice.updated`, `invoice.deleted`
- `payment_intent.created`, `payment_intent.succeeded`,
  `payment_intent.payment_failed`, `payment_intent.canceled`

`invoice.updated` is emitted whenever the lines of an invoice are added to,
removed from, or updated; a rejected change emits nothing.

`invoice.deleted` is emitted when a draft invoice is deleted.

`payment_intent.succeeded` is also emitted by the checkout completion flow, so
an application can handle one event name for both sources. See
[`webhooks.md`](webhooks.md) for delivery, signing, and filtering.

## The hosted invoice page

Once an invoice is finalized it has a `hosted_invoice_url` and an
`invoice_pdf`, both built from the configured `public_base_url`:

```json
{
  "hosted_invoice_url": "http://127.0.0.1:12111/mock/invoices/in_...",
  "invoice_pdf": "http://127.0.0.1:12111/mock/invoices/in_.../pdf"
}
```

The page describes the invoice — its status, its lines, and its totals — and
the PDF holds the same summary:

```bash
curl -s "$BASE/mock/invoices/$IID"
curl -s "$BASE/mock/invoices/$IID/pdf" -o invoice.pdf
```

A draft invoice has neither, as on Stripe: the page is published when the
invoice is finalized.

## Not supported

- No `/v1/tax_rates` resource — tax rates are given inline on a line.
- No `/v1/invoiceitems`; the lines live on the invoice itself and can be
  added, removed, and updated while it is a draft.
- No `past_due` status and no automatic transitions — the mock has no
  server-side time advance.
- No refunds, credit notes, or dunning.
- `number` is always `null`.
