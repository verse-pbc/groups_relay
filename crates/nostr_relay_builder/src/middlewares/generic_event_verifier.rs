//! Generic event signature verification middleware

use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::marker::PhantomData;
use tracing::warn;
use websocket_builder::{InboundContext, Middleware, SendMessage};

/// Generic middleware that verifies event signatures
#[derive(Debug)]
pub struct GenericEventVerifierMiddleware<T> {
    _phantom: PhantomData<T>,
}

impl<T> Default for GenericEventVerifierMiddleware<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> GenericEventVerifierMiddleware<T> {
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<T> Middleware for GenericEventVerifierMiddleware<T>
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
        if let Some(ClientMessage::Event(event)) = ctx.message.as_ref() {
            // Verify the event signature
            match event.verify() {
                Ok(_) => {
                    // Event is valid, continue processing
                    ctx.next().await
                }
                Err(e) => {
                    warn!(
                        "Event {} has invalid signature from {}: {}",
                        event.id.to_hex(),
                        event.pubkey.to_hex(),
                        e
                    );

                    // Send error response
                    ctx.send_message(RelayMessage::Ok {
                        event_id: event.id,
                        status: false,
                        message: format!("invalid: signature verification failed: {}", e).into(),
                    })?;

                    // Don't continue processing this event
                    Ok(())
                }
            }
        } else {
            // Not an event message, continue processing
            ctx.next().await
        }
    }
}
