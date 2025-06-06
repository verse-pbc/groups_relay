//! Error handling middleware

use crate::error::Error;
use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::borrow::Cow;
use tracing::error;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

/// Message ID for error handling
#[derive(Debug, Clone)]
pub enum ClientMessageId {
    Event(EventId),
    Subscription(String),
}

/// Middleware for handling errors in the message processing chain
#[derive(Debug)]
pub struct ErrorHandlingMiddleware<T = ()> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T> ErrorHandlingMiddleware<T> {
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> Default for ErrorHandlingMiddleware<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<T: Clone + Send + Sync + std::fmt::Debug + 'static> Middleware for ErrorHandlingMiddleware<T> {
    type State = NostrConnectionState<T>;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let client_message_id = match &ctx.message {
            Some(ClientMessage::Event(event)) => ClientMessageId::Event(event.id),
            Some(ClientMessage::Req {
                subscription_id, ..
            }) => ClientMessageId::Subscription(subscription_id.to_string()),
            Some(ClientMessage::ReqMultiFilter {
                subscription_id, ..
            }) => ClientMessageId::Subscription(subscription_id.to_string()),
            Some(ClientMessage::Close(subscription_id)) => {
                ClientMessageId::Subscription(subscription_id.to_string())
            }
            Some(ClientMessage::Auth(auth)) => ClientMessageId::Event(auth.id),
            _ => {
                error!("Skipping unhandled client message: {:?}", ctx.message);
                return Ok(());
            }
        };

        match ctx.next().await {
            Ok(()) => Ok(()),
            Err(e) => {
                if let Some(err) = e.downcast_ref::<Error>() {
                    if let Err(err) = handle_inbound_error(err, ctx, client_message_id).await {
                        error!("Failed to handle inbound error: {}", err);
                    }
                } else {
                    error!("Unhandled error in middleware chain: {}", e);
                }
                Ok(())
            }
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }
}

