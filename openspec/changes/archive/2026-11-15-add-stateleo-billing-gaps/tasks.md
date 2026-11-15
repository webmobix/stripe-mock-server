# Tasks

## 1. Billing meters and usage

- [x] 1.1 Add `billing_meters` and `meter_events` tables plus migration columns in `src/db.rs`, and `MeterRow`/`MeterEventRow` renderers in `src/models.rs`; verify `cargo test db::` and `cargo test models::`.
- [x] 1.2 Add `src/api/meters.rs` with create/list/retrieve/update/deactivate/reactivate, meter-event creation with generated identifiers and duplicate rejection (`duplicate_meter_event`), and unknown-meter rejection (`no_meter`); verify the meter integration test.
- [x] 1.3 Implement `GET /v1/billing/meters/{id}/event_summaries` with `sum`/`count`/`last`, hour/day grouping, stable `mtrusg_` ids, and customer/time validation; verify the summary assertion in `tests/stateleo.rs`.
- [x] 1.4 Register meter routes in `src/lib.rs` and document usage in `docs/meters.md`; verify the docs match the tested request shapes.

## 2. Metered and tiered prices

- [x] 2.1 Add price columns (`billing_scheme`, `tiers`, `tiers_mode`, `usage_type`, `meter_id`) and expose them from `PriceRow::to_json`; verify an existing per-unit price still renders.
- [x] 2.2 Parse `recurring.usage_type`/`recurring.meter` in `create_price`, require an active meter for metered prices, and reject a metered one-time price; verify with the metered-price test.
- [x] 2.3 Parse graduated/volume `tiers[]` with `up_to=inf`, decimal amounts, increasing bounds, and a required unbounded final tier; verify the tiered-price test.
- [x] 2.4 Add shared `amount_for_quantity`, `advance_period`, and meter aggregation helpers in `src/api/billing.rs` with unit tests for per-unit, graduated, volume, and flat tiers.
- [x] 2.5 Reject a supplied quantity on metered subscription items, checkout lines, subscription updates, and preview overrides; store `quantity: null` on metered items; verify all rejection paths.

## 3. Embedded checkout

- [x] 3.1 Add `ui_mode`, `return_url`, `client_secret`, and `subscription_metadata` checkout columns and render them, keeping hosted sessions unchanged.
- [x] 3.2 Accept `ui_mode=embedded_page`/`embedded`/`elements`, require `return_url` for non-hosted sessions, generate a client secret, and omit `url`; verify the embedded session response.
- [x] 3.3 Parse `subscription_data[metadata]` and apply it to the subscription created at completion; verify `metadata.plan` after completion.
- [x] 3.4 Redirect embedded completions to `return_url` with `{CHECKOUT_SESSION_ID}` substitution and keep `success_url` for hosted sessions; verify the 303 target.

## 4. Configurable keys and signing secrets

- [x] 4.1 Add `[main_account] secret_key`/`publishable_key` to `src/config.rs` and the sample config, with validation and docs.
- [x] 4.2 Make `ensure_keys` replace generated keys with configured values and reapply them through the reset seeding path; verify configured keys before and after `/internal/reset`.
- [x] 4.3 Add `webhook_endpoint.signing_secret`, prefer it per URL in seeding and normalization, and apply it to API-created destinations for the same URL; verify the configured `whsec_` survives reset.

## 5. Billing periods, first and renewal invoices

- [x] 5.1 Add subscription invoice columns (`subscription`, `billing_reason`, period bounds, `collection_method`, `attempt_count`, `payment_intent`) and render them.
- [x] 5.2 Raise a paid `subscription_create` invoice, with its `invoice.created`, `invoice.payment_succeeded`, and `invoice.paid` events, on direct subscription creation and checkout completion.
- [x] 5.3 Implement `POST /internal/subscriptions/{id}/advance` with `paid`/`failed`, period advancement, licensed-forward and metered-arrears lines, and `past_due`/`active` transitions.
- [x] 5.4 Retry an existing open invoice on a subsequent `advance` instead of raising another, and reactivate a `past_due` subscription when its invoice is paid; verify the failure/recovery test and invoice count.
- [x] 5.5 Support `GET /v1/invoices?subscription=` and document the control-plane flow in `docs/billing.md` and `docs/internal.md`.

## 6. Upcoming invoice preview

- [x] 6.1 Add `POST /v1/invoices/create_preview` and `GET /v1/invoices/upcoming`, returning a non-persistent draft `upcoming_in_` invoice with `billing_reason=upcoming`.
- [x] 6.2 Price licensed items for the next period and metered items for the current period, infer a customer from the subscription, and support `subscription_details[items]`; verify the preview test.
- [x] 6.3 Document the endpoint in `docs/invoicing.md` and note it does not appear in lists or emit events.

## 7. Default card after checkout

- [x] 7.1 Add the `payment_methods` table, `PaymentMethodRow` renderer, and card payload helper, plus the customer `invoice_settings` column.
- [x] 7.2 Attach a simulated Visa `pm_` card on checkout completion and set `invoice_settings.default_payment_method`, reusing an existing card for a customer.
- [x] 7.3 Add payment-method retrieve/list endpoints and expand `invoice_settings.default_payment_method`; verify the expanded card in the integration test and SDK e2e.

## 8. Idempotency keys

- [x] 8.1 Add the `idempotency_keys` table and a v1 middleware in `src/idempotency.rs` keyed by key, credential, method, path, and body fingerprint.
- [x] 8.2 Replay cached status/body/content type with `Idempotent-Replay: true`, reject a different body with `idempotency_error`, skip caching 5xx responses, and clear keys on reset.
- [x] 8.3 Verify replay and conflict in `tests/stateleo.rs` and through the Stripe Node SDK `idempotencyKey` option; document the behavior in `docs/api-foundations.md`.

## 9. Validation

- [x] 9.1 Run `cargo test --locked` and confirm the full unit and integration suite passes.
- [x] 9.2 Run `openspec validate --all --strict` and confirm the change and specs have no errors or warnings.
- [x] 9.3 Run `scripts/sdk-e2e/run.sh` with Stripe Node SDK 23 and confirm it ends with `E2E_SDK_OK`, including the new meter, preview, embedded checkout, card, and idempotency checks.
