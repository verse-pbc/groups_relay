use crate::error::Error;
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
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        match ctx.next().await {
            Ok(()) => Ok(()),
            Err(e) => {
                if let Some(e) = e.downcast_ref::<Error>() {
                    if let Err(err) = e.handle_inbound_error(ctx).await {
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
