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

## Lookup keys

A price may carry a `lookup_key`, which the consuming app can use instead of an
id to resolve a plan:

```bash
PRO=$(curl -s -X POST $BASE/v1/prices \
  -H "Authorization: Bearer $KEY" \
  -d "product=$PID&currency=usd&unit_amount=1000&lookup_key=pro_monthly")
PRICE_ID=$(echo "$PRO" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
curl -s "$BASE/v1/prices/$PRICE_ID" -H "Authorization: Bearer $KEY"
```

The price response always includes `lookup_key`, as `null` when absent.
Lookup keys are unique among an account's active prices. A duplicate is
rejected with `invalid_request_error` naming `lookup_key`; pass
`transfer_lookup_key=true` to move the key from the previous price, which then
reports `lookup_key` as `null`:

```bash
curl -s -X POST $BASE/v1/prices \
  -H "Authorization: Bearer $KEY" \
  -d "product=$PID&currency=usd&unit_amount=1500&lookup_key=pro_monthly&transfer_lookup_key=true"
```

Filter prices by one or more keys with repeated `lookup_keys[]` values:

```bash
curl -s "$BASE/v1/prices?lookup_keys[]=pro_monthly&lookup_keys[]=team_monthly" \
  -H "Authorization: Bearer $KEY"
```

A key with no matching price returns an empty list. Keys are account-scoped, so
two connected accounts may each hold the same key.

## Product expansion

Price responses accept `expand[]` on create, retrieve, and list, as well as
inside subscription and checkout responses:

```bash
curl -s "$BASE/v1/prices/$PRICE_ID?expand[]=product" \
  -H "Authorization: Bearer $KEY"
```

On list responses, paths are resolved against the list envelope, so use
`data.product` to expand every returned price:

```bash
curl -s "$BASE/v1/prices?lookup_keys[]=normal_monthly_flat&expand[]=data.product" \
  -H "Authorization: Bearer $KEY"
```

The item-relative list form `expand[]=product` remains accepted for
compatibility. The `product` field is then the full product object instead of
its `prod_` id. See `docs/api-foundations.md` for every accepted expansion
shape.

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
