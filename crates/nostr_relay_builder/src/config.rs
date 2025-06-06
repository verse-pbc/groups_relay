//! Configuration options for the relay builder

use crate::database::RelayDatabase;
use crate::error::Error;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::sync::Arc;

/// Configuration for scope/subdomain handling
#[derive(Debug, Clone)]
pub enum ScopeConfig {
    /// No scope/subdomain support - all data stored in default scope
    Disabled,
    /// Enable subdomain extraction with specified base domain parts
    Subdomain {
        /// Number of parts that constitute the base domain
        /// For example, with 2: "sub.example.com" -> base is "example.com"
        base_domain_parts: usize,
    },
    /// Use a fixed scope for all connections
    Fixed {
        /// The scope to use for all connections
        scope: Scope,
    },
}

impl Default for ScopeConfig {
    fn default() -> Self {
        Self::Disabled
    }
}

impl ScopeConfig {
    /// Create a subdomain-based scope configuration
    pub fn subdomain(base_domain_parts: usize) -> Self {
        Self::Subdomain { base_domain_parts }
    }

    /// Create a fixed scope configuration
    pub fn fixed(scope: Scope) -> Self {
        Self::Fixed { scope }
    }

    /// Resolve a scope from a host string
    pub fn resolve_scope(&self, host: Option<&str>) -> Scope {
        match self {
            Self::Disabled => Scope::Default,
            Self::Subdomain { base_domain_parts } => host
                .and_then(|h| crate::subdomain::extract_subdomain(h, *base_domain_parts))
                .and_then(|s| {
                    if !s.is_empty() {
                        Scope::named(&s).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(Scope::Default),
            Self::Fixed { scope } => scope.clone(),
        }
    }
}

/// WebSocket server configuration
#[derive(Debug, Clone)]
pub struct WebSocketConfig {
    /// Channel size for message passing
    pub channel_size: usize,
    /// Maximum number of concurrent connections
    pub max_connections: Option<usize>,
    /// Maximum connection time in seconds
    pub max_connection_time: Option<u64>,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            channel_size: 100,
            max_connections: None,
            max_connection_time: None,
        }
    }
}

/// Database configuration - either a path or an existing database instance
#[derive(Debug, Clone)]
pub enum DatabaseConfig {
    /// Create a new database at the specified path
    Path(String),
    /// Use an existing database instance
    Instance(Arc<RelayDatabase>),
}

impl From<String> for DatabaseConfig {
    fn from(path: String) -> Self {
        DatabaseConfig::Path(path)
    }
}

impl From<&str> for DatabaseConfig {
    fn from(path: &str) -> Self {
        DatabaseConfig::Path(path.to_string())
    }
}

impl From<Arc<RelayDatabase>> for DatabaseConfig {
    fn from(db: Arc<RelayDatabase>) -> Self {
        DatabaseConfig::Instance(db)
    }
}

/// Main configuration for the relay
#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// URL of the relay (used for NIP-42 auth and other purposes)
    pub relay_url: String,
    /// Database configuration
    pub database: DatabaseConfig,
    /// Relay keys
    pub keys: Keys,
    /// Scope configuration
    pub scope_config: ScopeConfig,
    /// Whether to enable NIP-42 authentication
    pub enable_auth: bool,
    /// Authentication configuration (if enabled)
    pub auth_config: Option<crate::middlewares::AuthConfig>,
    /// WebSocket server configuration
    pub websocket_config: WebSocketConfig,
    /// Query limit for REQ filters (both default and maximum)
    pub query_limit: usize,
}

impl RelayConfig {
    /// Create a new relay configuration
    pub fn new<D: Into<DatabaseConfig>>(
        relay_url: impl Into<String>,
        database: D,
        keys: Keys,
    ) -> Self {
        Self {
            relay_url: relay_url.into(),
            database: database.into(),
            keys,
            scope_config: ScopeConfig::default(),
            enable_auth: false,
            auth_config: None,
            websocket_config: WebSocketConfig::default(),
            query_limit: 500,
        }
    }

    /// Create database instance from configuration
    pub fn create_database(&self) -> Result<Arc<RelayDatabase>, Error> {
        match &self.database {
            DatabaseConfig::Path(path) => {
                Ok(Arc::new(RelayDatabase::new(path, self.keys.clone())?))
            }
            DatabaseConfig::Instance(db) => Ok(db.clone()),
        }
    }

    /// Set the scope configuration
    pub fn with_scope_config(mut self, scope_config: ScopeConfig) -> Self {
        self.scope_config = scope_config;
        self
    }

    /// Enable subdomain-based scopes
    pub fn with_subdomains(mut self, base_domain_parts: usize) -> Self {
        self.scope_config = ScopeConfig::subdomain(base_domain_parts);
        self
    }

    /// Enable NIP-42 authentication
    pub fn with_auth(mut self, auth_config: crate::middlewares::AuthConfig) -> Self {
        self.enable_auth = true;
        self.auth_config = Some(auth_config);
        self
    }

    /// Configure WebSocket server settings
    pub fn with_websocket_config(mut self, config: WebSocketConfig) -> Self {
        self.websocket_config = config;
        self
    }

    /// Set the channel size for WebSocket connections
    pub fn with_channel_size(mut self, size: usize) -> Self {
        self.websocket_config.channel_size = size;
        self
    }

    /// Set the maximum number of concurrent connections
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.websocket_config.max_connections = Some(max);
        self
    }

    /// Set the maximum connection time in seconds
    pub fn with_max_connection_time(mut self, seconds: u64) -> Self {
        self.websocket_config.max_connection_time = Some(seconds);
        self
    }

    /// Set the query limit for REQ filters
    pub fn with_query_limit(mut self, limit: usize) -> Self {
        self.query_limit = limit;
        self
    }
}

// No Default implementation as RelayConfig requires Keys to be explicitly provided
