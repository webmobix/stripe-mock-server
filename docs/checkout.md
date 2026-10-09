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
