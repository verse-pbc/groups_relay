use crate::error::Error;
use crate::nostr_database::RelayDatabase;
use crate::{StoreCommand, SubscriptionManager};
use nostr_sdk::prelude::*;
use std::backtrace::Backtrace;
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
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.authed_pubkey.is_some()
    }

    /// Sets up a new event store connection
    pub async fn setup_connection(
        &mut self,
        database: Arc<RelayDatabase>,
        sender: MessageSender<RelayMessage>,
    ) -> Result<(), Error> {
        debug!("Setting up connection",);

        let connection = SubscriptionManager::new(database, sender)
            .await
            .map_err(|e| Error::Internal {
                message: format!("Failed to create connection: {}", e),
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

    pub fn get_challenge_event(&mut self) -> RelayMessage {
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
}

#[derive(Clone)]
pub struct NostrConnectionFactory {
    relay_url: RelayUrl,
}

impl NostrConnectionFactory {
    pub fn new(relay_url: String) -> Result<Self, Error> {
        let relay_url = RelayUrl::parse(&relay_url).map_err(|e| Error::Internal {
            message: format!("Invalid relay URL: {}", e),
            backtrace: Backtrace::capture(),
        })?;
        Ok(Self { relay_url })
    }
}

impl StateFactory<NostrConnectionState> for NostrConnectionFactory {
    fn create_state(&self, token: CancellationToken) -> NostrConnectionState {
        NostrConnectionState {
            challenge: None,
            authed_pubkey: None,
            relay_url: self.relay_url.clone(),
            subscription_manager: None,
            connection_token: token,
            event_start_time: None,
            event_kind: None,
        }
    }
}
