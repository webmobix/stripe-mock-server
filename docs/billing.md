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
`customer` (or `customer_account`), and period fields. For an account-backed
customer:

```bash
curl -s -X POST $BASE/v1/subscriptions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "customer_account=acct_..." \
  --data-urlencode "items[0][price]=$PRICE_ID"
```

## Lifecycle and events

- Create → `status=active`, emits `customer.subscription.created`.
- Update is not exposed in the MVP beyond cancellation.
- Cancel → `DELETE /v1/subscriptions/{id}` sets `status=canceled` and emits
  `customer.subscription.deleted`.

## Retrieve and list

```bash
curl -s "$BASE/v1/subscriptions/$SID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/subscriptions?customer=$CID" -H "Authorization: Bearer $KEY"
```

`GET /v1/subscriptions` accepts `customer` and `customer_account` filters and
uses the v1 list envelope.
