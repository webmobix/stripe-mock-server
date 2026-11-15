# Proposal

## Why

Real Stripe resolves `expand[]` paths on list endpoints relative to the list envelope, so a lookup-key price list is expanded with `GET /v1/prices?lookup_keys[]=normal_monthly_flat&expand[]=data.product`. The mock currently applies `expand[]` to each item, which makes only item-relative paths such as `expand[]=product` work: `data.product` is silently ignored and prices keep a `prod_` id. Consumers therefore need a fallback that retrieves each product when it is still an id.

## What Changes

- List endpoints that support expansion resolve each `expand[]` path against the list envelope in addition to each item.
- Envelope-relative paths such as `data.product`, `data.price.product`, and `data.items.data.price.product` now expand the matching fields of every returned item.
- Existing item-relative list paths such as `expand[]=product` and `items.data.price.product` keep working, so this is not a breaking change.
- The behavior is applied to `GET /v1/prices` and `GET /v1/subscriptions`; checkout line items already resolve `data.price.product` against their envelope.
- Tests and the catalog/API-foundations documentation are updated to show the list-envelope form.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `api-foundations`: The response-expansion requirement gains list-envelope path semantics and a scenario for expanding a price list with `expand[]=data.product`.
- `catalog`: The price-product-expansion requirement documents the envelope-relative list form `expand[]=data.product` while retaining the item-relative form.

## Impact

- `src/api/v1.rs`: list price and subscription handlers expand the completed list envelope as well as each item.
- `src/api/expand.rs`: optional shared helper for applying list expansion consistently.
- `tests/flows.rs`: coverage for `GET /v1/prices?lookup_keys[]=...&expand[]=data.product` and the existing item-relative paths.
- `docs/catalog.md` and `docs/api-foundations.md`: document that list expansion paths are envelope-relative, with the item-relative form retained for compatibility.
- No database, configuration, or dependency changes.
