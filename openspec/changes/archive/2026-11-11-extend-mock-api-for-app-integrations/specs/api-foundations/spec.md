# Spec Delta

## ADDED Requirements

### Requirement: Response expansion

The server SHALL accept repeated `expand[]` parameters on endpoints that support expansion: query parameters on retrieval and list requests, and form parameters on create and update requests. A path SHALL be dot-separated, SHALL traverse every element of an embedded list, and SHALL replace a reference id with the full object it names.

#### Scenario: Expand a price's product

- **WHEN** a client retrieves a price with `expand[]=product`
- **THEN** the price's `product` is the full product object instead of its `prod_` id

#### Scenario: Expand a nested subscription price product

- **WHEN** a client retrieves a subscription with `expand[]=items.data.price.product`
- **THEN** every subscription item's `price.product` is the full product object

#### Scenario: Expand checkout line item products

- **WHEN** a client lists a checkout session's line items with `expand[]=data.price.product`
- **THEN** every line item's `price.product` is the full product object

#### Scenario: Several expansions at once

- **WHEN** a client sends two `expand[]` parameters naming different paths
- **THEN** both paths are expanded in the response

#### Scenario: Unsupported expansion path

- **WHEN** a client requests an expansion path the server cannot resolve
- **THEN** the response is returned unchanged for that path and no error is raised

#### Scenario: Expansion is account-scoped

- **WHEN** an expanded reference belongs to another account and is therefore invisible
- **THEN** the field stays as its original id rather than exposing the other account's object
