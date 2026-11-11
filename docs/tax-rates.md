# Tax rates

Tax rates are account-scoped percentages used to tax checkout line items.
See the `tax-rates` spec for the normative requirements. Tax-rate calls do not
emit events.

## Create a tax rate

`display_name`, `inclusive`, and `percentage` are required. `percentage` must
be between 0 and 100. The server assigns a `txr_` id and defaults `active` to
true:

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

EXCLUSIVE=$(curl -s -X POST $BASE/v1/tax_rates \
  -H "Authorization: Bearer $KEY" \
  -d 'display_name=VAT&inclusive=false&percentage=20&country=DE')
TXR=$(echo "$EXCLUSIVE" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')

curl -s -X POST $BASE/v1/tax_rates \
  -H "Authorization: Bearer $KEY" \
  -d 'display_name=Included+VAT&inclusive=true&percentage=10'
```

The response is a `tax_rate` with `object`, `active`, `inclusive`, `percentage`,
and the optional `country`, `state`, `jurisdiction`, `description`, `tax_type`,
`metadata`, `livemode`, and `created`.

To create an inactive rate, pass `active=false`:

```bash
curl -s -X POST $BASE/v1/tax_rates \
  -H "Authorization: Bearer $KEY" \
  -d 'display_name=Old+tax&inclusive=false&percentage=5&active=false'
```

## Retrieve and list

```bash
curl -s $BASE/v1/tax_rates/$TXR -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/tax_rates?active=true" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/tax_rates?inclusive=false" -H "Authorization: Bearer $KEY"
```

Lists use the v1 envelope and support `limit`, `starting_after`, and
`ending_before`. `active` and `inclusive` filter the results where supplied.

An unknown id, or one belonging to another account, returns `404`.

Use the ids on a checkout session; see `docs/checkout.md`.
