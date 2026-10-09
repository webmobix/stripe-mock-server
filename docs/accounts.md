# Accounts

Accounts v2 is the canonical model; v1 is a projection of the same record, so an
`acct_` created through v2 is usable from v1 and vice versa.

## Create a v2 account

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | python3 -c 'import sys,json;print(json.load(sys.stdin)["secret_key"])')

curl -s -X POST $BASE/v2/core/accounts \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "identity": {"business_details": {"name": "Acme"}},
    "configuration": {"merchant": {"capabilities": {"card_payments": {"requested": true}}}},
    "defaults": {"currency": "usd"}
  }'
```

The response is a `v2.core.account` with an `acct_` id, `object`,
`created` (RFC 3339 ms), `livemode`, and `applied_configurations` (e.g.
`["merchant"]`).

## `include` and default-null

For v2 reads, expandable properties are `null` unless named in `include`. This
matches real behavior so tests do not pass on data Stripe would omit.

```bash
ID=acct_...
# identity, requirements and configuration are null:
curl -s "$BASE/v2/core/accounts/$ID" -H "Authorization: Bearer $KEY"
# populated:
curl -s "$BASE/v2/core/accounts/$ID?include[]=identity&include[]=requirements&include[]=configuration" \
  -H "Authorization: Bearer $KEY"
```

## List accounts

`GET /v2/core/accounts?limit=2` uses `limit`/`starting_after` and returns
`next_page_url` when more remain. `GET /v1/accounts` returns the v1 list
envelope.

## v1 projection

```bash
curl -s "$BASE/v1/accounts/$ID" -H "Authorization: Bearer $KEY"
```

Returns an `account` object with epoch `created` and derived fields:
`charges_enabled`, `payouts_enabled`, `details_submitted`, `capabilities`, and
`requirements`.

## Account links

v1:

```bash
curl -s -X POST $BASE/v1/account_links \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "account=$ID" \
  --data-urlencode "type=account_onboarding" \
  --data-urlencode "refresh_url=https://example.com/refresh" \
  --data-urlencode "return_url=https://example.com/return"
```

v2:

```bash
curl -s -X POST $BASE/v2/core/account_links \
  -H "Authorization: Bearer $KEY" \
  -H "Content-Type: application/json" \
  -d "{\"account\":\"$ID\",\"use_case\":{\"account_onboarding\":{\"refresh_url\":\"https://example.com/refresh\",\"return_url\":\"https://example.com/return\"}}}"
```

Both return a `url` pointing at the mock onboarding page. See
[`hosted-ui.md`](hosted-ui.md) for driving it.

## Onboarding state transitions

Completing the mock onboarding page:

- **KYC pass** — sets `details_submitted=true`, activates requested
  capabilities, derives `charges_enabled`/`payouts_enabled=true`, and emits an
  `account.updated` event. Retrieve with `include=configuration` to see each
  requested capability reported as `{"status":"active","status_details":[]}`
  under `configuration.<section>.capabilities`.
- **KYC fail** — leaves `details_submitted=false`, records unsatisfied
  `requirements`, and activates no capabilities.

On completion (either outcome) the user is redirected to `return_url` (pass) or
`refresh_url` (fail/stale), as configured on the link.
