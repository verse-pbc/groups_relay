use anyhow::Result;
use config::{Config as ConfigTree, ConfigError, Environment, File};
use nostr_sdk::prelude::*;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tracing::info;

const ENVIRONMENT_PREFIX: &str = "NIP29";
const CONFIG_SEPARATOR: &str = "__";

#[derive(Debug, Deserialize)]
pub struct RelaySettings {
    pub relay_secret_key: String,
    pub local_addr: String,
    pub relay_url: String,
    pub db_path: String,
    #[serde(default)]
    pub websocket: WebSocketSettings,
    #[serde(default = "default_max_limit")]
    pub max_limit: usize,
    #[serde(default = "default_max_subscriptions")]
    pub max_subscriptions: usize,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WebSocketSettings {
    #[serde(with = "humantime_serde", default = "default_max_connection_duration")]
    pub max_connection_duration: Option<Duration>,
    #[serde(with = "humantime_serde", default = "default_idle_timeout")]
    pub idle_timeout: Option<Duration>,
    #[serde(default = "default_max_connections")]
    pub max_connections: Option<usize>,
}

fn default_max_connection_duration() -> Option<Duration> {
    Some(Duration::from_secs(10 * 60)) // 10 minutes default
}

fn default_idle_timeout() -> Option<Duration> {
    Some(Duration::from_secs(10 * 60)) // 10 minutes default, same as max_connection_duration
}

fn default_max_connections() -> Option<usize> {
    Some(1000) // Default max connections
}

fn default_max_limit() -> usize {
    500 // Default/maximum limit for queries
}

fn default_max_subscriptions() -> usize {
    50 // Default max subscriptions per connection
}

impl RelaySettings {
    pub fn relay_keys(&self) -> Result<Keys, anyhow::Error> {
        let secret_key = SecretKey::from_hex(&self.relay_secret_key)?;
        Ok(Keys::new(secret_key))
    }

    pub fn relay_url(&self) -> Result<RelayUrl, anyhow::Error> {
        Ok(RelayUrl::parse(&self.relay_url)?)
    }
}

impl WebSocketSettings {
    pub fn max_connection_duration(&self) -> Option<Duration> {
        self.max_connection_duration
            .or_else(default_max_connection_duration)
    }

    pub fn idle_timeout(&self) -> Option<Duration> {
        self.idle_timeout
            .or_else(default_idle_timeout)
    }

    pub fn max_connections(&self) -> Option<usize> {
        self.max_connections.or_else(default_max_connections)
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    config: ConfigTree,
}

impl Config {
    pub fn new<P: AsRef<Path>>(config_dir: P) -> Result<Self, ConfigError> {
        let environment =
            std::env::var(format!("{ENVIRONMENT_PREFIX}{CONFIG_SEPARATOR}ENVIRONMENT"))
                .unwrap_or_else(|_| "development".into());

        let config_dir = config_dir.as_ref();
        let default_config = config_dir.join("settings.yml");
        let env_config = config_dir.join(format!("settings.{environment}.yml"));
        let local_config = config_dir.join("settings.local.yml");

        let config = ConfigTree::builder()
            .add_source(File::from(default_config))
            .add_source(File::from(env_config).required(false))
            .add_source(File::from(local_config).required(false))
            .add_source(
                Environment::with_prefix(ENVIRONMENT_PREFIX)
                    .separator(CONFIG_SEPARATOR)
                    .try_parsing(true),
            )
            .build()?;

        Ok(Config { config })
    }

    pub fn get_settings(&self) -> Result<RelaySettings, ConfigError> {
        let settings: RelaySettings = self.config.get("relay")?;
        // Only log non-sensitive WebSocket settings
        info!(
            "WebSocket settings: max_connections={:?}, max_connection_duration={:?}, idle_timeout={:?}",
            settings.websocket.max_connections,
            settings.websocket.max_connection_duration,
            settings.websocket.idle_timeout,
        );
        Ok(settings)
    }
}

pub struct Settings {
    pub relay_url: String,
    pub local_addr: String,
    pub admin_keys: Vec<String>,
    pub websocket: WebSocketSettings,
    pub db_path: String,
    pub max_limit: usize,
    pub max_subscriptions: usize,
}

pub use nostr_sdk::Keys;
