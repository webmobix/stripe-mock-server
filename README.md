# stripe-mock

A local, stateful **Stripe-compatible server** for automated tests. Point the
official Stripe SDK at it (via a base-URL override) and run Connect onboarding,
catalog, billing, checkout, and webhook flows deterministically — no Stripe
account, network, or dashboard required.

The server is a single Rust process backed by a SQLite file. It implements a
focused subset of the API across two conventions:

- **v1** (`/v1/**`): form-encoded bodies, epoch-second timestamps.
- **v2 core** (`/v2/**`): JSON bodies, RFC 3339-millisecond timestamps,
  distinct error envelope.

Object identifiers use real Stripe prefixes (`acct_`, `prod_`, `price_`, `cus_`,
`sub_`, `cs_test_`, `pi_`, `evt_`, `sk_test_`, `pk_test_`, `whsec_`), and state
changes create events synchronously while webhook delivery stays asynchronous.

> **This is a test tool.** It intentionally exposes generated secrets and
> never talks to real payment networks. Do not run it in production or on a
> public interface.

## Requirements

- Rust with `edition 2024` support (Rust 1.85+; developed on 1.99).
- Optional: Node.js 18+ to run the official Stripe Node SDK end-to-end script.

## Build

```bash
cargo build --release
```

## Configure

Copy the committed sample and edit it:

```bash
cp config.toml stripe-mock.toml
```

```toml
[server]
bind = "127.0.0.1"
port = 12111
public_base_url = "http://127.0.0.1:12111"

[database]
path = "stripe-mock.db"
reset_on_start = false

[main_account]
business_type = "company"
country = "US"
email = "owner@example.com"
name = "Mock Platform"

[[event_destinations]]
name = "snapshot-app"
event_payload = "snapshot"          # snapshot (v1) | thin (v2)
events_from = ["@self", "@accounts"] # your account | connected accounts
enabled_events = ["*"]              # or exact names / "prefix.*"
webhook_endpoint = { url = "http://localhost:4000/stripe/webhook" }
```

Every field is documented in [`docs/configuration.md`](docs/configuration.md).

## Run

```bash
STRIPE_MOCK_CONFIG=stripe-mock.toml ./target/release/stripe-mock
# or
STRIPE_MOCK_CONFIG=stripe-mock.toml cargo run --release
```

The server binds the configured address and answers a health probe:

```bash
curl -s http://127.0.0.1:12111/health
# {"status":"ok"}
```

On first boot it seeds the main account and generates its keys. Retrieve them
from the local-only control plane:

```bash
curl -s http://127.0.0.1:12111/internal/main
# { "id": "acct_...", "secret_key": "sk_test_...", "publishable_key": "pk_test_...", ... }
```

## Docker

A runtime image is built from [`Dockerfile`](Dockerfile) and published to
Docker Hub. Pull and run it:

```bash
docker run --rm -p 127.0.0.1:12111:12111 -v stripe-mock-data:/data webmobix/stripe-mock-server
curl -s http://127.0.0.1:12111/health
# {"status":"ok"}
```

The image runs as a non-root user, binds `0.0.0.0:12111`, and persists SQLite
state in the `/data` volume. The baked-in configuration lives at
`/etc/stripe-mock/config.toml`; mount your own file and point
`STRIPE_MOCK_CONFIG` at it to override it:

```bash
docker run --rm -p 127.0.0.1:12111:12111 \
  -v "$PWD/stripe-mock.toml:/etc/stripe-mock/config.toml:ro" \
  webmobix/stripe-mock-server
```

Build and run it locally with:

```bash
docker build -t stripe-mock .
docker run --rm -p 127.0.0.1:12111:12111 stripe-mock
```

> Publishing the port also exposes the unauthenticated `/internal/**` API,
> which returns account and webhook secret keys. Bind it to `127.0.0.1`
> (`-p 127.0.0.1:12111:12111`) and never expose it on a public interface.

## Webhook signing secrets

Use the local-only control plane to access webhook secrets. Every event
destination from `[[event_destinations]]` (and every runtime destination) is
returned with its `whsec_` secret. There is **one secret per endpoint URL**: all
destinations that share a `webhook_endpoint.url` return the same secret, so a
single receiver verifies snapshot and thin deliveries alike with one value.

