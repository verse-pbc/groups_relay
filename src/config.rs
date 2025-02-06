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
    #[serde(default)]
    pub websocket: WebSocketSettings,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct WebSocketSettings {
    #[serde(
        default = "default_channel_size",
        deserialize_with = "validate_channel_size"
    )]
    pub channel_size: usize,
    #[serde(with = "humantime_serde", default)]
    pub max_connection_time: Option<Duration>,
    #[serde(default)]
    pub max_connections: Option<usize>,
}

fn default_channel_size() -> usize {
    300 // Default channel size matching settings.yml
}

fn validate_channel_size<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let size = usize::deserialize(deserializer)?;
    if size == 0 {
        return Err(D::Error::custom("channel_size must be greater than 0"));
    }
    Ok(size)
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
        // Only log non-sensitive websocket configuration
        tracing::debug!(
            "WebSocket config: channel_size={}, max_connections={:?}, max_connection_time={:?}",
            settings.websocket.channel_size,
            settings.websocket.max_connections,
            settings.websocket.max_connection_time,
        );
        Ok(settings)
    }
}
