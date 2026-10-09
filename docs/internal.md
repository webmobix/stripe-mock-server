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
