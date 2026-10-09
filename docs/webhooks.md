# Webhooks and events

## Synchronous events, asynchronous delivery

When a request changes state, the `evt_` Event object and its `pending` delivery
rows are written **in the same transaction as the state change**, before the
response returns. Only the outbound HTTP call is deferred. A test can therefore
retrieve state and list events immediately after an API call returns.

```bash
curl -s "$BASE/v1/events?type=checkout.session.completed" -H "Authorization: Bearer $KEY"
curl -s "$BASE/v1/events/$EVENT_ID" -H "Authorization: Bearer $KEY"
```

## Event shape

Events are serialized as **v1 snapshot** events: `id`, `object: "event"`, `type`,
`data.object` inline, epoch `created`, `api_version`, and `account` when the
source account is connected.

```json
{
  "id": "evt_...",
  "object": "event",
  "api_version": "2026-09-30.endive",
  "created": 1760000000,
  "type": "account.updated",
  "account": "acct_...",
  "data": { "object": { "id": "acct_...", "object": "account", "...": "..." } },
  "livemode": false,
  "pending_webhooks": 1,
  "request": { "id": null, "idempotency_key": null }
}
```

## Event destinations

Destinations are declared in [`config.toml`](configuration.md#event_destinations)
or created at runtime via `POST /v2/core/event_destinations`. Each chooses one
`event_payload` (`snapshot` or `thin`), the accounts it listens to (`events_from`:
`@self` / `@accounts`), an `enabled_events` allow-list (`*`, exact names,
`prefix.*`), and a `webhook_endpoint.url`. Selection is by payload format, scope,
then event filter. A destination picks exactly one payload format; register two
to receive the same event in both formats, exactly as Stripe does (the
"dual-destination" strategy). Both may point at the **same URL** and there they
share a single `whsec_` secret. The payload format does **not** restrict the event
family: `enabled_events` and `events_from` are lists, so a single destination can
receive both v1-derived (`v1.*`) and v2-derived (`v2.core.*`) event types. See
[`configuration.md`](configuration.md#payload-format-is-not-the-same-as-event-family).

## Delivery

Each target owns a FIFO queue. Deliveries are sent in emit order, spaced a fixed
**200 ms** apart, so a response always returns before its webhook and two events
for the same target arrive ordered ~200 ms apart. Each attempt uses:

- `Stripe-Signature: t=<ts>,v1=<HMAC_SHA256(secret, "<ts>.<payload>")>`
- `X-Stripe-Mock-Delivery-Id: dl_...` (deduplication)
- `X-Stripe-Mock-Event-Id: evt_...`

`pending_webhooks` on a snapshot event is the number of its deliveries not yet
settled, and a destination's `snapshot_api_version` sets the delivered
`api_version`. A delivery to a destination that has since been disabled is marked
`failed_exhausted` without being sent.

A non-2xx response is retried **inline up to three attempts**, then the delivery
is marked `failed_exhausted` and the queue advances. Every attempt is recorded
in the delivery journal with status, HTTP status, and timestamps.

### Verifying a signature

With the Node SDK:

```js
const event = stripe.webhooks.constructEvent(rawBody, signatureHeader, whsecSecret);
```

The `whsec_` secret is available from `/internal/destinations`
(`webhook_endpoint.signing_secret`); see
[`internal.md`](internal.md). There is **one secret per endpoint URL**: snapshot
and thin destinations configured with the same `webhook_endpoint.url` share it,
so a single receiver verifies both formats with the same secret. (Real Stripe
gives each destination its own secret; the mock shares it by URL as a
convenience.) The end-to-end script in `scripts/sdk-e2e/` performs exactly this
verification.

## v2 thin events

v2 Accounts use **thin events** and **event destinations**. A thin event is an
unversioned notification (`v2.core.event`) that identifies the related resource;
the full versioned event is fetched separately. Destinations are the same
concept as v1 webhooks: one destination per payload format, so a snapshot
destination and a thin destination can point at the same receiver.

### Managing destinations

Destinations can be created at runtime through the v2 API:

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | jq -r .secret_key)

curl -s -X POST $BASE/v2/core/event_destinations \
  -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"name":"thin","type":"webhook_endpoint","event_payload":"thin",
       "enabled_events":["v2.core.account.created"],
       "webhook_endpoint":{"url":"http://localhost:4000/thin"},
       "include":["webhook_endpoint.signing_secret"]}'
