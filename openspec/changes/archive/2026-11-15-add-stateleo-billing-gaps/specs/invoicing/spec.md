# Spec Delta

## ADDED Requirements

### Requirement: Subscription invoice fields

Subscription invoices SHALL report `subscription`, `billing_reason` (`subscription_create` or `subscription_cycle`), `period_start`, `period_end`, `collection_method=charge_automatically`, and `attempt_count`. `GET /v1/invoices` SHALL filter by `subscription`.

#### Scenario: Filter by subscription

- **WHEN** a client lists invoices with `subscription=<sub_...>`
- **THEN** only that subscription's invoices are returned

#### Scenario: Paid renewal fields

- **WHEN** a renewal invoice is paid
- **THEN** it reports `billing_reason=subscription_cycle` and `period_start`/`period_end` for the new period

### Requirement: Preview the upcoming invoice

`POST /v1/invoices/create_preview` and `GET /v1/invoices/upcoming` SHALL price the next invoice without persisting it or emitting events. The preview SHALL be a draft `upcoming_in_` invoice with `billing_reason=upcoming`, period bounds, and computed lines. A customer SHALL be required explicitly or through the subscription. Licensed items SHALL be priced for the next period; metered items SHALL report current-period usage. `subscription_details[items]` SHALL preview a potential subscription.

#### Scenario: Preview an existing subscription

- **WHEN** a client previews the upcoming invoice for a subscription
- **THEN** the response is a draft `upcoming_in_` invoice with `billing_reason=upcoming`

#### Scenario: Preview is not stored

- **WHEN** a client previews an invoice
- **THEN** listing invoices does not include the preview and no event is created

#### Scenario: Preview a potential subscription

- **WHEN** a client supplies `subscription_details[items]` with a price and quantity
- **THEN** the preview lines reflect those items
