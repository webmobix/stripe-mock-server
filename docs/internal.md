# Internal control plane

`/internal/**` is a local-only tool surface for tests. It requires **no Stripe
authentication** and binds only to the configured `server.bind` address. All
generated material that the real API does not return is exposed here.

## Main account and secrets

```bash
BASE=http://127.0.0.1:12111

# The seeded platform account with its keys.
curl -s $BASE/internal/main
# {"id":"acct_...","is_main":true,"secret_key":"sk_test_...","publishable_key":"pk_test_...", ...}

# All accounts with keys.
curl -s $BASE/internal/accounts

# One account's keys.
curl -s $BASE/internal/accounts/acct_.../secrets

# Event destinations, including whsec_ signing secrets and configured URLs.
# The secret is shared by every destination with the same URL (one per endpoint URL).
curl -s $BASE/internal/destinations
```

## Seed a connected account

`POST /internal/accounts` seeds a connected (`is_main = false`) account directly,
without Stripe authentication or the create/onboarding flow, and returns its
`acct_` id and freshly generated keys. This is a test fixture: no events are
emitted, so tests that need `account.updated` must use the real create flow.

The body accepts `name` (default `"Mock Business"`), optional `email`, optional
`country` (default `"US"`), and `state` (`active` or `pending`, default
`pending`). Each call creates a new account.

```bash
# Seed an active, fully onboarded account.
curl -s -X POST $BASE/internal/accounts \
  -H "Content-Type: application/json" \
  -d '{"name":"Acme","email":"owner@acme.test","state":"active"}'
# {"id":"acct_...","object":"account","is_main":false,
#  "details_submitted":true,"charges_enabled":true,"payouts_enabled":true,
#  "secret_key":"sk_test_...","publishable_key":"pk_test_..."}

# Seed a pending account awaiting onboarding (state defaults to pending).
curl -s -X POST $BASE/internal/accounts \
  -H "Content-Type: application/json" \
  -d '{"name":"Acme","state":"pending"}'
# {"id":"acct_...","object":"account","is_main":false,
#  "details_submitted":false,"charges_enabled":false,"payouts_enabled":false,
#  "secret_key":"sk_test_...","publishable_key":"pk_test_..."}
```

An unknown `state` fails with an `invalid_request_error` (`param: "state"`) and
creates no account. Seeded keys are usable immediately: `/internal/accounts/{id}/secrets`
returns the same values, `GET /v1/accounts/{id}` reflects the seeded state, and the
secret key authenticates Stripe API requests.

## Delivery inspection

```bash
# Full journal.
curl -s $BASE/internal/deliveries

# Block until a given event's deliveries reach a terminal state.
curl -s -X POST $BASE/internal/deliveries/wait \
  -H "Content-Type: application/json" \
  -d '{"event_id":"evt_...","timeout_ms":5000}'
# => {"event":"evt_...","settled":true,"deliveries":[...]}
# On timeout: {"settled":false,"deliveries":[]}

# Block until all pending deliveries settle.
curl -s -X POST $BASE/internal/deliveries/flush \
  -H "Content-Type: application/json" -d '{"timeout_ms":5000}'

# Re-queue a terminal delivery.
curl -s -X POST $BASE/internal/deliveries/dl_.../retry
```

## Reset

```bash
curl -s -X POST $BASE/internal/reset
# {"status":"ok"}
```

Clears all objects, events, and deliveries and reapplies configuration seeding.
After a reset only the reseeded main account (with fresh keys) remains; call
`/internal/main` again to read them.
