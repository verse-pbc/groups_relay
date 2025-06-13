//! Connection state management

use crate::config::ScopeConfig;
use crate::database::RelayDatabase;
use crate::error::Error;
use crate::subscription_service::{StoreCommand, SubscriptionService};
use anyhow::Result;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};
use websocket_builder::{MessageSender, StateFactory};

const DEFAULT_RELAY_URL: &str = "wss://default.relay";

tokio::task_local! {
    /// Task-local storage for the current request host.
    /// This is set by the HTTP handler before processing WebSocket connections.
    pub static CURRENT_REQUEST_HOST: Option<String>;
}

/// Type alias for the default NostrConnectionState without custom state
pub type DefaultNostrConnectionState = NostrConnectionState<()>;

/// Connection state for a WebSocket client
#[derive(Debug, Clone)]
pub struct NostrConnectionState<T = ()> {
    /// The relay URL this connection is for
    pub relay_url: RelayUrl,
    /// Challenge for NIP-42 authentication
    pub challenge: Option<String>,
    /// Authenticated public key (if authenticated via NIP-42)
    pub authed_pubkey: Option<PublicKey>,
    /// Subscription service for this connection (private - use methods below)
    subscription_service: Option<SubscriptionService>,
    /// Cancellation token for this connection
    pub connection_token: CancellationToken,
    /// Start time for event processing (for metrics)
    pub event_start_time: Option<Instant>,
    /// Kind of event being processed (for metrics)
    pub event_kind: Option<u16>,
    /// Subdomain/scope for this connection
    pub subdomain: Scope,
    /// Custom state for application-specific data
    pub custom: T,
}

impl<T: Default> Default for NostrConnectionState<T> {
    fn default() -> Self {
        Self {
            relay_url: RelayUrl::parse(DEFAULT_RELAY_URL).expect("Invalid default relay URL"),
            challenge: None,
            authed_pubkey: None,
            subscription_service: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
            event_kind: None,
            subdomain: Scope::Default,
            custom: T::default(),
        }
    }
}

impl<T: Default> NostrConnectionState<T> {
    /// Create a new connection state
    pub fn new(relay_url: String) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url)
            .map_err(|e| Error::internal(format!("Invalid relay URL: {}", e)))?;

        Ok(Self {
            relay_url,
            challenge: None,
            authed_pubkey: None,
            subscription_service: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
            event_kind: None,
            subdomain: Scope::Default,
            custom: T::default(),
        })
    }
}

impl<T> NostrConnectionState<T> {
    /// Create a new connection state with custom data
    pub fn with_custom(relay_url: String, custom: T) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url)
            .map_err(|e| Error::internal(format!("Invalid relay URL: {}", e)))?;

        Ok(Self {
            relay_url,
            challenge: None,
            authed_pubkey: None,
            subscription_service: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
            event_kind: None,
            subdomain: Scope::Default,
            custom,
        })
    }

    /// Check if the connection is authenticated
    pub fn is_authenticated(&self) -> bool {
        self.authed_pubkey.is_some()
    }

    /// Set up the connection with a database and message sender
    pub async fn setup_connection(
        &mut self,
        database: Arc<RelayDatabase>,
        sender: MessageSender<RelayMessage<'static>>,
    ) -> Result<(), Error> {
        debug!("Setting up connection");

        let metrics_handler = crate::global_metrics::get_subscription_metrics_handler();

        let service = if let Some(handler) = metrics_handler {
            SubscriptionService::new_with_metrics(database, sender, Some(handler))
                .await
                .map_err(|e| {
                    Error::internal(format!("Failed to create subscription service: {}", e))
                })?
        } else {
            SubscriptionService::new(database, sender)
                .await
                .map_err(|e| {
                    Error::internal(format!("Failed to create subscription service: {}", e))
                })?
        };
        self.subscription_service = Some(service);

        debug!("Connection setup complete");
        Ok(())
    }

    /// Save events to the database
    pub async fn save_events(&mut self, events: Vec<StoreCommand>) -> Result<(), Error> {
        let Some(service) = &self.subscription_service else {
            return Err(Error::internal("No subscription service available"));
        };

        for event in events {
            if let Err(e) = service.save_and_broadcast(event).await {
                error!("Failed to save event: {}", e);
                return Err(e);
            }
        }

        Ok(())
    }

    /// Save and broadcast a single store command
    pub async fn save_and_broadcast(&self, command: StoreCommand) -> Result<(), Error> {
        let Some(service) = &self.subscription_service else {
            return Err(Error::internal("No subscription service available"));
        };

        service.save_and_broadcast(command).await
    }

    /// Remove a subscription
    pub fn remove_subscription(&self, subscription_id: SubscriptionId) -> Result<(), Error> {
        let Some(service) = &self.subscription_service else {
            return Err(Error::internal("No subscription service available"));
        };

        service.remove_subscription(subscription_id)
    }

    /// Get the subscription service (only for internal use)
    pub(crate) fn subscription_service(&self) -> Option<&SubscriptionService> {
        self.subscription_service.as_ref()
    }

    /// Set the subscription service (only for testing)
    #[cfg(test)]
    pub(crate) fn set_subscription_service(&mut self, service: SubscriptionService) {
        self.subscription_service = Some(service);
    }

    /// Get or create a challenge for NIP-42 authentication
    pub fn get_challenge_event(&mut self) -> RelayMessage<'static> {
        let challenge = match self.challenge.as_ref() {
            Some(challenge) => challenge.clone(),
            None => {
                let challenge = format!("{}", rand::random::<u64>());
                self.challenge = Some(challenge.clone());
                challenge
            }
        };
        RelayMessage::auth(challenge)
    }

    /// Clean up the connection
    pub fn cleanup(&self) {
        debug!("Cleaning up connection");

        if let Some(subscription_service) = &self.subscription_service {
            subscription_service.cleanup();
        }

        self.connection_token.cancel();
    }

    /// Convert the Scope to an Option<&str> for backward compatibility
    pub fn subdomain_str(&self) -> Option<&str> {
        match &self.subdomain {
            Scope::Named { name, .. } => Some(name),
            Scope::Default => None,
        }
    }

    /// Get subdomain as owned string to avoid borrowing issues
    pub fn subdomain_string(&self) -> Option<String> {
        match &self.subdomain {
            Scope::Named { name, .. } => Some(name.clone()),
            Scope::Default => None,
        }
    }

    /// Get the subdomain scope
    pub fn subdomain(&self) -> &Scope {
        &self.subdomain
    }
}

