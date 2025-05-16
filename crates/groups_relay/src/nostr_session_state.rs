use crate::error::Error;
use crate::groups::Groups;
use crate::handler::CURRENT_REQUEST_HOST;
use crate::nostr_database::RelayDatabase;
use crate::subdomain::extract_subdomain;
use crate::{StoreCommand, SubscriptionManager};
use anyhow::Result;
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
    pub subscription_manager: Option<SubscriptionManager>,
    pub connection_token: CancellationToken,
    pub event_start_time: Option<Instant>,
    pub event_kind: Option<u16>,
    pub subdomain: Option<String>,
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
            subdomain: None,
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
            subdomain: None,
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

        let connection = SubscriptionManager::new(database, sender)
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
                return Err(e);
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

    pub fn subdomain(&self) -> Option<&str> {
        self.subdomain.as_deref()
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

        let subdomain =
            host_opt.and_then(|host_str| extract_subdomain(&host_str, self.base_domain_parts));

        NostrConnectionState {
            relay_url: self.relay_url.clone(),
            challenge: None,
            authed_pubkey: None,
            subscription_manager: None,
            connection_token: token,
            subdomain,
            event_start_time: None,
            event_kind: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Import items from the parent module (NostrConnectionFactory, NostrConnectionState, etc.)
    use crate::groups::Groups;
    use crate::handler::CURRENT_REQUEST_HOST; // To set the task-local
    use crate::nostr_database::RelayDatabase;
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

        let database = Arc::new(
            RelayDatabase::new(db_path.to_str().unwrap(), relay_keys.clone())
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

        assert_eq!(connection_state.subdomain.as_deref(), Some("test"));
        assert_eq!(connection_state.subdomain(), Some("test"));
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
        assert_eq!(connection_state.subdomain, None);
    }

    #[tokio::test]
    async fn test_subdomain_extraction_task_local_not_set() {
        let (factory, _tmp_dir) = create_test_factory(2).await;
        let cancellation_token = CancellationToken::new();

        let connection_state = factory.create_state(cancellation_token);

        assert_eq!(connection_state.subdomain, None);
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
        assert_eq!(connection_state.subdomain.as_deref(), Some("sub.test"));
    }
}