/// Handle inbound errors by sending appropriate relay messages
async fn handle_inbound_error<T: Clone + Send + Sync + std::fmt::Debug + 'static>(
    error: &Error,
    ctx: &mut InboundContext<
        '_,
        NostrConnectionState<T>,
        ClientMessage<'static>,
        RelayMessage<'static>,
    >,
    client_message_id: ClientMessageId,
) -> Result<(), anyhow::Error> {
    use Error::*;

    let message = match error {
        Notice { message, .. } => RelayMessage::notice(message.clone()),
        EventError {
            message, event_id, ..
        } => RelayMessage::Ok {
            event_id: *event_id,
            status: false,
            message: format!("error: {}", message).into(),
        },
        SubscriptionError {
            message,
            subscription_id,
            ..
        } => RelayMessage::Closed {
            subscription_id: Cow::Owned(SubscriptionId::new(subscription_id)),
            message: format!("error: {}", message).into(),
        },
        AuthRequired { message, .. } => {
            // For auth errors, use the auth-required prefix as per NIP-42
            match client_message_id {
                ClientMessageId::Event(event_id) => RelayMessage::Ok {
                    event_id,
                    status: false,
                    message: format!("auth-required: {}", message).into(),
                },
                ClientMessageId::Subscription(subscription_id) => RelayMessage::Closed {
                    subscription_id: Cow::Owned(SubscriptionId::new(subscription_id)),
                    message: format!("auth-required: {}", message).into(),
                },
            }
        }
        Restricted { message, .. } => {
            // For restricted access errors, use the restricted prefix as per NIP-42
            match client_message_id {
                ClientMessageId::Event(event_id) => RelayMessage::Ok {
                    event_id,
                    status: false,
                    message: format!("restricted: {}", message).into(),
                },
                ClientMessageId::Subscription(subscription_id) => RelayMessage::Closed {
                    subscription_id: Cow::Owned(SubscriptionId::new(subscription_id)),
                    message: format!("restricted: {}", message).into(),
                },
            }
        }
        _ => {
            // For other error types, use generic error prefix
            match client_message_id {
                ClientMessageId::Event(event_id) => RelayMessage::Ok {
                    event_id,
                    status: false,
                    message: format!("error: {}", error).into(),
                },
                ClientMessageId::Subscription(subscription_id) => RelayMessage::Closed {
                    subscription_id: Cow::Owned(SubscriptionId::new(subscription_id)),
                    message: format!("error: {}", error).into(),
                },
            }
        }
    };

    ctx.send_message(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_message_id_event() {
        let event_id = EventId::all_zeros();
        let message_id = ClientMessageId::Event(event_id);

        match message_id {
            ClientMessageId::Event(id) => assert_eq!(id, event_id),
            _ => panic!("Expected Event variant"),
        }
    }

    #[test]
    fn test_client_message_id_subscription() {
        let subscription_id = "test_sub".to_string();
        let message_id = ClientMessageId::Subscription(subscription_id.clone());

        match message_id {
            ClientMessageId::Subscription(id) => assert_eq!(id, subscription_id),
            _ => panic!("Expected Subscription variant"),
        }
    }

    #[test]
    fn test_client_message_id_debug_impl() {
        let event_id = EventId::all_zeros();
        let msg_id = ClientMessageId::Event(event_id);
        let debug_str = format!("{:?}", msg_id);
        assert!(debug_str.contains("Event"));

        let sub_id = ClientMessageId::Subscription("test".to_string());
        let debug_str = format!("{:?}", sub_id);
        assert!(debug_str.contains("Subscription"));
    }

    #[test]
    fn test_error_handling_middleware_debug_impl() {
        let middleware = ErrorHandlingMiddleware::<()>::new();
        let debug_str = format!("{:?}", middleware);
        assert!(debug_str.contains("ErrorHandlingMiddleware"));
    }

    #[test]
    fn test_error_message_formatting() {
        let event_id = EventId::all_zeros();

        // Test notice error
        let notice_error = Error::notice("Test notice");
        match &notice_error {
            Error::Notice { message, .. } => {
                assert_eq!(message, "Test notice");
            }
            _ => panic!("Expected Notice error"),
        }

        // Test event error
        let event_error = Error::event_error("Invalid signature", event_id);
        match &event_error {
            Error::EventError {
                message,
                event_id: id,
                ..
            } => {
                assert_eq!(message, "Invalid signature");
                assert_eq!(*id, event_id);
            }
            _ => panic!("Expected EventError"),
        }

        // Test subscription error
        let sub_error = Error::subscription_error("Invalid filter", "sub1");
        match &sub_error {
            Error::SubscriptionError {
                message,
                subscription_id,
                ..
            } => {
                assert_eq!(message, "Invalid filter");
                assert_eq!(subscription_id, "sub1");
            }
            _ => panic!("Expected SubscriptionError"),
        }

        // Test unauthorized error
        let auth_error = Error::auth_required("Authentication required");
        match &auth_error {
            Error::AuthRequired { message, .. } => {
                assert_eq!(message, "Authentication required");
            }
            _ => panic!("Expected AuthRequired error"),
        }

        // Test restricted error
        let restricted_error = Error::restricted("Access denied");
        match &restricted_error {
            Error::Restricted { message, .. } => {
                assert_eq!(message, "Access denied");
            }
            _ => panic!("Expected Restricted error"),
        }
    }

    #[test]
    fn test_relay_message_creation_for_errors() {
        let event_id = EventId::all_zeros();

        // Test OK message creation for event errors
        let ok_msg = RelayMessage::Ok {
            event_id,
            status: false,
            message: "error: Invalid signature".into(),
        };

        match ok_msg {
            RelayMessage::Ok {
                event_id: id,
                status,
                message,
            } => {
                assert_eq!(id, event_id);
                assert!(!status);
                assert_eq!(message, "error: Invalid signature");
            }
            _ => panic!("Expected OK message"),
        }

        // Test CLOSED message creation for subscription errors
        let closed_msg = RelayMessage::Closed {
            subscription_id: Cow::Owned(SubscriptionId::new("sub1")),
            message: "error: Invalid filter".into(),
        };

        match closed_msg {
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                assert_eq!(subscription_id.as_str(), "sub1");
                assert_eq!(message, "error: Invalid filter");
            }
            _ => panic!("Expected Closed message"),
        }

        // Test NOTICE message creation
        let notice_msg = RelayMessage::notice("Test notice");
        match notice_msg {
            RelayMessage::Notice(message) => {
                assert_eq!(message, "Test notice");
            }
            _ => panic!("Expected Notice message"),
        }
    }

    #[test]
    fn test_error_prefix_formatting() {
        // Test auth-required prefix
        let auth_msg = format!("auth-required: {}", "User not authenticated");
        assert!(auth_msg.starts_with("auth-required: "));
        assert!(auth_msg.contains("User not authenticated"));

        // Test restricted prefix
        let restricted_msg = format!("restricted: {}", "Access denied to group");
        assert!(restricted_msg.starts_with("restricted: "));
        assert!(restricted_msg.contains("Access denied to group"));

        // Test generic error prefix
        let error_msg = format!("error: {}", "Database connection failed");
        assert!(error_msg.starts_with("error: "));
        assert!(error_msg.contains("Database connection failed"));
    }
}
