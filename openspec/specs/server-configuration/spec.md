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

On first boot the server SHALL create the main account from configuration and SHALL generate and persist its secret and publishable keys.

#### Scenario: Keys persisted

- **WHEN** the server restarts with the same database
- **THEN** the main account's keys are unchanged

### Requirement: Event destination configuration

The configuration SHALL model webhooks solely as event destinations (`[[event_destinations]]`), each declaring a `webhook_endpoint.url`, an `event_payload` of `snapshot` or `thin`, an `events_from` list of `@self`/`@accounts`, an `enabled_events` allow-list or `*`, and an `enabled` flag, and the server SHALL generate a signing secret **per endpoint URL** (shared by all destinations with the same `webhook_endpoint.url`). It SHALL NOT accept a separate `[[webhook_targets]]` section.

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
