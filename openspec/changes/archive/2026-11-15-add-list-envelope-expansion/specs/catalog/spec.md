# Spec Delta

## MODIFIED Requirements

### Requirement: Price product expansion

Price responses SHALL support `expand[]=product`, replacing the product id with the full product object. Expansion SHALL apply to creation, retrieval, listing, and prices embedded in subscription and checkout responses. On price list responses, envelope-relative `expand[]=data.product` SHALL expand every returned price, and item-relative `expand[]=product` SHALL remain supported.

#### Scenario: Expanded product on a retrieved price

- **WHEN** a client retrieves a price with `expand[]=product`
- **THEN** the price's `product` is the full product object

#### Scenario: Expanded product in a price list

- **WHEN** a client lists prices with `expand[]=product`
- **THEN** every returned price carries its full product object

#### Scenario: Expanded product in a price list through the envelope

- **WHEN** a client lists prices with `lookup_keys[]=normal_monthly_flat` and `expand[]=data.product`
- **THEN** every returned price carries its full product object

#### Scenario: Unknown product reference

- **WHEN** a price's product cannot be read for the account during expansion
- **THEN** the price's `product` remains the original product id
