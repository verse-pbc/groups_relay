use crate::error::Error;
use crate::groups::Groups;
use nostr_relay_builder::{subdomain::extract_subdomain, RelayDatabase, SubscriptionService, state::CURRENT_REQUEST_HOST, StoreCommand};
use anyhow::Result;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use snafu::Backtrace;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};
use websocket_builder::{MessageSender, StateFactory};

const DEFAULT_RELAY_URL: &str = "wss://default.relay";

#[derive(Debug, Clone)]
pub struct NostrConnectionState {
    pub relay_url: RelayUrl,
    pub challenge: Option<String>,
    pub authed_pubkey: Option<PublicKey>,
    pub subscription_manager: Option<SubscriptionService>,
    pub connection_token: CancellationToken,
    pub event_start_time: Option<Instant>,
    pub event_kind: Option<u16>,
    pub subdomain: Scope,
}

impl Default for NostrConnectionState {
    fn default() -> Self {
        Self {
            relay_url: RelayUrl::parse(DEFAULT_RELAY_URL).expect("Invalid default relay URL"),
            challenge: None,
            authed_pubkey: None,
            subscription_manager: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
            event_kind: None,
            subdomain: Scope::Default,
        }
    }
}

impl NostrConnectionState {
    pub fn new(relay_url: String) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url).map_err(|e| Error::Internal {
            message: format!("Invalid relay URL: {}", e),
            backtrace: Backtrace::capture(),
        })?;

        Ok(Self {
            relay_url,
            challenge: None,
            authed_pubkey: None,
            subscription_manager: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
            event_kind: None,
            subdomain: Scope::Default,
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.authed_pubkey.is_some()
    }

    pub async fn setup_connection(
        &mut self,
        database: Arc<RelayDatabase>,
        sender: MessageSender<RelayMessage<'static>>,
    ) -> Result<(), Error> {
        debug!("Setting up connection",);

        let connection = SubscriptionService::new(database, sender)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to create subscription manager: {}", e),
                backtrace: Backtrace::capture(),
            })?;

        self.subscription_manager = Some(connection);

        debug!("Connection setup complete",);

        Ok(())
    }

    pub async fn save_events(&mut self, events: Vec<StoreCommand>) -> Result<(), Error> {
        let Some(connection) = &self.subscription_manager else {
            return Err(Error::Internal {
                message: "No connection available".to_string(),
                backtrace: Backtrace::capture(),
            });
        };

        for event in events {
            if let Err(e) = connection.save_and_broadcast(event).await {
                error!("Failed to save event: {}", e);
                return Err(Error::Internal {
                    message: format!("Failed to save event: {}", e),
                    backtrace: Backtrace::capture(),
                });
            }
        }

        Ok(())
    }

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

    pub fn cleanup(&self) {
        // ... existing code ...
    }

    /// Convert the Scope to an Option<&str> for backward compatibility with code that
    /// expects Option<&str> representing a subdomain.
    /// This is NOT used for database operations, only for logging and compatibility.
    pub fn subdomain_str(&self) -> Option<&str> {
        match &self.subdomain {
            Scope::Named { name, .. } => Some(name),
            Scope::Default => None,
        }
    }

    pub fn subdomain(&self) -> &Scope {
        &self.subdomain
    }
}

#[derive(Clone)]
pub struct NostrConnectionFactory {
    relay_url: RelayUrl,
    #[allow(dead_code)]
    database: Arc<RelayDatabase>,
    #[allow(dead_code)]
    groups: Arc<Groups>,
    base_domain_parts: usize,
}

impl NostrConnectionFactory {
    pub fn new(
        relay_url: String,
        database: Arc<RelayDatabase>,
        groups: Arc<Groups>,
        base_domain_parts: usize,
    ) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url).map_err(|e| Error::Internal {
            message: format!("Invalid relay URL: {}", e),
            backtrace: Backtrace::capture(),
        })?;
        Ok(Self {
            relay_url,
            database,
            groups,
            base_domain_parts,
        })
    }
}

