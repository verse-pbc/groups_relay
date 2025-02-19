use crate::nostr_session_state::NostrConnectionState;
use nostr_sdk::client::Error as NostrSdkError;
use nostr_sdk::prelude::*;
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
            NostrSdkError::DMsRelaysNotFound => Error::nostr_sdk("DMs relays not found"),
            NostrSdkError::MetadataNotFound => Error::nostr_sdk("Metadata not found"),
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
        }
    }
}

impl Error {
    pub fn to_relay_messages_from_subscription_id(
        &self,
        state: &mut NostrConnectionState,
        subscription_id: SubscriptionId,
    ) -> Vec<RelayMessage> {
        match self {
            Error::Notice { message, .. } => {
                warn!("Notice: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Borrowed(message.as_str()),
                )]
            }
            Error::AuthRequired { message, .. } => {
                warn!("Auth required: {}", message);
                let challenge_event = state.get_challenge_event();
                vec![
                    challenge_event,
                    RelayMessage::closed(subscription_id, Cow::Borrowed(message.as_str())),
                ]
            }
            Error::Restricted { message, .. } => {
                warn!("Restricted: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Borrowed(message.as_str()),
                )]
            }
            Error::Internal { message, .. } => {
                error!("Internal error: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Borrowed("Internal error"),
                )]
            }
            Error::NostrSdk { message, .. } => {
                error!("Nostr SDK error: {}", message);
                vec![RelayMessage::closed(
                    subscription_id,
                    Cow::Borrowed("Nostr SDK error"),
                )]
            }
        }
    }

    pub fn to_relay_messages_from_event(
        &self,
        state: &mut NostrConnectionState,
        event_id: EventId,
    ) -> Vec<RelayMessage> {
        match self {
            Error::Notice { message, .. } => {
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Borrowed(message.as_str()),
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
            Error::Internal { message, .. } => {
                error!("Internal error: {}", message);
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Borrowed("error: Internal error"),
                )]
            }
            Error::NostrSdk { message, .. } => {
                error!("Nostr SDK error: {}", message);
                vec![RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Borrowed("error: Internal error"),
                )]
            }
        }
    }

    pub async fn handle_inbound_error(
        &self,
        ctx: &mut InboundContext<'_, NostrConnectionState, ClientMessage, RelayMessage>,
    ) -> Result<()> {
        let relay_messages = match &ctx.message {
            ClientMessage::Event(event) => self.to_relay_messages_from_event(ctx.state, event.id),
            ClientMessage::Req {
                subscription_id, ..
            } => self.to_relay_messages_from_subscription_id(ctx.state, subscription_id.clone()),
            ClientMessage::Close(subscription_id) => {
                self.to_relay_messages_from_subscription_id(ctx.state, subscription_id.clone())
            }
            ClientMessage::Auth(auth) => self.to_relay_messages_from_event(ctx.state, auth.id),
            _ => {
                error!("{}", self);
                return Ok(());
            }
        };

        for msg in relay_messages {
            if let Err(e) = ctx.send_message(msg).await {
                error!("Failed to send error message: {:?}", e);
            }
        }
        Ok(())
    }
}
