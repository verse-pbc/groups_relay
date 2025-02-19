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
    pub auth_url: String,
    pub db_path: String,
    #[serde(default)]
    pub websocket: WebSocketSettings,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WebSocketSettings {
    #[serde(default = "default_channel_size")]
    pub channel_size: usize,
    #[serde(with = "humantime_serde", default = "default_max_connection_time")]
    pub max_connection_time: Option<Duration>,
    #[serde(default = "default_max_connections")]
    pub max_connections: Option<usize>,
}

fn default_channel_size() -> usize {
    300 // Default channel size
}

fn default_max_connection_time() -> Option<Duration> {
    Some(Duration::from_secs(10 * 60)) // 10 minutes default
}

fn default_max_connections() -> Option<usize> {
    Some(1000) // Default max connections
}

impl RelaySettings {
    pub fn relay_keys(&self) -> Result<Keys, anyhow::Error> {
        let secret_key = SecretKey::from_hex(&self.relay_secret_key)?;
        Ok(Keys::new(secret_key))
    }

    pub fn relay_url(&self) -> Result<RelayUrl, anyhow::Error> {
        Ok(RelayUrl::parse(&self.relay_url)?)
    }

    pub fn auth_url(&self) -> Result<RelayUrl, anyhow::Error> {
        Ok(RelayUrl::parse(&self.auth_url)?)
    }
}

impl WebSocketSettings {
    pub fn channel_size(&self) -> usize {
        if self.channel_size == 0 {
            default_channel_size()
        } else {
            self.channel_size
        }
    }

    pub fn max_connection_time(&self) -> Option<Duration> {
        self.max_connection_time
            .or_else(default_max_connection_time)
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
        let env_config = config_dir.join(format!("settings.{}.yml", environment));
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
            "WebSocket settings: channel_size={}, max_connections={:?}, max_connection_time={:?}",
            settings.websocket.channel_size,
            settings.websocket.max_connections,
            settings.websocket.max_connection_time,
        );
        Ok(settings)
    }
}

pub struct Settings {
    pub relay_url: String,
    pub local_addr: String,
    pub auth_url: String,
    pub admin_keys: Vec<String>,
    pub websocket: WebSocketSettings,
    pub db_path: String,
}

pub use nostr_sdk::Keys;
