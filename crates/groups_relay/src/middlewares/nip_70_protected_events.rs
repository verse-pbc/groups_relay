use crate::error::Error;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

#[derive(Debug)]
pub struct Nip70Middleware;

#[async_trait]
impl Middleware for Nip70Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, ClientMessage<'static>, RelayMessage<'static>>,
    ) -> Result<(), anyhow::Error> {
        let Some(ClientMessage::Event(event)) = &ctx.message else {
            return ctx.next().await;
        };

        if event.tags.find_standardized(TagKind::Protected).is_none() {
            return ctx.next().await;
        }

        let Some(auth_pubkey) = ctx.state.authed_pubkey else {
            return Err(
                Error::auth_required("this event may only be published by its author").into(),
            );
        };

        if auth_pubkey != event.pubkey {
            return ctx.send_message(RelayMessage::ok(
                event.id,
                false,
                "rejected: this event may only be published by its author",
            ));
        }

        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, ClientMessage<'static>, RelayMessage<'static>>,
    ) -> Result<(), anyhow::Error> {
        if let Some(RelayMessage::Event { event: _, .. }) = &ctx.message {
            // If an auth public key is present, it means the user is authenticated.
            if ctx.state.authed_pubkey.is_some() {
                // ... existing code ...
            }
        }
        ctx.next().await
    }
}
