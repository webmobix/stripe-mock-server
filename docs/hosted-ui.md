# Hosted simulation UI

The mock serves two server-rendered HTML pages: **onboarding** and **checkout**.
Both share a simulation panel. Pages use Tailwind and daisyUI loaded from a CDN
and deliberately ship **no client-side framework bundle**, so the same pages
work headlessly with plain HTTP.

## Onboarding page

`GET /mock/connect/{token}` renders the onboarding form for the account behind
an account link. The shared simulation panel exposes:

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
The panel exposes:

- card payment — **success** / **declined**
- alternative payment method selection (card / SEPA debit)

Submitting posts to `POST /mock/checkout/{id}` with `card=success`, which
completes the session transactionally and redirects to `success_url`
(substituting `{CHECKOUT_SESSION_ID}`). `card=fail` re-renders with a decline
notice and leaves the session open. `GET /mock/checkout/{id}/cancel` redirects to
`cancel_url`.

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
