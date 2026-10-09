# Design

## D1. One config concept: the event destination

`[[webhook_targets]]` is deleted outright. All receivers are
`[[event_destinations]]`, mirroring the `v2.core.event_destination` object.
There is no compatibility layer and no alias; the old table (`webhook_targets`),
ids (`wht_`), and `scope`/`accounts` fields are removed. `deliveries` now
references `destination_id`.

Rationale: Stripe's dashboard exposes a single "Event destinations" surface.
Adding a parallel v1-only concept makes the mock harder to reason about than the
real thing and caused the original support case (a v1 target never receives thin
events).

## D2. `event_payload` is exclusive and required

A destination is either `snapshot` (v1 `event` objects) or `thin`
(`v2.core.event` notifications), never both. This is faithful to Stripe: the
delivered payload shape is a property of the destination, fixed at creation.
A user who wants both declares two destinations, exactly as Stripe requires a
separate endpoint per format.

Matching therefore filters on `destination.event_payload == event_payload`
before scope and event-type checks.

## D3. `events_from` replaces `scope`

| Stripe Workbench | API `events_from` | Mock behavior |
|------------------|-------------------|---------------|
| Your account | `["@self"]` (default) | destination receives events owned by its account |
| Connected accounts | `["@accounts"]` | destination receives events emitted for another (connected) account |

Scope is computed from the event's context account (`events.account_id`): if it
equals the destination's account the event is "self"; otherwise, with a
connected `event_account`, it is "connected". The default is `["@self"]`, like
Stripe.

## D4. v2 Accounts emit both families, with Stripe's scope split

Stripe: *"a v2 Account update can send a v1 `account.updated`, a v2 thin
`v1.account.updated`, and a v2 `v2.core.account.updated`."* And *"v2 events use
the Your account scope, while v1 events use the Connected accounts scope, even
for the same v2 Account."*

The mock reproduces this:

- `POST /v2/core/accounts/{id}` emits snapshot `account.updated` and thin
  `v1.account.updated` (connected scope), plus `v2.core.account.updated` and the
  capability/configuration thin events (self scope).
- Onboarding emits snapshot `account.updated`, thin `v1.account.updated`, and the
  v2 capability/requirements/updated events.
- `POST /v2/core/accounts` keeps emitting `v2.core.account.created`.

A thin destination scoped only to `@accounts` therefore misses
`v2.core.account.*`; the docs say so, so users do not get a false sense of
completeness.

## D5. Config shape

```toml
[[event_destinations]]
name = "snapshot-app"
event_payload = "snapshot"            # required: snapshot | thin
events_from = ["@self", "@accounts"]  # default ["@self"]
enabled_events = ["*"]                # default ["*"]
enabled = true                        # -> status enabled | disabled
webhook_endpoint = { url = "http://localhost:4000/stripe/webhook" }
```

`url` is nested under `webhook_endpoint`, matching the API object rather than a
bespoke top-level field. `Config::from_str` validates `type`, `event_payload`,
`webhook_endpoint.url`, and `events_from`, so a bad config fails at boot with a
clear message instead of silently seeding a broken destination.

## D6. Runtime API parity

`POST /v2/core/event_destinations` accepts both payloads. Only
`type = "webhook_endpoint"` is supported (EventBridge/Azure are a future
change). Config-declared destinations and API-created destinations share one
table, one dispatch path, and one internal representation.

## D7. Rendering is driven by the event version

`process_delivery` resolves the destination and renders `event.to_v2_thin()` for
v2 rows and `event.to_json()` for v1 rows. Matching guarantees the destination's
payload format agrees with the event version, so this is a safe invariant rather
than an additional decision point.
