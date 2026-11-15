# Spec Delta

## ADDED Requirements

### Requirement: Metered prices

`POST /v1/prices` SHALL accept `recurring[usage_type]=metered` together with `recurring[meter]` naming an active meter. The price SHALL report `recurring.usage_type` and `recurring.meter`, and its unit amount SHALL be the charge per aggregated unit. A metered price SHALL be recurring and SHALL reference an existing active meter.

#### Scenario: Create a metered price

- **WHEN** a client creates a recurring price with `usage_type=metered` and a meter id
- **THEN** the price reports the meter id and `usage_type=metered`

#### Scenario: Meter must be active

- **WHEN** a client names an unknown or inactive meter
- **THEN** the request is rejected with an `invalid_request_error` naming `recurring[meter]`

### Requirement: Tiered prices

`POST /v1/prices` SHALL accept `billing_scheme=tiered`, `tiers_mode` of `graduated` or `volume`, and ordered `tiers[]` entries with `up_to` (`inf` for the unbounded final tier), `unit_amount` and/or `flat_amount`, and their decimal variants. A tiered price SHALL report `unit_amount=null`, its `tiers`, and `tiers_mode`. The final tier MUST be unbounded and upper bounds MUST increase.

#### Scenario: Create a graduated price

- **WHEN** a client creates a tiered price with tiers up to 10 and `inf`
- **THEN** the price reports `billing_scheme=tiered`, both tiers, and `tiers_mode=graduated`

#### Scenario: Graduated billing

- **WHEN** a subscription bills 25 units against 10 units at 100 and the remainder at 50
- **THEN** the first invoice totals 1750

#### Scenario: Bounded final tier

- **WHEN** the final tier has a numeric `up_to`
- **THEN** the request is rejected with an `invalid_request_error`

### Requirement: No quantity on metered lines

A subscription item or checkout line that names a metered price SHALL reject a supplied `quantity` with an `invalid_request_error`; the resulting subscription item SHALL report `quantity=null`.

#### Scenario: Reject metered quantity

- **WHEN** a client creates a subscription item or checkout line for a metered price with a quantity
- **THEN** the request is rejected and names the quantity parameter
