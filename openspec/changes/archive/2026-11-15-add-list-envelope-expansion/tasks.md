# Tasks

## 1. Add list expansion helper

- [x] 1.1 Add `expand_list_response` to `src/api/expand.rs`, expanding the full list envelope first and then each `data` item with `std::mem::take`, and verify unit tests cover `data.product`, plain `product`, and `data.items.data.price.product` paths with `cargo test api::expand`.

## 2. Apply envelope expansion to list endpoints

- [x] 2.1 Replace the per-item expansion loop in `list_prices` with `expand_list_response` over the completed list envelope, and extend `tests/flows.rs` to request `GET /v1/prices?lookup_keys[]=...&expand[]=data.product` while keeping the existing `expand[]=product` list assertion; verify `cargo test expansion_embeds_referenced_products` passes.
- [x] 2.2 Replace the per-item expansion loop in `list_subscriptions` with `expand_list_response`, and add a list assertion using `expand[]=data.items.data.price.product` (plus the existing item-relative path); verify the subscription flow test passes.

## 3. Update documentation

- [x] 3.1 Update `docs/catalog.md` product-expansion examples so the list form uses `expand[]=data.product` and note that the item-relative form remains accepted; verify the example matches the request shape exercised in `tests/flows.rs`.
- [x] 3.2 Update `docs/api-foundations.md` response-expansion section to state that list paths are resolved against the list envelope, with `data.product` as the example; verify the section also documents the retained item-relative form.

## 4. Validation

- [x] 4.1 Run `cargo test --locked` and confirm the full suite passes.
- [x] 4.2 Run `openspec validate --all --strict` and confirm the change and specs have no errors or warnings.
