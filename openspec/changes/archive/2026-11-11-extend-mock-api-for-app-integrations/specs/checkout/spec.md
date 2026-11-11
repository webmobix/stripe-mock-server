# Spec Delta

## ADDED Requirements

### Requirement: Checkout customer email

`POST /v1/checkout/sessions` SHALL accept `customer_email`. When the request carries no `customer` and no `customer_account`, the server SHALL immediately create a `cus_` customer with that email, link it to the session, and create a `customer.created` event. When a customer is already supplied, the server SHALL NOT create another one.

#### Scenario: Create a session with only an email

- **WHEN** a client creates a session with `customer_email` and no `customer`
- **THEN** the session records a new `cus_` customer and reports `customer_email`

#### Scenario: Customer created event

- **WHEN** a checkout session creates a customer from `customer_email`
- **THEN** a `customer.created` event exists whose `data.object` is that customer

#### Scenario: Existing customer is kept

- **WHEN** a client creates a session with both `customer` and `customer_email`
- **THEN** the session keeps the supplied customer and creates no additional customer

#### Scenario: Retrieved session exposes the email

- **WHEN** a client retrieves a session created with `customer_email`
- **THEN** the session reports `customer_email` and `customer_details.email`

### Requirement: Checkout discounts

A checkout session SHALL accept repeated `discounts[][coupon]` values naming coupons. The server SHALL validate that each coupon exists, is valid, matches the session currency when it carries a fixed amount, and applies to the session's products. The discount SHALL reduce the subtotal, never below zero, before tax is computed.

#### Scenario: Percentage discount

- **WHEN** a session with a subtotal of 2000 applies a 25 percent coupon
- **THEN** the session reports an `amount_discount` of 500 and an `amount_total` reduced accordingly

#### Scenario: Fixed-amount discount

- **WHEN** a session in `usd` applies a 500-cent coupon
- **THEN** the session reports an `amount_discount` of 500

#### Scenario: Discount larger than the subtotal

- **WHEN** the applied discount exceeds the subtotal
- **THEN** `amount_discount` equals the subtotal and the discounted amount is zero, not negative

#### Scenario: Unknown coupon

- **WHEN** a session names a coupon id that does not exist for the account
- **THEN** the server responds with an `invalid_request_error` and creates no session

#### Scenario: Coupon in another currency

- **WHEN** a fixed-amount coupon's currency differs from the session currency
- **THEN** the server responds with an `invalid_request_error` and creates no session

#### Scenario: Discount breakdown

- **WHEN** a session applies a coupon
- **THEN** `total_details.breakdown.discounts` contains one entry with the discount amount and coupon id

### Requirement: Checkout tax rates

A checkout session SHALL accept `tax_rates[]` applying to every line, and a line MAY carry its own `line_items[][tax_rates][]`, which takes precedence. Each referenced tax rate SHALL exist for the account and be active. Tax SHALL be computed on the discounted line amount.

#### Scenario: Exclusive tax

- **WHEN** a session with a discounted amount of 1000 applies an exclusive 20 percent tax rate
- **THEN** `amount_tax` is 200 and `amount_total` is 1200

#### Scenario: Inclusive tax

- **WHEN** a session applies an inclusive 20 percent tax rate
- **THEN** `amount_tax` reports the included tax and `amount_total` is not increased by it

#### Scenario: Line-level tax rate overrides the session rate

- **WHEN** a session has a session-level tax rate and a line names its own tax rate
- **THEN** that line is taxed with its own rate

#### Scenario: Unknown tax rate

- **WHEN** a session references a tax rate id that does not exist for the account
- **THEN** the server responds with an `invalid_request_error` and creates no session

#### Scenario: Inactive tax rate

- **WHEN** a session references an inactive tax rate
- **THEN** the server responds with an `invalid_request_error` and creates no session

#### Scenario: Tax breakdown

- **WHEN** a session is taxed
- **THEN** `total_details.breakdown.taxes` contains an entry with the tax amount, the tax rate id, and the taxable amount

### Requirement: Checkout totals

Checkout sessions SHALL report `amount_subtotal` as the sum of line amounts before discount and tax, `amount_total` as the discounted subtotal plus exclusive tax, and `total_details` with `amount_discount`, `amount_tax`, and the discount and tax breakdowns. A session without discounts or tax SHALL report zero discount and tax and an `amount_total` equal to `amount_subtotal`.

#### Scenario: Discounted and taxed total

- **WHEN** a session with a subtotal of 2000 applies a 25 percent coupon and an exclusive 10 percent tax rate
- **THEN** `amount_subtotal` is 2000, `amount_discount` is 500, `amount_tax` is 150, and `amount_total` is 1650

#### Scenario: No discount or tax

- **WHEN** a session is created without discounts or tax rates
- **THEN** `amount_subtotal` equals `amount_total` and both discount and tax are zero

#### Scenario: Totals survive retrieval

- **WHEN** a client retrieves a completed session
- **THEN** its subtotal, discount, tax, and total are unchanged

#### Scenario: Payment uses the discounted total

- **WHEN** a `payment`-mode session with a discount completes
- **THEN** its payment intent amount equals the session's `amount_total`
