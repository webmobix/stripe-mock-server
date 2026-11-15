# API foundations

The protocol layer shared by every implemented endpoint. See the
`api-foundations` spec for the normative requirements.

## Versions

One process serves both conventions:

| | v1 (`/v1/**`) | v2 core (`/v2/**`) |
|---|---|---|
| Request body | `application/x-www-form-urlencoded` | JSON |
| Timestamps | Unix epoch **seconds** (integer) | RFC 3339 UTC with **milliseconds** (`2026-10-09T08:55:23.000Z`) |
| Errors | `{"error":{"type","message","code?","param?"}}` | `{"error":{"type","code","message"}}` |

The `Stripe-Version` header is accepted but not validated; events echo the
implemented API version `2026-09-30.endive`.

## Authentication

The server accepts an API key via:

- `Authorization: Bearer <key>`
- `Authorization: Basic <key>`
- `Authorization: Basic <base64(<key> + ":")>`

A missing or unknown key returns `401 authentication_error`. Generated keys are
persisted, so they survive restarts.

## Account context

Every request resolves to a target account:

- The account owning the key, by default.
- The account named by `Stripe-Account`, when present. A platform (main) key may
  act as any existing connected account; a non-platform key may only act as
  itself. An unknown `acct_` returns `401` and mutates nothing.

Objects are scoped to the resolved account; see `docs/catalog.md` for the
isolation example.

## Idempotency keys

`POST`, `DELETE`, and `PATCH` requests under `/v1/` accept an `Idempotency-Key`
header. The first response is cached (status, body, and content type) and
replayed for the same key, credential, method, path, and request body. Replays
carry `Idempotent-Replay: true`.

```bash
curl -s -X POST $BASE/v1/customers \
  -H "Authorization: Bearer $KEY" \
  -H "Idempotency-Key: customer-1" \
  -d 'email=buyer@example.com'
```

Reusing a key with a different body is rejected with Stripe's idempotency
envelope and no handler runs:

```json
{
  "error": {
    "type": "idempotency_error",
    "message": "Keys for idempotent requests can only be used with the same parameters they were first used with. ..."
  }
}
```

Keys are scoped to the presented credential and cleared by
`POST /internal/reset`. Server errors (HTTP 5xx) are not cached.

## Error envelopes

v1:

```json
{"error":{"type":"invalid_request_error","message":"Missing required param: name","code":"parameter_missing","param":"name"}}
```

v2:

```json
{"error":{"type":"invalid_request_error","code":"parameter_invalid","message":"At least one configuration is required"}}
```

HTTP status maps to the class: `400` invalid request, `401` authentication,
`404` resource missing, `500` internal.

## List envelopes

v1:

```json
{"object":"list","data":[...],"has_more":true,"url":"/v1/products"}
```

`limit`, `starting_after`, and `ending_before` are supported. Items are ordered
newest first, with insertion order as a deterministic tiebreaker.

v2 (`GET /v2/core/accounts`) additionally returns `next_page_url`.

## Response expansion

Endpoints that return expandable references accept repeated `expand[]`
parameters. Retrieval and list requests read them from the query string;
create and update requests read them from the form body. Each path is
dot-separated, traverses every element of an embedded list, and replaces a
reference id with the full object it names. On list requests, a path is also
resolved against the list envelope, so `data.product` expands every returned
item's product. Item-relative list paths such as `product` remain supported.

Accepted parameter shapes:

| Shape | Example |
|-------|---------|
| repeated | `expand[]=product&expand[]=price.product` |
| bare | `expand=product` |
| indexed | `expand[0]=product&expand[1]=customer` |
| comma-separated | `expand=product,price.product` |

Supported reference fields: `product`, `price`, `customer`, `subscription`,
`payment_intent`, `payment_method`, and `default_payment_method`. A field that
already holds an object is left as-is.
Unknown paths, unresolvable ids, and references owned by another account leave
the original value untouched and raise no error.

```bash
curl -s "http://127.0.0.1:12111/v1/prices/price_123?expand[]=product" \
  -H "Authorization: Bearer $STRIPE_SECRET_KEY"
```

The response's `product` is then the full `product` object instead of its
`prod_` id. Nested paths include `items.data.price.product` on subscriptions
and `line_items.data.price.product` on checkout sessions.

To expand a list, use the envelope-relative path:

```bash
curl -s "http://127.0.0.1:12111/v1/prices?lookup_keys[]=normal_monthly_flat&expand[]=data.product" \
  -H "Authorization: Bearer $STRIPE_SECRET_KEY"
```

Every returned price then carries its full `product` object. The item-relative
list form `expand[]=product` remains accepted for compatibility.

## Prefixed identifiers and keys

| Prefix | Object |
|--------|--------|
| `acct_` | account |
| `prod_` | product |
| `price_` | price |
| `cus_` | customer |
| `txi_` | customer tax id |
| `in_` | invoice |
| `txr_` | tax rate |
| `il_` | invoice line |
| `sub_` | subscription |
| `cs_test_` | checkout session |
| `coupon_` | coupon |
| `bps_` | billing portal session |
| `pi_` | payment intent |
| `pm_` | payment method |
| `mtr_` | billing meter |
| `mtrusg_` | meter event summary |
| `evt_` | event |
| `sk_test_` | secret API key |
| `pk_test_` | publishable API key |
| `whsec_` | webhook signing secret |

Keys and signing secrets are persisted; previously generated material is
unchanged after a restart.

## Form parsing

v1 bodies use Stripe's bracketed notation, which is expanded into nested JSON:

| Input | Result |
|-------|--------|
| `amount=1000&currency=usd` | `{"amount":"1000","currency":"usd"}` |
| `line_items[0][price]=price_1` | `{"line_items":[{"price":"price_1"}]}` |
| `items[][price]=a&items[][price]=b` | `{"items":[{"price":"a"},{"price":"b"}]}` |
| `a=1&a=2` | `{"a":"2"}` (last wins) |

Keys and values are percent-decoded first.
