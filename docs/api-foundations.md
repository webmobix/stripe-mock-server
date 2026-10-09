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

## Prefixed identifiers and keys

| Prefix | Object |
|--------|--------|
| `acct_` | account |
| `prod_` | product |
| `price_` | price |
| `cus_` | customer |
| `sub_` | subscription |
| `cs_test_` | checkout session |
| `pi_` | payment intent |
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
