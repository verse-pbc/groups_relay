use crate::error::{ClientMessageId, Error};
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::error;
use websocket_builder::{InboundContext, Middleware, OutboundContext};

#[derive(Debug)]
pub struct ErrorHandlingMiddleware;

#[async_trait]
impl Middleware for ErrorHandlingMiddleware {
    type State = NostrConnectionState;
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
            }) => ClientMessageId::Subscription(subscription_id.as_ref().clone()),
            Some(ClientMessage::ReqMultiFilter {
                subscription_id, ..
            }) => ClientMessageId::Subscription(subscription_id.as_ref().clone()),
            Some(ClientMessage::Close(subscription_id)) => {
                ClientMessageId::Subscription(subscription_id.as_ref().clone())
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
                    if let Err(err) = err.handle_inbound_error(ctx, client_message_id).await {
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
