use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use websocket_builder::{InboundContext, Middleware, SendMessage};

#[derive(Debug)]
pub struct Nip70Middleware;

#[async_trait]
impl Middleware for Nip70Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let ClientMessage::Event(event) = &ctx.message else {
            return ctx.next().await;
        };

        if event.tags.find_standardized(TagKind::Protected).is_none() {
            return ctx.next().await;
        }

        let Some(auth_pubkey) = ctx.state.authed_pubkey else {
            return ctx
                .send_message(RelayMessage::ok(
                    event.id,
                    false,
                    "auth-required: this event may only be published by its author",
                ))
                .await;
        };

        if auth_pubkey != event.pubkey {
            return ctx
                .send_message(RelayMessage::ok(
                    event.id,
                    false,
                    "rejected: this event may only be published by its author",
                ))
                .await;
        }

        ctx.next().await
    }
}
