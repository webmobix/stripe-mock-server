# api-foundations Specification

## Purpose

Provides the Stripe protocol primitives shared by every implemented endpoint: request authentication, account-context resolution, version-specific serialization, and the v1/v2 error and list envelopes.

## Requirements

### Requirement: API key authentication

The server SHALL authenticate requests presenting an API key via `Authorization: Bearer <key>`, `Authorization: Basic <key>`, or `Authorization: Basic <base64(key + ":")>`, and SHALL reject requests with a missing or unknown key using the version-appropriate error envelope.

#### Scenario: Valid secret key

- **WHEN** a request includes `Authorization: Bearer sk_test_...` for a known account
- **THEN** the server processes the request as that account

#### Scenario: Invalid key

- **WHEN** a request omits the Authorization header or presents an unknown key
- **THEN** the server responds with an authentication error in the appropriate envelope

### Requirement: Account context resolution

The server SHALL resolve each request to a target account: the account owning the API key by default, or the account named by the `Stripe-Account` header when present and authorized for the key's account.

#### Scenario: Platform key with Stripe-Account

- **WHEN** a request uses the main account key and sets `Stripe-Account` to a connected account
- **THEN** the request acts on that connected account and resulting objects carry its account id

#### Scenario: Unknown connected account

- **WHEN** `Stripe-Account` names an account not connected to the key's account
- **THEN** the server responds with an error and does not mutate state

### Requirement: v1 error envelope

The server SHALL return v1 API errors as `{"error":{"type":...,"message":...}}` with an HTTP status matching the error class, including `code` and `param` where applicable.

#### Scenario: Invalid parameter on v1 request

- **WHEN** a v1 request omits a required parameter
- **THEN** the response has an error `type` of `invalid_request_error` and a `param` naming the field

### Requirement: v2 error envelope

The server SHALL return v2 API errors as `{"error":{"type":...,"code":...,"message":...}}` with the same status-code semantics as v1.

#### Scenario: Invalid parameter on v2 request

- **WHEN** a v2 request supplies an invalid field value
- **THEN** the response populates `error.code` and `error.message`

### Requirement: List envelope

The server SHALL return list endpoints as `{"object":"list","data":[...],"has_more":<bool>,"url":"..."}`, supporting `limit`, `starting_after`, and `ending_before` where the endpoint declares them, ordered newest first.

#### Scenario: Paginated list

- **WHEN** a client lists resources with `limit=2` and more than two exist
- **THEN** the response contains two items, `has_more` is true, and items are ordered newest first

### Requirement: Prefixed identifiers and keys

The server SHALL generate object identifiers with the Stripe prefix for their type (`acct_`, `prod_`, `price_`, `cus_`, `sub_`, `cs_test_`, `pi_`, `in_`, `txi_`, `evt_`, `sk_test_`, `pk_test_`, `whsec_`) and SHALL persist generated key material so it is stable across restarts.

#### Scenario: Stable generated keys

- **WHEN** the server restarts with an existing database
- **THEN** previously generated API keys and webhook signing secrets are unchanged

#### Scenario: Prefix per object type

- **WHEN** the server creates an invoice and a customer tax id
- **THEN** the invoice id starts with `in_` and the tax id starts with `txi_`

### Requirement: Version-specific time serialization

The server SHALL serialize v1 timestamps as Unix epoch seconds and v2 timestamps as RFC 3339 UTC strings with millisecond precision.

#### Scenario: Object timestamps by version

- **WHEN** the same account is retrieved through `/v2/core/accounts/{id}` and `/v1/accounts/{id}`
- **THEN** the v2 `created` value is an RFC 3339 string and the v1 `created` value is an integer

### Requirement: Request body encoding support

The server SHALL accept form-encoded bodies for `/v1/**` endpoints and JSON bodies for `/v2/**` endpoints.

#### Scenario: Form-encoded v1 request

- **WHEN** a client POSTs `amount=1000&currency=usd` as `application/x-www-form-urlencoded`
- **THEN** the server parses the parameters as a flat key/value map

#### Scenario: JSON v2 request

- **WHEN** a client POSTs a JSON object to a `/v2/**` endpoint
- **THEN** the server parses the nested JSON structure

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