```bash
curl -s http://127.0.0.1:12111/internal/destinations | jq
# { "object": "list", "data": [
#   { "id": "ed_test_...", "name": "snapshot-app",
#     "event_payload": "snapshot", "status": "enabled",
#     "webhook_endpoint": { "url": "...", "signing_secret": "whsec_..." } } ] }

# Grab one destination's secret by name:
WHSEC=$(curl -s http://127.0.0.1:12111/internal/destinations \
  | jq -r '.data[] | select(.name=="snapshot-app") | .webhook_endpoint.signing_secret')
```

Secrets are keyed by endpoint URL and persisted, so they stay stable across
restarts and destinations sharing a URL always agree. (Real Stripe issues one
secret *per destination*; the mock shares it per URL for convenience.) Use the
secret to verify deliveries with the
SDK, e.g. `stripe.webhooks.constructEvent(rawBody, signatureHeader, whsec)`; see
[`docs/webhooks.md`](docs/webhooks.md) and [`docs/internal.md`](docs/internal.md).

## v2 thin events and event destinations

An **event destination** is the single webhook concept (v1 and v2). Each chooses
one `event_payload`: `snapshot` receives v1 `event` objects, `thin` receives
`v2.core.event` notifications. To receive both you register two destinations —
which is exactly what Stripe requires. Point both at the **same URL** and they
share one `whsec_` secret. v2 Accounts emit **both** families: a v1
snapshot `account.updated` + thin `v1.account.updated` ("Connected accounts"
scope) alongside the v2 thin events ("Your account" scope). Creating an account
through `POST /v2/core/accounts` emits `v2.core.account.created`; the event and
its deliveries commit with the account, then deliver asynchronously.

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | jq -r .secret_key)

# Register a thin webhook destination and keep its signing secret.
DEST=$(curl -s -X POST $BASE/v2/core/event_destinations \
  -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"name":"thin","type":"webhook_endpoint","event_payload":"thin",
       "enabled_events":["v2.core.account.created"],
       "webhook_endpoint":{"url":"http://localhost:4000/thin"},
       "include":["webhook_endpoint.signing_secret"]}')
DEST_ID=$(echo "$DEST" | jq -r .id)
WHSEC=$(echo "$DEST" | jq -r '.webhook_endpoint.signing_secret')

# Create an account through the v2 API -> v2.core.account.created is emitted.
curl -s -X POST $BASE/v2/core/accounts \
  -H "Authorization: Bearer $KEY" -H "Content-Type: application/json" \
  -d '{"configuration":{"merchant":{}}}' | jq -r .id

# Read the thin event back (the full versioned event).
curl -s "$BASE/v2/core/events" -H "Authorization: Bearer $KEY" \
  | jq '.data[0] | {id, type, related_object}'
```

Stripe returns a destination's signing secret only at creation time; the mock
also exposes it at the internal control plane for local verification:

```bash
WHSEC=$(curl -s $BASE/internal/destinations \
  | jq -r '.data[0].webhook_endpoint.signing_secret')
```

The delivered body is the unversioned thin payload (`id`, `object`, `type`,
`created`, `livemode`, `related_object`); the fetched-only envelope
(`data`/`changes`/`reason`/`context`) is returned by
`GET /v2/core/events/{id}`. See [`docs/webhooks.md`](docs/webhooks.md).

An already-created account can emit thin events without being recreated: update
it with `POST /v2/core/accounts/{id}` (e.g. `{"metadata":{"note":"hi"}}`)
to get `v2.core.account.updated`, or complete onboarding on an account link to
get capability/requirements events. See
[`docs/webhooks.md`](docs/webhooks.md).

## Quickstart

```bash
BASE=http://127.0.0.1:12111
KEY=$(curl -s $BASE/internal/main | jq -r .secret_key)

# Create a product and a one-time price.
PRODUCT=$(curl -s -X POST $BASE/v1/products \
  -H "Authorization: Bearer $KEY" -d 'name=Widget')
PID=$(echo "$PRODUCT" | jq -r .id)

PRICE=$(curl -s -X POST $BASE/v1/prices \
  -H "Authorization: Bearer $KEY" \
  -d "product=$PID&unit_amount=1000&currency=usd")
