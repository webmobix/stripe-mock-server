# Coupons

Coupons are account-scoped discounts that a checkout session can apply.
See the `coupons` spec for the normative requirements. Coupons do not emit
events.

## Create a coupon

A coupon carries exactly one of `percent_off` (greater than 0 and at most 100)
or `amount_off` (a positive integer in minor units) together with a
three-letter `currency`:

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

curl -s -X POST $BASE/v1/coupons \
  -H "Authorization: Bearer $KEY" \
  -d 'percent_off=25&duration=forever&name=Quarter+off'

curl -s -X POST $BASE/v1/coupons \
  -H "Authorization: Bearer $KEY" \
  -d 'amount_off=500&currency=usd&duration=once'
```

Without a client-supplied `id` the server generates a `coupon_` id. A
client-supplied id may be used instead:

```bash
curl -s -X POST $BASE/v1/coupons \
  -H "Authorization: Bearer $KEY" \
  -d 'id=friends_only&percent_off=10&applies_to[products][0]=prod_123'
```

## Duration and restrictions

`duration` defaults to `once` and may be `once`, `forever`, or `repeating`. A
`repeating` coupon requires a positive `duration_in_months`:

```bash
curl -s -X POST $BASE/v1/coupons \
  -H "Authorization: Bearer $KEY" \
  -d 'percent_off=15&duration=repeating&duration_in_months=3'
```

The server also accepts `max_redemptions`, `redeem_by` (epoch seconds), `name`,
`metadata`, and `applies_to[products][]`. The response exposes `times_redeemed`
and a computed `valid`, which is false once `redeem_by` has passed or
`times_redeemed` reaches `max_redemptions`.

## Retrieve and list

```bash
curl -s $BASE/v1/coupons/coupon_123 -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/coupons?limit=2" -H "Authorization: Bearer $KEY"
```

Lists use the v1 envelope and support `limit`, `starting_after`, and
`ending_before`.

## Delete

```bash
curl -s -X DELETE $BASE/v1/coupons/coupon_123 -H "Authorization: Bearer $KEY"
```

The response is a deleted tombstone:

```json
{"id":"coupon_123","object":"coupon","deleted":true}
```

Retrieving a deleted coupon returns `404`, and it no longer appears in lists.

## Validation rules

| Request | Result |
|---------|--------|
| both `percent_off` and `amount_off` | `invalid_request_error` |
| neither discount form | `invalid_request_error` |
| `percent_off` of 0 or above 100 | `invalid_request_error` naming `percent_off` |
| `amount_off` without `currency` | `invalid_request_error` naming `currency` |
| `duration` other than the three values | `invalid_request_error` naming `duration` |
| `duration=repeating` without a positive month count | `invalid_request_error` naming `duration_in_months` |
| duplicate client-supplied `id` for the account | `invalid_request_error`, existing coupon unchanged |

Discounts are applied to checkout sessions; see `docs/checkout.md`.
