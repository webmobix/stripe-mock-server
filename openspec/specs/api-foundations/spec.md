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

The server SHALL generate object identifiers with the Stripe prefix for their type (`acct_`, `prod_`, `price_`, `cus_`, `sub_`, `cs_test_`, `pi_`, `evt_`, `sk_test_`, `pk_test_`, `whsec_`) and SHALL persist generated key material so it is stable across restarts.

#### Scenario: Stable generated keys

- **WHEN** the server restarts with an existing database
- **THEN** previously generated API keys and webhook signing secrets are unchanged

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
