# Catalog

Products and prices are owned by the resolved account and isolated between the
platform and each connected account.

## Create and retrieve products

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

PRODUCT=$(curl -s -X POST $BASE/v1/products \
  -H "Authorization: Bearer $KEY" \
  -d 'name=Widget&description=A+widget')
PID=$(echo "$PRODUCT" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

curl -s "$BASE/v1/products/$PID" -H "Authorization: Bearer $KEY"
```

A `product` has a `prod_` id, `object`, `active`, epoch `created`, `name`,
`default_price`, and `metadata`.

## Create and retrieve prices

One-time:

```bash
PRICE=$(curl -s -X POST $BASE/v1/prices \
  -H "Authorization: Bearer $KEY" \
  -d "product=$PID&currency=usd&unit_amount=1000")
```

Recurring:

```bash
curl -s -X POST $BASE/v1/prices \
  -H "Authorization: Bearer $KEY" \
  -d "product=$PID&currency=usd&unit_amount=2000&recurring[interval]=month"
```

A `price` has a `price_` id, `currency`, `unit_amount`, `type`
(`one_time`/`recurring`), and an optional `recurring.interval`. Creating a price
updates the product's `default_price`.

## Lists and filters

```bash
curl -s "$BASE/v1/products?limit=2" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/prices?product=$PID" -H "Authorization: Bearer $KEY"
```

Both use the v1 list envelope.

## Account scoping

Create a product for a connected account with `Stripe-Account`:

```bash
ACCT=acct_...
curl -s -X POST $BASE/v1/products \
  -H "Authorization: Bearer $KEY" \
  -H "Stripe-Account: $ACCT" \
  -d 'name=Connected+Widget'
```

Retrieving that product id **without** the header returns `404`: catalog entries
are invisible across accounts.
