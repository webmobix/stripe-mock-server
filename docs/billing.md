# Billing

Customers and subscriptions are account-scoped like the catalog.

## Customers

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

CUSTOMER=$(curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" \
  -d 'email=buyer@example.com&name=Buyer')
CID=$(echo "$CUSTOMER" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

curl -s "$BASE/v1/customers/$CID" -H "Authorization: Bearer $KEY"
```

A completed checkout saves a simulated Visa card ending `4242` and sets it as
the customer's `invoice_settings.default_payment_method`; read it directly or
with `expand[]=invoice_settings.default_payment_method`. See
[`checkout.md`](checkout.md#completion).

### Tax identification

A customer may carry tax ids. Supply them as `tax_id_data[][type]` and
`tax_id_data[][value]`; each becomes a `tax_ids` entry with a `txi_` id.

```bash
curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'email=buyer@example.com' \
  --data-urlencode 'tax_id_data[0][type]=eu_vat' \
  --data-urlencode 'tax_id_data[0][value]=DE123456789'
```

A tax id entry with a type but no value is rejected with an
`invalid_request_error`.

### Currency

A customer may carry a default `currency`. It is the currency the invoices
raised against the customer are priced in, unless the invoice names one
itself.

```bash
curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode 'email=buyer@example.com' \
  --data-urlencode 'currency=eur'
```

A `currency` that is not a three-letter ISO code is rejected with an
`invalid_request_error`.

### List, update, and delete

```bash
# List, optionally filtered by email.
curl -s "$BASE/v1/customers" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/customers?email=buyer@example.com" -H "Authorization: Bearer $KEY"

# Update. A tax id update appends to the existing entries.
curl -s -X POST $BASE/v1/customers/$CID \
  -H "Authorization: Bearer $KEY" \
  -d 'name=New Name'

# Delete. The customer is no longer readable afterwards.
curl -s -X DELETE $BASE/v1/customers/$CID -H "Authorization: Bearer $KEY"
```

### Events

- Create → `customer.created`.
- Update → `customer.updated`.
- Delete → `customer.deleted`.

See [`invoicing.md`](invoicing.md) for invoices and payment intents.

## Subscriptions

Create a price first (see [`catalog.md`](catalog.md)), then:

```bash
SUB=$(curl -s -X POST $BASE/v1/subscriptions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "items[0][price]=$PRICE_ID" \
  --data-urlencode "items[0][quantity]=1")
SID=$(echo "$SUB" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
```

The `sub_` object includes `status`, `items.data` (each with an embedded price),
`customer` (or `customer_account`), and period fields. Every item in
`items.data` also carries its own `subscription`, `current_period_start`, and
`current_period_end`, matching the subscription period, on create, retrieve,
list, and update responses. Creating a subscription also raises its first
invoice with `billing_reason=subscription_create`, a paid status, and one line
per item. For an account-backed customer:

```bash
curl -s -X POST $BASE/v1/subscriptions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer_account=acct_..." \
  --data-urlencode "items[0][price]=$PRICE_ID"
```

### Metered items

A metered price (see [`meters.md`](meters.md)) tracks usage rather than a
quantity. Omit `items[][quantity]`; supplying one is rejected with
`invalid_request_error`. The item reports `quantity: null`. Renewal invoices
bill the usage aggregated over the period that just ended.

## End a billing period

`POST /internal/subscriptions/{id}/advance` ends the current billing period and
collects the renewal invoice. `outcome` is `paid` or `failed`:

```bash
curl -s -X POST "$BASE/internal/subscriptions/$SID/advance" \
  -H 'Content-Type: application/json' \
  -d '{"outcome":"paid"}'
```

The response includes the updated `subscription` and the raised `invoice`. A
paid outcome advances `current_period_start`/`current_period_end`, marks the
subscription `active`, raises a `subscription_cycle` invoice, and emits
`invoice.paid` (`invoice.payment_succeeded` too). A failed outcome leaves the
invoice `open` with `attempt_count=1`, sets the subscription `past_due`, and
emits `invoice.payment_failed`; the period still advances.

When an `open` invoice already exists, the call **retries that invoice** instead
of advancing again. Calling it with `outcome=paid` after a failure therefore
models recovery: the same invoice becomes `paid`, `attempt_count` increments,
and the subscription returns to `active`.

The full sequence — first invoice, renewal, failure, recovery — is:

```bash
# Creation raises the first, paid invoice.
INVOICES=$(curl -s "$BASE/v1/invoices?subscription=$SID" -H "Authorization: Bearer $KEY")

# Renewal fails...
curl -s -X POST "$BASE/internal/subscriptions/$SID/advance" \
  -H 'Content-Type: application/json' -d '{"outcome":"failed"}'

# ...then the same open invoice is paid on the retry.
curl -s -X POST "$BASE/internal/subscriptions/$SID/advance" \
  -H 'Content-Type: application/json' -d '{"outcome":"paid"}'
```

## Update a subscription

`POST /v1/subscriptions/{id}` changes item quantities by item id. The current
item ids are in `items.data[].id`:

```bash
ITEM_ID=$(echo "$SUB" | python3 -c 'import sys,json;print(json.load(sys.stdin)["items"]["data"][0]["id"])')

curl -s -X POST "$BASE/v1/subscriptions/$SID" \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "items[0][id]=$ITEM_ID" \
  --data-urlencode "items[0][quantity]=5"
```

When the subscription has exactly one item, it may be named by price instead:

```bash
curl -s -X POST "$BASE/v1/subscriptions/$SID" \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "items[0][price]=$PRICE_ID" \
  --data-urlencode "items[0][quantity]=2"
```

The same request may set `cancel_at_period_end` and `metadata`. Unknown item
ids, unknown prices, and non-positive quantities are rejected with an
`invalid_request_error` and change nothing.

### Prorations

`proration_behavior` defaults to `create_prorations`; `none` and
`always_invoice` are also accepted. When a quantity changes and prorations are
enabled, the update response includes a top-level `prorations` array with one
entry per changed item:

```json
{
  "id": "pror_...",
  "object": "proration",
  "item": "si_...",
  "price": "price_...",
  "quantity_delta": 4,
  "amount": 4000,
  "currency": "usd",
  "period_start": 1760000000,
  "period_end": 1762592000
}
```

The amount is
`round(unit_amount * quantity_delta * remaining / period_length)`, with
`remaining` clamped to the current period and rounding half away from zero. A
decreased quantity yields a negative amount, `none` returns an empty array, and
`always_invoice` returns the same figures without creating an invoice. These
figures are response-only and are not persisted. An unsupported behavior is
rejected with an `invalid_request_error`.

## Lifecycle and events

- Create → `status=active`, emits `customer.subscription.created`.
- Update → changes items, `cancel_at_period_end`, or `metadata`, emits
  `customer.subscription.updated` with the updated subscription as
  `data.object`.
- Cancel → `DELETE /v1/subscriptions/{id}` sets `status=canceled` and emits
  `customer.subscription.deleted`.

## Retrieve and list

```bash
curl -s "$BASE/v1/subscriptions/$SID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/subscriptions?customer=$CID" -H "Authorization: Bearer $KEY"
```

A canceled subscription remains readable: retrieval returns it with
`status=canceled` instead of `404`, and listing still includes it.
`GET /v1/subscriptions` accepts `customer` and `customer_account` filters and
uses the v1 list envelope.

## Billing portal sessions

Create a `billing_portal.session` for an existing customer with a return URL:

```bash
PORTAL=$(curl -s -X POST $BASE/v1/billing_portal/sessions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer=$CID" \
  --data-urlencode "return_url=https://example.com/account")
echo "$PORTAL" | python3 -c 'import sys,json;print(json.load(sys.stdin)["url"])'
```

The response has a `bps_` id, `created`, the `customer`, the `return_url`, and a
`url` pointing at the mock billing portal page. An unknown customer returns
`404` and creates no session; a missing `return_url` is an
`invalid_request_error`. No event is emitted. See
[`hosted-ui.md`](hosted-ui.md) for the page itself.
