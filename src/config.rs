use config::{Config as ConfigTree, ConfigError, Environment, File};
use nostr_sdk::{Keys, SecretKey};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

const ENVIRONMENT_PREFIX: &str = "NIP29";
const CONFIG_SEPARATOR: &str = "__";

#[derive(Debug, Deserialize)]
pub struct RelaySettings {
    pub relay_secret_key: String,
    pub local_addr: String,
    pub relay_url: String,
    pub auth_url: String,
    pub db_path: String,
    pub websocket: WebSocketSettings,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WebSocketSettings {
    #[serde(default = "default_channel_size")]
    pub channel_size: usize,
    #[serde(with = "humantime_serde", default)]
    pub max_connection_time: Option<Duration>,
    #[serde(default)]
    pub max_connections: Option<usize>,
}

fn default_channel_size() -> usize {
    100
}

impl RelaySettings {
    pub fn relay_keys(&self) -> Result<Keys, anyhow::Error> {
        let secret_key = SecretKey::from_hex(&self.relay_secret_key)?;
        Ok(Keys::new(secret_key))
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

        ConfigTree::builder()
            .add_source(File::from(default_config))
            .add_source(File::from(env_config).required(false))
            .add_source(File::from(local_config).required(false))
            .add_source(
                Environment::with_prefix(ENVIRONMENT_PREFIX)
                    .separator(CONFIG_SEPARATOR)
                    .try_parsing(true),
            )
            .build()
            .map(|config| Config { config })
    }

    pub fn get_settings(&self) -> Result<RelaySettings, ConfigError> {
        self.config.get("relay")
    }
}
