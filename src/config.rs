use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub main_account: MainAccountConfig,
    /// Stripe-style event destinations. This is the single webhook concept:
    /// each destination chooses a payload format (`snapshot` or `thin`), the
    /// accounts it listens to (`events_from`), the event types it receives
    /// (`enabled_events`), and a transport (`webhook_endpoint.url`).
    #[serde(default)]
    pub event_destinations: Vec<EventDestinationConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    pub port: u16,
    pub public_base_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
    #[serde(default)]
    pub reset_on_start: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MainAccountConfig {
    #[serde(default)]
    pub business_type: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

/// A declarative `v2.core.event_destination`, seeded at boot.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventDestinationConfig {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Delivery transport. Only `webhook_endpoint` is supported.
    #[serde(default = "default_destination_type", rename = "type")]
    pub destination_type: String,
    /// `snapshot` (v1 events) or `thin` (v2 events). Required, mirroring Stripe.
    pub event_payload: String,
    /// Accounts that supply events: `@self` (this account) or `@accounts`
    /// (accounts you manage). Defaults to `["@self"]`.
    #[serde(default = "default_events_from")]
    pub events_from: Vec<String>,
    #[serde(default = "default_events")]
    pub enabled_events: Vec<String>,
    /// Maps to Stripe's `status`. `true` -> `enabled`, `false` -> `disabled`.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub webhook_endpoint: Option<WebhookEndpointConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebhookEndpointConfig {
    pub url: String,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_db_path() -> String {
    "stripe-mock.db".to_string()
}

fn default_events() -> Vec<String> {
    vec!["*".to_string()]
}

fn default_events_from() -> Vec<String> {
    vec!["@self".to_string()]
}

fn default_true() -> bool {
    true
}

fn default_destination_type() -> String {
    "webhook_endpoint".to_string()
}

impl Config {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let text =
            std::fs::read_to_string(path.as_ref()).map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::from_str(&text)
    }

    pub fn from_str(text: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        config.validate().map_err(ConfigError::Parse)?;
        Ok(config)
    }

    pub fn base_url(&self) -> String {
        self.server
            .public_base_url
            .trim_end_matches('/')
            .to_string()
    }

    fn validate(&self) -> Result<(), String> {
        for destination in &self.event_destinations {
            if destination.destination_type != "webhook_endpoint" {
                return Err(format!(
                    "event destination '{}': only type = \"webhook_endpoint\" is supported",
                    destination.name
                ));
            }
            match destination.event_payload.as_str() {
                "snapshot" | "thin" => {}
                other => {
                    return Err(format!(
                        "event destination '{}': event_payload must be \"snapshot\" or \"thin\", got \"{other}\"",
                        destination.name
                    ));
                }
            }
            match &destination.webhook_endpoint {
                Some(endpoint) if !endpoint.url.is_empty() => {}
                _ => {
                    return Err(format!(
                        "event destination '{}': webhook_endpoint.url is required",
                        destination.name
                    ));
                }
            }
            for from in &destination.events_from {
                if from != "@self" && from != "@accounts" {
                    return Err(format!(
                        "event destination '{}': events_from entries must be \"@self\" or \"@accounts\", got \"{from}\"",
                        destination.name
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(m) => write!(f, "could not read config: {m}"),
            ConfigError::Parse(m) => write!(f, "could not parse config: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../config.toml");

    #[test]
    fn parses_sample_config() {
        let config = Config::from_str(SAMPLE).expect("sample config should parse");
        assert_eq!(config.server.bind, "127.0.0.1");
        assert!(config.server.port > 0);
        assert_eq!(config.database.path, "stripe-mock.db");
    }

    #[test]
    fn defaults_are_applied() {
        let text = r#"
[server]
port = 9999
public_base_url = "http://localhost:9999"
[database]
path = "x.db"
[main_account]
"#;
        let config = Config::from_str(text).unwrap();
        assert_eq!(config.server.bind, "127.0.0.1");
        assert!(!config.database.reset_on_start);
        assert!(config.event_destinations.is_empty());
    }

    #[test]
    fn parses_event_destinations() {
        let text = r#"
[server]
port = 9999
public_base_url = "http://localhost:9999"
[database]
path = "x.db"
[main_account]

[[event_destinations]]
name = "thin-app"
event_payload = "thin"
enabled_events = ["v2.core.account.created", "v2.core.account.updated"]
webhook_endpoint = { url = "http://localhost:4000/thin" }
"#;
        let config = Config::from_str(text).unwrap();
        assert_eq!(config.event_destinations.len(), 1);
        let destination = &config.event_destinations[0];
        assert_eq!(destination.name, "thin-app");
        assert_eq!(destination.event_payload, "thin");
        assert_eq!(destination.events_from, vec!["@self".to_string()]);
        assert!(destination.enabled);
        assert_eq!(destination.enabled_events.len(), 2);
        assert_eq!(
            destination.webhook_endpoint.as_ref().unwrap().url,
            "http://localhost:4000/thin"
        );
    }

    #[test]
    fn rejects_invalid_event_payload() {
        let text = r#"
[server]
port = 9999
public_base_url = "http://localhost:9999"
[database]
path = "x.db"
[main_account]

[[event_destinations]]
name = "bad"
event_payload = "fat"
webhook_endpoint = { url = "http://localhost:4000/thin" }
"#;
        assert!(Config::from_str(text).is_err());
    }

    #[test]
    fn rejects_missing_webhook_endpoint() {
        let text = r#"
[server]
port = 9999
public_base_url = "http://localhost:9999"
[database]
path = "x.db"
[main_account]

[[event_destinations]]
name = "bad"
event_payload = "thin"
"#;
        assert!(Config::from_str(text).is_err());
    }
}
