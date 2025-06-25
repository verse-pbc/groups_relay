use anyhow::Result;
use nostr_database::DatabaseError;
use nostr_relay_builder::NostrConnectionState;
use nostr_sdk::client::Error as NostrSdkError;
use nostr_sdk::prelude::*;
use nostr_sdk::RelayMessage;
use snafu::{Backtrace, Snafu};
use std::borrow::Cow;
use tracing::{error, warn};
use websocket_builder::{InboundContext, SendMessage};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum Error {
    #[snafu(display("{message}"))]
    Notice {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Auth required: {message}"))]
    AuthRequired {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Restricted: {message}"))]
    Restricted {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("duplicate: {message}"))]
    Duplicate {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Internal error: {message}"))]
    Internal {
        message: String,
        backtrace: Backtrace,
    },

    #[snafu(display("Nostr SDK error: {message}"))]
    NostrSdk {
        message: String,
        backtrace: Backtrace,
    },
}

impl Error {
    pub fn notice<S: Into<String>>(message: S) -> Self {
        Error::Notice {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    pub fn nostr_sdk(message: impl Into<String>) -> Self {
        Error::NostrSdk {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    pub fn auth_required<S: Into<String>>(message: S) -> Self {
        Error::AuthRequired {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    pub fn restricted<S: Into<String>>(message: S) -> Self {
        Error::Restricted {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    pub fn duplicate<S: Into<String>>(message: S) -> Self {
        Error::Duplicate {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    pub fn internal<S: Into<String>>(message: S) -> Self {
        Error::Internal {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }
}

impl From<NostrSdkError> for Error {
    fn from(error: NostrSdkError) -> Self {
        match error {
            NostrSdkError::EventNotFound(event_id) => {
                Error::nostr_sdk(format!("Event not found: {event_id}"))
            }
            NostrSdkError::ImpossibleToZap(message) => Error::nostr_sdk(message),
            NostrSdkError::GossipFiltersEmpty => {
                Error::nostr_sdk("Gossip broken down filters are empty")
            }
            NostrSdkError::Relay(relay_error) => Error::nostr_sdk(relay_error.to_string()),
            NostrSdkError::Database(database_error) => Error::nostr_sdk(database_error.to_string()),
            NostrSdkError::NIP59(nip59_error) => Error::nostr_sdk(nip59_error.to_string()),
            NostrSdkError::RelayPool(relay_pool_error) => {
                Error::nostr_sdk(relay_pool_error.to_string())
            }
            NostrSdkError::Signer(signer_error) => Error::nostr_sdk(signer_error.to_string()),
            NostrSdkError::EventBuilder(event_builder_error) => {
                Error::nostr_sdk(event_builder_error.to_string())
            }
            NostrSdkError::Json(json_error) => Error::nostr_sdk(json_error.to_string()),
            NostrSdkError::SharedState(state_error) => Error::nostr_sdk(state_error.to_string()),
            _ => Error::nostr_sdk(format!("Unhandled Nostr SDK error: {:?}", error)),
        }
    }
}

impl From<DatabaseError> for Error {
    fn from(error: DatabaseError) -> Self {
        Error::Internal {
            message: format!("Database error: {}", error),
            backtrace: Backtrace::capture(),
        }
    }
}

pub enum ClientMessageId {
    Event(EventId),
    Subscription(SubscriptionId),
}

impl Error {
    pub fn to_relay_messages_from_subscription_id(
        &self,
        state: &mut NostrConnectionState,
        subscription_id: SubscriptionId,
    ) -> Vec<RelayMessage<'static>> {
        match self {
            Error::Notice { message, .. } => {
                warn!("Notice: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Owned(message.clone()),
                )]
            }
            Error::AuthRequired { message, .. } => {
                warn!("Auth required: {}", message);
                let challenge_event = state.get_challenge_event();
                vec![
                    challenge_event,
                    RelayMessage::closed(subscription_id, Cow::Owned(message.clone())),
                ]
            }
            Error::Restricted { message, .. } => {
                warn!("Restricted: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Owned(message.clone()),
                )]
            }
            Error::Duplicate { message, .. } => {
                warn!("Duplicate: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Owned(message.clone()),
                )]
            }
            Error::Internal { message, .. } => {
                error!("Internal error: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Owned("Internal error".to_string()),
                )]
            }
            Error::NostrSdk { message, .. } => {
                error!("Nostr SDK error: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Owned("Nostr SDK error".to_string()),
                )]
            }
        }
    }

    pub fn to_relay_messages_from_event(
        &self,
        state: &mut NostrConnectionState,
        event_id: EventId,
    ) -> Vec<RelayMessage<'static>> {
        match self {
            Error::Notice { message, .. } => {
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Owned(message.clone()),
                )]
            }
            Error::AuthRequired { message, .. } => {
                let challenge_event = state.get_challenge_event();
                let msg = format!("auth-required: {}", message);
                vec![
                    challenge_event,
                    RelayMessage::ok(event_id, false, Cow::Owned(msg)),
                ]
            }
            Error::Restricted { message, .. } => {
                let msg = format!("restricted: {}", message);
                vec![RelayMessage::ok(event_id, false, Cow::Owned(msg))]
            }
            Error::Duplicate { message, .. } => {
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Owned(message.clone()),
                )]
            }
            Error::Internal { message, .. } => {
                error!("Internal error: {}", message);
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Owned("error: Internal error".to_string()),
                )]
            }
            Error::NostrSdk { message, .. } => {
                error!("Nostr SDK error: {}", message);
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Owned("error: Internal error".to_string()),
                )]
            }
        }
    }

    pub async fn handle_inbound_error<CM>(
        &self,
        ctx: &mut InboundContext<NostrConnectionState, CM, RelayMessage<'static>>,
        client_message_id: ClientMessageId,
    ) -> Result<()>
    where
        CM: Send + Sync + 'static,
    {
        let relay_messages: Vec<RelayMessage<'static>> = {
            let mut state = ctx.state.write().await;
            match client_message_id {
                ClientMessageId::Event(event_id) => {
                    self.to_relay_messages_from_event(&mut state, event_id)
                }
                ClientMessageId::Subscription(subscription_id) => {
                    self.to_relay_messages_from_subscription_id(&mut state, subscription_id)
                }
            }
        };

        for msg in relay_messages {
            if let Err(e) = ctx.send_message(msg) {
                error!("Failed to send error message: {:?}", e);
            }
        }
        Ok(())
    }
}
