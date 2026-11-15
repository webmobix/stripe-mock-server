# server-configuration Specification

## Purpose

Defines the TOML configuration file describing the server's main account, database, public URL, and event destinations, and the boot-time seeding derived from it.

## Requirements

### Requirement: Configuration file

The server SHALL load a TOML configuration specifying the server bind address and port, public base URL, database path, and the main account identity.

#### Scenario: Startup from config

- **WHEN** the server starts with a config file
- **THEN** it binds the configured address and uses the configured database path and public base URL

### Requirement: Main account seeding

On first boot the server SHALL create the main account from configuration and SHALL generate and persist its secret and publishable keys. When `[main_account]` supplies `secret_key` or `publishable_key`, the server SHALL use the configured value and re-apply it after every reset.

#### Scenario: Keys persisted

- **WHEN** the server restarts with the same database
- **THEN** the main account's keys are unchanged

#### Scenario: Configured keys survive reset

- **WHEN** the configuration fixes `secret_key` and `publishable_key` and a client calls `/internal/reset`
- **THEN** the main account again reports the configured values

### Requirement: Event destination configuration

The configuration SHALL model webhooks solely as event destinations (`[[event_destinations]]`) declaring a URL, an `event_payload` of `snapshot` or `thin`, an `events_from` list, an `enabled_events` allow-list, and an `enabled` flag. It SHALL NOT accept `[[webhook_targets]]`. The server SHALL generate one signing secret per endpoint URL, shared by destinations with that URL. `webhook_endpoint.signing_secret` MAY fix that value; a configured secret SHALL win and SHALL be re-applied after reset.

#### Scenario: Seed destinations at boot

- **WHEN** the configuration lists an event destination
- **THEN** the destination exists with a `whsec_` signing secret retrievable through the internal control plane (shared with any other destination on the same URL)

#### Scenario: Snapshot and thin destinations coexist

- **WHEN** the configuration declares a `snapshot` and a `thin` destination
- **THEN** both are seeded and receive their respective event families

### Requirement: Reset-on-start option

The configuration SHALL support resetting state at startup.

#### Scenario: Reset on boot

- **WHEN** reset-on-start is enabled and the server starts
- **THEN** the database is cleared and reseeded before serving requests