```

The object is `v2.core.event_destination` with an `ed_test_` id. Lifecycle
endpoints are `POST /v2/core/event_destinations/{id}` (update),
`.../{id}/enable`, `.../{id}/disable`, `.../{id}/ping` (thin destinations only; it bypasses the event filter), and `DELETE`. Only
`type = "webhook_endpoint"` is supported; other transports return a v2
`invalid_request_error`. Both `event_payload = "thin"` and `"snapshot"` are
supported. `enabled_events` accepts exact names, `prefix.*`, and `*`.

For a fully config-driven setup you can also declare destinations in
`config.toml` with `[[event_destinations]]` (see
[`configuration.md`](configuration.md#event_destinations)); they are seeded at
boot and receive the same events as API-created ones.

### Emission and matching

`POST /v2/core/accounts` emits `v2.core.account.created` in the same transaction
as the account, with `related_object` pointing at the new account. A v2 Account
state change emits **both** event families, as Stripe does:

- `POST /v2/core/accounts/{id}` emits the v1 family — a snapshot `account.updated`
  and a thin `v1.account.updated` — plus `v2.core.account.updated`,
  `v2.core.account[configuration.<section>].capability_status_updated` for each
  newly requested capability, and
  `v2.core.account[configuration.<section>].updated` for each changed section.
- Completing onboarding emits a snapshot `account.updated`, a thin
  `v1.account.updated`, and `v2.core.account[configuration.<section>].capability_status_updated`
  (KYC pass) or `v2.core.account[requirements].updated` (KYC fail), plus
  `v2.core.account.updated`.

The v1 family uses the **Connected accounts** scope (`events_from: ["@accounts"]`)
while the v2 family uses the **Your account** scope (`events_from: ["@self"]`).
Because these are lists, a **single** thin destination with
`events_from = ["@self", "@accounts"]` receives both the thin `v1.account.updated`
and the thin `v2.core.account.*` notifications — no second destination required.
An event is delivered to every **enabled** destination whose payload format,
scope, and `enabled_events` all match. Snapshot destinations never receive thin
events (and vice versa); a second destination is only needed to receive the same
event in the *other* payload format.

### Retrieving the account after a thin event

Thin events carry no state; re-fetch the resource. `GET /v2/core/accounts/{id}`
returns `configuration` only when requested. Both `include=configuration`
(all sections) and `include[0]=configuration.merchant` (one section, the form
stripe-node sends) work, as do `include[]=...` and comma-separated values.
Without an include, `configuration` is `null`.

### Delivered vs fetched shape

The delivered webhook body is the thin payload only:

```json
{
  "id": "evt_test_...",
  "object": "v2.core.event",
  "type": "v2.core.account.created",
  "created": "2026-10-09T08:55:23.000Z",
  "livemode": false,
  "related_object": {
    "id": "acct_...",
    "type": "v2.core.account",
    "url": "/v2/core/accounts/acct_..."
  }
}
```

`GET /v2/core/events/{id}` returns the full versioned event, adding `data`,
`changes`, `reason`, and `context`. `GET /v2/core/events` lists them newest-first
and supports `types` and `object_id` filters. A platform key sees its own events plus
connected-scope thin events (e.g. `v1.account.updated`); other keys see only their own.

### Signing

Thin deliveries use the destination's `whsec_` secret with the same
`Stripe-Signature: t=<ts>,v1=<HMAC_SHA256(secret, "<ts>.<payload>")>` scheme and
the `X-Stripe-Mock-Delivery-Id` / `X-Stripe-Mock-Event-Id` headers as v1
deliveries. Parse thin events with the SDK's thin-event verifier — in
`stripe` v23 this is `stripe.parseEventNotification(payload, signatureHeader,
secret)` (renamed from `parseThinEvent`) — then fetch the full event with
`stripe.v2.core.events.retrieve(id)`. Retrieve the secret
from the create response include or from `/internal/destinations`.

## Deterministic testing

Never sleep-and-hope. Use the internal control plane to wait on delivery state:

- `GET /internal/deliveries` — the journal.
- `POST /internal/deliveries/wait` — block until a given event's deliveries
  settle or time out.
- `POST /internal/deliveries/flush` — block until all pending deliveries settle.
- `POST /internal/deliveries/{id}/retry` — re-queue a terminal delivery.

## Deferred engine variant

The chosen engine is **per-target FIFO** (ordered, ~200 ms spacing). The
alternative — concurrent per-event delivery, matching real Stripe's unordered
behavior and higher throughput — is deferred behind the `DeliveryEngine` trait.
Both share the journal, signatures, and internal endpoints.
