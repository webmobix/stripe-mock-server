# Hosted simulation UI

The mock serves two server-rendered HTML pages: **onboarding** and **checkout**.
Both share a set of simulation controls. Pages use Tailwind and daisyUI loaded
from a CDN and deliberately ship **no client-side framework bundle**, so the same
pages work headlessly with plain HTTP.

## Onboarding page

`GET /mock/connect/{token}` renders the onboarding form for the account behind
an account link. The shared simulation controls expose:

- KYC verification — **pass** / **fail**
- payout IBAN selection

Submitting posts to `POST /mock/connect/{token}` with `kyc=pass` or `kyc=fail`:

- pass → account updated, `details_submitted=true`, capabilities active, an
  `account.updated` event created, redirect to `return_url`.
- fail → requirements recorded, no capability activation, redirect to
  `refresh_url`.
- a stale/expired/used link redirects to `refresh_url` (or renders an error when
  none is configured).

## Checkout page

`GET /mock/checkout/{id}` renders the session summary and a minimal payment form.
The controls expose:

- **Simulate payment** — **Successful** / **Declined**
- alternative payment method selection (card / SEPA debit)

The payment and cancel controls expose stable ids for automation:
`#payment-success`, `#payment-decline`, and `#payment-cancel`. The cancel
control is a button-styled link (`<a id="payment-cancel" class="btn">`) placed
next to the simulate-payment buttons; it navigates to the cancel route rather
than submitting the completion form.

Submitting posts to `POST /mock/checkout/{id}` with `card=success`, which
completes the session transactionally and redirects to `success_url`
(substituting `{CHECKOUT_SESSION_ID}`). `card=fail` re-renders with a decline
notice and leaves the session open. `GET /mock/checkout/{id}/cancel` redirects to
`cancel_url`.

### Static demo session

A well-known session is seeded at boot so the checkout page can be opened
directly, with no product/price/session setup:

```
http://127.0.0.1:12111/mock/checkout/cs_test_static
```

It renders a fixed demo line item (`Static demo item`, 2000 `usd`, `payment`
mode). Completing it runs the normal flow — a succeeded payment intent plus
`checkout.session.completed` and `payment_intent.succeeded` events for the main
account. Because it has no `success_url`, completion shows the in-page success
notice instead of redirecting.

The session is always reusable: opening the static URL when it is `complete` or
`expired` resets it to a fresh `open` demo session, so repeated manual checks
always show the payment form. (`POST /internal/reset` also restores it.) This id
is for manual/visual testing — programmatically created sessions keep their
random `cs_test_...` ids and single-use semantics.

## Driving it headlessly

Because the pages are plain HTML forms, any HTTP client works. With `curl`:

```bash
# 1. create a link, read its url, extract the token
LINK=$(curl -s -X POST $BASE/v1/account_links -H "Authorization: Bearer $KEY" \
  --data-urlencode "account=$ACCT" \
  --data-urlencode "type=account_onboarding" \
  --data-urlencode "refresh_url=https://example.com/refresh" \
  --data-urlencode "return_url=https://example.com/return")
TOKEN=$(echo "$LINK" | python3 -c 'import sys,json;print(json.load(sys.stdin)["url"].rsplit("/",1)[1])')

# 2. complete onboarding with a passing KYC (follow no redirects to inspect Location)
curl -s -i -X POST $BASE/mock/connect/$TOKEN -d 'kyc=pass' | grep -i '^location'

# 3. complete a checkout session
curl -s -i -X POST $BASE/mock/checkout/$SID -d 'card=success' | grep -i '^location'
```

The integration tests use exactly this pattern, asserting the redirect status
and `Location` header.
