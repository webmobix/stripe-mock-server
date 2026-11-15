# Design

## Context

See `proposal.md` for motivation. Expansion lives in `src/api/expand.rs`: `expand_response` walks a dot-separated path over a JSON value and replaces reference ids with loaded objects. Retrieval and create handlers pass the resource object. The list price and subscription handlers currently pass each item, then build the v1 list envelope; checkout line items already pass the whole envelope. Real Stripe resolves list `expand[]` paths against the list envelope, which is why `data.product` works there.

## Goals / Non-Goals

**Goals:**

- Make envelope-relative expansion paths work on list endpoints that already accept `expand[]`.
- Keep item-relative list paths working so existing consumers and tests are unaffected.
- Centralize the list behavior so price and subscription lists cannot drift apart.

**Non-Goals:**

- Adding `expand[]` support to list endpoints that do not accept it today (products, customers, coupons, tax rates, events, invoices, payment intents).
- Making unsupported expansion paths return an error; the mock remains lenient and leaves them untouched.
- Removing item-relative expansion or changing retrieval/create semantics.

## Decisions

### Expand the list envelope and each item

Add `expand_list_response` to `src/api/expand.rs`. It first runs `expand_response` on the complete list envelope, then runs it on each element of `data` using `std::mem::take` so no clone is needed. The envelope pass makes `data.product`, `data.price.product`, and `data.items.data.price.product` work; the per-item pass preserves `product` and `items.data.price.product` on subscription lists.

Alternative considered: switch list handlers to envelope-only expansion. This matches Stripe's documented semantics but breaks the existing `expand[]=product` list form that the mock's own spec and tests require, so it is rejected.

Alternative considered: duplicate the two-pass logic inside each list handler. A helper is preferred because there are two handlers and the ordering between envelope and item expansion is easy to get wrong.

### Apply the helper to price and subscription lists

`list_prices` and `list_subscriptions` build their `data` items, wrap them with `list_envelope`, and hand the result to `expand_list_response`. The existing per-item expansion loops are removed. Checkout line items already pass the envelope to `expand_response` and do not need to change.

### Test both path forms

Extend `tests/flows.rs` so the price-expansion flow keeps asserting `expand[]=product` on a list and also asserts `expand[]=data.product` alongside the `lookup_keys[]` filter from the report. Add a subscription-list assertion for `expand[]=data.items.data.price.product` so the second handler is covered.

## Risks / Trade-offs

- Applying the same paths twice can repeat work, but the second pass sees already-expanded objects and performs no database lookups. The cost is negligible for a mock.
- An unsupported path that happens to match a field at both levels could be expanded by both passes. No such field exists in the current object shapes, and both passes use the same lenient "leave untouched" rule.
- Keeping item-relative paths means the mock accepts forms real Stripe may reject. This is intentional leniency and preserves backward compatibility.

## Migration Plan

None. The response shape only gains expansions that previously stayed as ids.
