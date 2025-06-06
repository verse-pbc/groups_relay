//! Generic error handling middleware

use crate::error::Error;
use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::borrow::Cow;
use std::marker::PhantomData;
use tracing::error;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

use super::ClientMessageId;

/// Generic middleware for handling errors in the message processing chain
#[derive(Debug)]
pub struct GenericErrorHandlingMiddleware<T> {
    _phantom: PhantomData<T>,
}

impl<T> Default for GenericErrorHandlingMiddleware<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> GenericErrorHandlingMiddleware<T> {
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<T> Middleware for GenericErrorHandlingMiddleware<T>
where
    T: Send + Sync + std::fmt::Debug + 'static,
{
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
        if let Err(e) = ctx.next().await {
            error!("Error in outbound middleware chain: {}", e);
        }
        Ok(())
    }
}

/// Handle inbound errors by sending appropriate relay messages
async fn handle_inbound_error<T>(
    err: &Error,
    ctx: &mut InboundContext<
        '_,
        NostrConnectionState<T>,
        ClientMessage<'static>,
        RelayMessage<'static>,
    >,
    client_message_id: ClientMessageId,
) -> Result<(), anyhow::Error>
where
    T: Send + Sync + 'static,
{
    match client_message_id {
        ClientMessageId::Event(event_id) => {
            let message = match err {
                Error::AuthRequired { .. } => Cow::from(format!("auth-required: {}", err)),
                Error::Restricted { .. } => Cow::from(format!("restricted: {}", err)),
                Error::Internal { .. } => Cow::from(format!("error: {}", err)),
                _ => Cow::from(err.to_string()),
            };

            ctx.send_message(RelayMessage::Ok {
                event_id,
                status: false,
                message,
            })?;
        }
        ClientMessageId::Subscription(subscription_id) => {
            let message = match err {
                Error::AuthRequired { .. } => format!(
                    "auth-required: subscription {} requires authentication",
                    subscription_id
                ),
                Error::Restricted { .. } => {
                    format!("restricted: subscription {} not allowed", subscription_id)
                }
                Error::Internal { .. } => {
                    format!("internal: subscription {} failed", subscription_id)
                }
                _ => format!("error: subscription {} - {}", subscription_id, err),
            };

            ctx.send_message(RelayMessage::Notice(message.clone().into()))?;

            // Convert subscription_id string to SubscriptionId
            let sub_id = SubscriptionId::new(subscription_id);
            ctx.send_message(RelayMessage::Closed {
                subscription_id: Cow::Owned(sub_id),
                message: err.to_string().into(),
            })?;
        }
    }

    Ok(())
}
