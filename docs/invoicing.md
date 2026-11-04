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

An invoice needs a `customer` and at least one line. A line is either priced
from the catalog (`lines[][price]`, with an optional `lines[][quantity]`) or
written directly (`lines[][amount]`, with an optional
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
and `lines` computed from the catalog prices. `quantity` defaults to 1. Every
line must use the same currency; a mismatch is rejected.

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
`amount` is rejected, and so is an invoice without any line.

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
  `invoice.marked_uncollectible`, `invoice.updated`
- `payment_intent.created`, `payment_intent.succeeded`,
  `payment_intent.payment_failed`, `payment_intent.canceled`

`invoice.updated` is emitted whenever the lines of an invoice are added to,
removed from, or updated; a rejected change emits nothing.

`payment_intent.succeeded` is also emitted by the checkout completion flow, so
an application can handle one event name for both sources. See
[`webhooks.md`](webhooks.md) for delivery, signing, and filtering.

## Not supported

- No hosted invoice page and no `hosted_invoice_url`; invoices are read and
  driven through the API and the control plane.
- No `/v1/tax_rates` resource — tax rates are given inline on a line.
- No `/v1/invoiceitems`; the lines live on the invoice itself and can be
  added, removed, and updated while it is a draft.
- No `past_due` status and no automatic transitions — the mock has no
  server-side time advance.
- No refunds, credit notes, or dunning.
- `number` is always `null`.