PRICE_ID=$(echo "$PRICE" | jq -r .id)

# Create a checkout session.
curl -s -X POST $BASE/v1/checkout/sessions \
  -H "Authorization: Bearer $KEY" \
  --data-urlencode "mode=payment" \
  --data-urlencode "line_items[0][price]=$PRICE_ID" \
  --data-urlencode "success_url=https://example.com/success?session={CHECKOUT_SESSION_ID}"
```

## Using the official Stripe SDK

The drop-in guarantee is validated with the **official Stripe Node SDK
(`stripe` v23)**. Point it at the mock:

```js
const Stripe = require("stripe");
const stripe = new Stripe(process.env.SECRET_KEY, {
  host: "127.0.0.1",
  port: 12111,
  protocol: "http",
});
```

A complete SDK-driven flow (v2 account + onboarding → product/price →
customer → subscription → checkout → mock payment → verified webhook
deliveries for both snapshot and thin destinations) lives in
[`scripts/sdk-e2e/`](scripts/sdk-e2e/). Run it with:

```bash
cd scripts/sdk-e2e
npm install
./run.sh
```

The script verifies snapshot and connected-scope webhook signatures with
`stripe.webhooks.constructEvent` using each destination's `whsec_` secret, and
creates a v2 event destination and verifies a `v2.core.account.created` thin
event with `stripe.parseEventNotification`, then retrieves it with
`stripe.v2.core.events.retrieve`. It has been run successfully against this
build; the recorded result is `E2E_SDK_OK`.

## Testing

```bash
cargo test          # unit + integration tests
openspec validate build-stripe-mock
openspec validate add-v2-thin-events
openspec validate unify-event-destinations
```

The integration tests boot the server on an ephemeral port with a fresh temp
database and drive it over HTTP, including asynchronous webhook delivery.

## CI and Docker Hub publishing

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on pushes and pull
requests targeting `main`:

1. **Test** - `cargo test --locked`.
2. **Docker** - compiles the release binary inside the image and pushes it to
   Docker Hub. It only runs for pushes (not pull requests) and `needs` the test
   job, so untested code is never published. On `main` the image is tagged
   `latest`, `main`, and `sha-<short>`; pushing a `v*.*.*` tag also publishes
   `X.Y.Z` and `X.Y` tags.

Configure these under **Settings → Secrets and variables → Actions**:

| Name | Kind | Description |
|------|------|-------------|
| `DOCKERHUB_USERNAME` | Variable | Docker Hub account used to log in; must have write access to the `webmobix` organization. |
| `DOCKERHUB_TOKEN` | Secret | Docker Hub access token with *Read & Write* scope. |

The workflow pushes to `webmobix/stripe-mock-server`. Make sure that
repository exists and is **public** on Docker Hub so anyone can pull it (Docker
Hub may create it on first push when the token has permission).

## Documentation

- [`docs/api-foundations.md`](docs/api-foundations.md) — auth, versions, envelopes, ids, timestamps, form parsing.
- [`docs/accounts.md`](docs/accounts.md) — v2 accounts, `include`/default-null, v1 projection, account links, onboarding.
- [`docs/catalog.md`](docs/catalog.md) — products and prices.
- [`docs/billing.md`](docs/billing.md) — customers and subscriptions.
- [`docs/checkout.md`](docs/checkout.md) — checkout sessions and completion.
- [`docs/hosted-ui.md`](docs/hosted-ui.md) — mock onboarding/checkout pages and headless driving.
- [`docs/webhooks.md`](docs/webhooks.md) — events, destinations, signing, ordering, retries.
- [`docs/internal.md`](docs/internal.md) — `/internal/**` control plane.
- [`docs/configuration.md`](docs/configuration.md) — config reference.

## Out of scope (future work)

Non-webhook event destination transports (Amazon EventBridge, Azure Event Grid),
directly usable connected-account keys,
server-side time advance / billing renewals, full v2 onboarding
(`account_tokens` + `persons`), invoices/tax, idempotency keys, concurrent
per-event delivery, and endpoints beyond the implemented subset. See
`openspec/changes/build-stripe-mock/design.md` for the deferred list.

## License

MIT — see [`LICENSE`](LICENSE). Copyright (c) 2026 Webmobix.
