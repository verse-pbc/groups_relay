use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::{ClientMessage, RelayMessage};
use websocket_builder::{InboundContext, Middleware, SendMessage};

#[derive(Debug)]
pub struct EventVerifierMiddleware;

#[async_trait]
impl Middleware for EventVerifierMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        match &ctx.message {
            ClientMessage::Event(event) => {
                if event.verify().is_err() {
                    ctx.send_message(RelayMessage::ok(
                        event.id,
                        false,
                        "invalid: event signature verification failed",
                    ))
                    .await
                } else {
                    ctx.next().await
                }
            }
            _ => ctx.next().await,
        }
    }
}
