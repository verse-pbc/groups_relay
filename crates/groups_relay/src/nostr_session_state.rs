use crate::error::Error;
use crate::nostr_database::RelayDatabase;
use crate::{EventStoreConnection, StoreCommand};
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
    pub relay_connection: Option<EventStoreConnection>,
    pub connection_token: CancellationToken,
    pub event_start_time: Option<Instant>,
}

impl Default for NostrConnectionState {
    fn default() -> Self {
        Self {
            relay_url: RelayUrl::parse(DEFAULT_RELAY_URL).expect("Invalid default relay URL"),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
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
            relay_connection: None,
            connection_token: CancellationToken::new(),
            event_start_time: None,
        })
    }

    pub fn is_authenticated(&self) -> bool {
        self.authed_pubkey.is_some()
    }

    /// Sets up a new event store connection
    pub async fn setup_connection(
        &mut self,
        connection_id: String,
        database: Arc<RelayDatabase>,
        sender: MessageSender<RelayMessage>,
    ) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Setting up connection",
            connection_id
        );

        let connection = EventStoreConnection::new(
            connection_id.clone(),
            database,
            connection_id.clone(),
            self.connection_token.clone(),
            sender,
        )
        .await
        .map_err(|e| Error::Internal {
            message: format!("Failed to create connection: {}", e),
            backtrace: Backtrace::capture(),
        })?;

        self.relay_connection = Some(connection);

        debug!(
            target: "event_store",
            "[{}] Connection setup complete",
            connection_id
        );

        Ok(())
    }

    /// Cleans up the event store connection
    pub fn cleanup_connection(&mut self) {
        if let Some(connection) = &self.relay_connection {
            connection.cleanup();
        }
    }

    pub async fn save_events(&mut self, events: Vec<StoreCommand>) -> Result<(), Error> {
        let Some(connection) = &self.relay_connection else {
            return Err(Error::Internal {
                message: "No connection available".to_string(),
                backtrace: Backtrace::capture(),
            });
        };

        for event in events {
            if let Err(e) = connection.save_event(event).await {
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
            relay_connection: None,
            connection_token: token,
            event_start_time: None,
        }
    }
}
