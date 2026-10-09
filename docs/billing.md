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
