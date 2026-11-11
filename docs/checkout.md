# Checkout

Checkout sessions support `payment`, `subscription`, and `setup` modes.

## Create a session

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

SESSION=$(curl -s -X POST $BASE/v1/checkout/sessions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "mode=payment" \
  --data-urlencode "line_items[0][price]=$PRICE_ID" \
  --data-urlencode "line_items[0][quantity]=2" \
  --data-urlencode "success_url=https://example.com/success?session={CHECKOUT_SESSION_ID}" \
  --data-urlencode "cancel_url=https://example.com/cancel")
SID=$(echo "$SESSION" | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
```

The response is a `cs_test_` session with `status=open`, `amount_total`,
`line_items`, and a `url` pointing at the mock checkout page. Optional
`customer` or `customer_account` fields are recorded.

## Customer email

A session may carry `customer_email`. When neither `customer` nor
`customer_account` is supplied, the server immediately creates a `cus_`
customer with that email, links it to the session, and emits
`customer.created`:

```bash
curl -s -X POST $BASE/v1/checkout/sessions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "mode=payment" \
  --data-urlencode "line_items[0][price]=$PRICE_ID" \
  --data-urlencode "customer_email=guest@example.com"
```

The session reports `customer_email` and `customer_details.email`. When a
customer is already supplied, no additional customer is created; the supplied
`customer_email` is kept, and omitting it falls back to the linked customer's
email. See [`billing.md`](billing.md) for customer events.

## Coupons

Create coupons first (see [`coupons.md`](coupons.md)), then name them with
repeated `discounts[][coupon]` values:

```bash
curl -s -X POST $BASE/v1/checkout/sessions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "mode=payment" \
  --data-urlencode "line_items[0][price]=$PRICE_ID" \
  --data-urlencode "line_items[0][quantity]=2" \
  --data-urlencode "discounts[0][coupon]=$COUPON_ID"
```

Each coupon must exist for the account, be valid, match the session currency
when it carries a fixed amount, and apply to every line's product. The
discount reduces the subtotal and never goes below zero. Coupons apply
sequentially in request order.

## Tax rates

Create tax rates first (see [`tax-rates.md`](tax-rates.md)). Session-level
`tax_rates[]` apply to every line; a line's own `line_items[][tax_rates][]`
takes precedence:

```bash
curl -s -X POST $BASE/v1/checkout/sessions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "mode=payment" \
  --data-urlencode "line_items[0][price]=$PRICE_ID" \
  --data-urlencode "tax_rates[]=$TXR_ID"
```

Each referenced rate must exist for the account and be active. Tax is computed
on the discounted line amount. Exclusive tax is added to the total; inclusive
tax is reported in `amount_tax` but is already inside the line amount.

## Totals

Sessions report the full breakdown:

```bash
curl -s "$BASE/v1/checkout/sessions/$SID" -H "Authorization: Bearer $KEY"
```

- `amount_subtotal` — line amounts before discount and tax.
- `amount_discount` — the applied coupon discount.
- `amount_tax` — the sum of line tax amounts.
- `amount_total` — the discounted subtotal plus exclusive tax.
- `discounts` — the applied coupon snapshots and their amounts.
- `total_details.amount_discount`, `total_details.amount_tax`
- `total_details.breakdown.discounts[]` — `amount` and `coupon` id per applied
  coupon.
- `total_details.breakdown.taxes[]` — `amount`, `taxable_amount`, and
  `tax_rate` id, aggregated by rate.

For example, a 2000 subtotal with a 25 percent coupon and exclusive 10 percent
tax reports `amount_subtotal=2000`, `amount_discount=500`, `amount_tax=150`,
and `amount_total=1650`. A session without discounts or tax reports both as
zero and an `amount_total` equal to `amount_subtotal`. All totals survive
retrieval after completion, and a `payment`-mode session charges its
`amount_total`.

## Retrieve

```bash
curl -s "$BASE/v1/checkout/sessions/$SID" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/checkout/sessions/$SID/line_items" -H "Authorization: Bearer $KEY"
```

`line_items` uses the v1 list envelope.

## Completion

Completing the mock checkout page (see [`hosted-ui.md`](hosted-ui.md)) runs a
single transaction that:

- marks the session `complete` (and `payment_status=paid`),
- for `payment` mode creates a **succeeded** `pi_` payment intent,
- for `subscription` mode creates an active `sub_` subscription,
- for `setup` mode creates neither,
- creates the associated events: `payment_intent.succeeded` (payment mode),
  `customer.subscription.created` (subscription mode), and
  `checkout.session.completed`.

Because state and events commit before the response returns, you can retrieve
the session and list events immediately afterwards.

On success the browser is redirected to `success_url` with
`{CHECKOUT_SESSION_ID}` substituted; a declined simulation leaves the session
open.

## Expiry

```bash
curl -s -X POST "$BASE/v1/checkout/sessions/$SID/expire" \
  -H "Authorization: Bearer $KEY"
```

Sets `status=expired` and emits `checkout.session.expired`. Only open sessions
can be expired.