/// Factory for creating NostrConnectionState instances
#[derive(Clone)]
pub struct NostrConnectionFactory<T = ()> {
    relay_url: RelayUrl,
    #[allow(dead_code)]
    database: Arc<RelayDatabase>,
    scope_config: ScopeConfig,
    _phantom: std::marker::PhantomData<T>,
}

/// Type alias for the original non-generic factory
pub type DefaultNostrConnectionFactory = NostrConnectionFactory<()>;

impl<T> NostrConnectionFactory<T> {
    /// Create a new connection factory
    pub fn new(
        relay_url: String,
        database: Arc<RelayDatabase>,
        scope_config: ScopeConfig,
    ) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url)
            .map_err(|e| Error::internal(format!("Invalid relay URL: {}", e)))?;

        Ok(Self {
            relay_url,
            database,
            scope_config,
            _phantom: std::marker::PhantomData,
        })
    }
}

impl<T: Default> StateFactory<NostrConnectionState<T>> for NostrConnectionFactory<T> {
    fn create_state(&self, token: CancellationToken) -> NostrConnectionState<T> {
        // Try to get host information from task-local storage
        let host_opt: Option<String> = CURRENT_REQUEST_HOST
            .try_with(|current_host_opt_ref| current_host_opt_ref.clone())
            .unwrap_or_else(|_| {
                warn!(
                    "CURRENT_REQUEST_HOST task_local not found when creating NostrConnectionState."
                );
                None
            });

        // Resolve scope based on configuration
        let subdomain_scope = self.scope_config.resolve_scope(host_opt.as_deref());

        let mut state = NostrConnectionState::new(self.relay_url.to_string())
            .unwrap_or_else(|_| NostrConnectionState::default());

        state.connection_token = token;
        state.subdomain = subdomain_scope;

        state
    }
}

/// Generic factory for creating NostrConnectionState instances with custom state
#[derive(Clone)]
pub struct GenericNostrConnectionFactory<T> {
    relay_url: RelayUrl,
    #[allow(dead_code)]
    database: Arc<RelayDatabase>,
    scope_config: ScopeConfig,
    state_factory: Arc<dyn Fn() -> T + Send + Sync>,
}

impl<T> GenericNostrConnectionFactory<T> {
    /// Create a new connection factory with a state factory function
    pub fn new_with_factory(
        relay_url: String,
        database: Arc<RelayDatabase>,
        scope_config: ScopeConfig,
        state_factory: Arc<dyn Fn() -> T + Send + Sync>,
    ) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url)
            .map_err(|e| Error::internal(format!("Invalid relay URL: {}", e)))?;

        Ok(Self {
            relay_url,
            database,
            scope_config,
            state_factory,
        })
    }

    /// Create a new connection factory that uses Default::default() for state
    pub fn new(
        relay_url: String,
        database: Arc<RelayDatabase>,
        scope_config: ScopeConfig,
    ) -> Result<Self, Error>
    where
        T: Default + 'static,
    {
        let factory = Arc::new(|| T::default());
        Self::new_with_factory(relay_url, database, scope_config, factory)
    }
}

impl<T: Send + Sync + 'static> StateFactory<NostrConnectionState<T>>
    for GenericNostrConnectionFactory<T>
{
    fn create_state(&self, token: CancellationToken) -> NostrConnectionState<T> {
        // Try to get host information from task-local storage
        let host_opt: Option<String> = CURRENT_REQUEST_HOST
            .try_with(|current_host_opt_ref| current_host_opt_ref.clone())
            .unwrap_or_else(|_| {
                warn!(
                    "CURRENT_REQUEST_HOST task_local not found when creating NostrConnectionState."
                );
                None
            });

        // Resolve scope based on configuration
        let subdomain_scope = self.scope_config.resolve_scope(host_opt.as_deref());

        let custom = (self.state_factory)();

        let mut state = NostrConnectionState::with_custom(self.relay_url.to_string(), custom)
            .unwrap_or_else(|_| panic!("Failed to create NostrConnectionState"));

        state.connection_token = token;
        state.subdomain = subdomain_scope;

        state
    }
}
