# Spec Delta

## MODIFIED Requirements

### Requirement: Prefixed identifiers and keys

The server SHALL generate object identifiers with the Stripe prefix for their type (`acct_`, `prod_`, `price_`, `cus_`, `sub_`, `cs_test_`, `pi_`, `in_`, `txi_`, `evt_`, `sk_test_`, `pk_test_`, `whsec_`) and SHALL persist generated key material so it is stable across restarts.

#### Scenario: Stable generated keys

- **WHEN** the server restarts with an existing database
- **THEN** previously generated API keys and webhook signing secrets are unchanged

#### Scenario: Prefix per object type

- **WHEN** the server creates an invoice and a customer tax id
- **THEN** the invoice id starts with `in_` and the tax id starts with `txi_`