impl StateFactory<NostrConnectionState> for NostrConnectionFactory {
    fn create_state(&self, token: CancellationToken) -> NostrConnectionState {
        let host_opt: Option<String> = CURRENT_REQUEST_HOST
            .try_with(|current_host_opt_ref| current_host_opt_ref.clone())
            .unwrap_or_else(|_| {
                tracing::warn!(
                    "CURRENT_REQUEST_HOST task_local not found when creating NostrConnectionState."
                );
                None
            });

        let subdomain_str =
            host_opt.and_then(|host_str| extract_subdomain(&host_str, self.base_domain_parts));

        // Convert subdomain string to scope
        let subdomain_scope = subdomain_str
            .and_then(|s| {
                if !s.is_empty() {
                    match Scope::named(s.as_str()) {
                        Ok(scope) => Some(scope),
                        Err(e) => {
                            tracing::warn!("Failed to create named scope: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            })
            .unwrap_or(Scope::Default);

        NostrConnectionState {
            relay_url: self.relay_url.clone(),
            challenge: None,
            authed_pubkey: None,
            subscription_manager: None,
            connection_token: token,
            event_start_time: None,
            event_kind: None,
            subdomain: subdomain_scope,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Import items from the parent module (NostrConnectionFactory, NostrConnectionState, etc.)
    use crate::groups::Groups;
    use nostr_relay_builder::state::CURRENT_REQUEST_HOST; // To set the task-local
    use nostr_relay_builder::{RelayDatabase, crypto_worker::CryptoWorker};
    use nostr_sdk::prelude::Keys; // Removed RelayUrl from here
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    // Helper to create a NostrConnectionFactory for tests
    async fn create_test_factory(base_domain_parts: usize) -> (NostrConnectionFactory, TempDir) {
        // Return TempDir to keep it alive
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("test_nostr_session_state.db");
        let relay_keys = Keys::generate();

        let cancellation_token = CancellationToken::new();
        let crypto_worker = Arc::new(CryptoWorker::new(Arc::new(relay_keys.clone()), cancellation_token));
        let database = Arc::new(
            RelayDatabase::new(db_path.to_str().unwrap(), crypto_worker)
                .expect("Failed to create test database"),
        );

        let groups = Arc::new(
            Groups::load_groups(Arc::clone(&database), relay_keys.public_key())
                .await
                .expect("Failed to load groups for test"),
        );

        let dummy_relay_url_str = "wss://test.relay";
        let factory = NostrConnectionFactory::new(
            dummy_relay_url_str.to_string(),
            database,
            groups,
            base_domain_parts,
        )
        .expect("Failed to create test factory");
        (factory, tmp_dir)
    }

    #[tokio::test]
    async fn test_subdomain_extraction_happy_path() {
        let (factory, _tmp_dir) = create_test_factory(2).await;
        let host_header_value = Some("test.example.com".to_string());
        let cancellation_token = CancellationToken::new();

        let connection_state = CURRENT_REQUEST_HOST
            .scope(host_header_value, async {
                factory.create_state(cancellation_token)
            })
            .await;

        assert_eq!(connection_state.subdomain_str(), Some("test"));
        match connection_state.subdomain {
            Scope::Named { name, .. } => assert_eq!(name, "test"),
            _ => panic!("Expected a named scope"),
        }
    }

    #[tokio::test]
    async fn test_subdomain_extraction_no_subdomain() {
        let (factory, _tmp_dir) = create_test_factory(2).await;
        let host_header_value = Some("example.com".to_string());
        let cancellation_token = CancellationToken::new();

        let connection_state = CURRENT_REQUEST_HOST
            .scope(host_header_value, async {
                factory.create_state(cancellation_token.clone())
            })
            .await;

        assert_eq!(connection_state.subdomain_str(), None);
        match connection_state.subdomain {
            Scope::Default => {} // This is expected
            _ => panic!("Expected the Default scope"),
        }
    }

    #[tokio::test]
    async fn test_subdomain_extraction_task_local_not_set() {
        let (factory, _tmp_dir) = create_test_factory(2).await;
        let cancellation_token = CancellationToken::new();

        let connection_state = factory.create_state(cancellation_token);

        assert_eq!(connection_state.subdomain_str(), None);
        match connection_state.subdomain {
            Scope::Default => {} // This is expected
            _ => panic!("Expected the Default scope"),
        }
    }

    #[tokio::test]
    async fn test_subdomain_extraction_different_base_parts() {
        let (factory, _tmp_dir) = create_test_factory(3).await;
        let host_header_value = Some("sub.test.example.co.uk".to_string());
        let cancellation_token = CancellationToken::new();

        let connection_state = CURRENT_REQUEST_HOST
            .scope(host_header_value, async {
                factory.create_state(cancellation_token)
            })
            .await;

        assert_eq!(connection_state.subdomain_str(), Some("sub.test"));
        match connection_state.subdomain {
            Scope::Named { name, .. } => assert_eq!(name, "sub.test"),
            _ => panic!("Expected a named scope"),
        }
    }
}
