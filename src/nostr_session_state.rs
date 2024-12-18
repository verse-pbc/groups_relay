use crate::error::Error;
use crate::{EventToSave, RelayClientConnection};
use nostr_sdk::prelude::*;
use std::backtrace::Backtrace;
use tokio_util::sync::CancellationToken;
use websocket_builder::StateFactory;

#[derive(Debug, Clone)]
pub struct NostrConnectionState {
    pub relay_url: String,
    pub challenge: Option<String>,
    pub authed_pubkey: Option<PublicKey>,
    pub relay_connection: Option<RelayClientConnection>,
    pub connection_token: CancellationToken,
}

impl NostrConnectionState {
    pub fn is_authenticated(&self) -> bool {
        self.authed_pubkey.is_some()
    }

    pub async fn save_events(&self, events_to_save: Vec<EventToSave>) -> Result<(), Error> {
        let Some(connection) = self.relay_connection.as_ref() else {
            return Err(Error::Internal {
                message: "No connection".to_string(),
                backtrace: Backtrace::capture(),
            });
        };

        for event in events_to_save {
            connection.save_event(event).await?
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

pub struct NostrConnectionFactory {
    relay_url: String,
}

impl NostrConnectionFactory {
    pub fn new(relay_url: String) -> Self {
        Self { relay_url }
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
        }
    }
}
