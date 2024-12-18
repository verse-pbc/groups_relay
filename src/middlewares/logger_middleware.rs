use crate::nostr_session_state::NostrConnectionState;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::{info, warn};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext,
};

#[derive(Debug)]
pub struct LoggerMiddleware;

impl LoggerMiddleware {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for LoggerMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for LoggerMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        match &ctx.message {
            ClientMessage::Event(event) => {
                info!(
                    "[{}] > event kind {}: {}",
                    ctx.connection_id.as_str(),
                    event.kind,
                    event.as_json().replace("\\\\\"", "\\\"")
                );
            }
            ClientMessage::Req {
                subscription_id,
                filters,
                ..
            } => {
                info!(
                    "[{}] > request {}: {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    filters
                        .iter()
                        .map(|f| f.as_json().replace("\\\\\"", "\\\""))
                        .collect::<Vec<String>>()
                        .join(", "),
                );
            }
            ClientMessage::Auth(challenge) => {
                info!("[{}] > auth: {:?}", ctx.connection_id.as_str(), challenge);
            }

            ClientMessage::Count {
                subscription_id,
                filters,
            } => {
                info!(
                    "[{}] > count: {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    filters
                );
            }
            ClientMessage::Close(subscription_id) => {
                info!(
                    "[{}] > close: {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id
                );
            }
            ClientMessage::NegClose { subscription_id } => {
                info!(
                    "[{}] > neg close: {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id
                );
            }
            ClientMessage::NegOpen {
                subscription_id,
                filter,
                id_size,
                initial_message,
            } => {
                info!(
                    "[{}] > neg open: {:?}, {:?}, {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    filter,
                    id_size,
                    initial_message
                );
            }
            ClientMessage::NegMsg {
                subscription_id,
                message,
            } => {
                info!(
                    "[{}] > neg msg: {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    message
                );
            }
        };
        ctx.next().await
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(outbound_message) = &ctx.message else {
            return Ok(());
        };

        match outbound_message {
            RelayMessage::Event {
                subscription_id,
                event,
            } => {
                info!(
                    "[{}] < event for sub {}: kind {}, id: {}, pubkey: {}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    event.kind,
                    event.id,
                    event.pubkey
                );
            }
            RelayMessage::Closed {
                subscription_id,
                message,
            } => {
                info!(
                    "[{}] < closed: {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    message
                );
            }
            RelayMessage::Notice { message } => {
                warn!("[{}] < notice: {:?}", ctx.connection_id.as_str(), message);
            }
            RelayMessage::Ok {
                event_id,
                status,
                message,
            } => {
                if *status {
                    info!(
                        "[{}] < ok: {:?}, {:?}",
                        ctx.connection_id.as_str(),
                        event_id,
                        message
                    );
                } else {
                    warn!(
                        "[{}] < ok: {:?}, {:?}",
                        ctx.connection_id.as_str(),
                        event_id,
                        message
                    );
                }
            }
            RelayMessage::EndOfStoredEvents(subscription_id) => {
                info!(
                    "[{}] < eose for sub {}",
                    ctx.connection_id.as_str(),
                    subscription_id
                );
            }
            RelayMessage::Auth { challenge } => {
                info!("[{}] < auth: {:?}", ctx.connection_id.as_str(), challenge);
            }
            RelayMessage::Count {
                subscription_id,
                count,
            } => {
                info!(
                    "[{}] < count: {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    count
                );
            }
            RelayMessage::NegErr {
                subscription_id,
                code,
            } => {
                warn!(
                    "[{}] < neg err: {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    code
                );
            }
            RelayMessage::NegMsg {
                subscription_id,
                message,
            } => {
                warn!(
                    "[{}] < neg msg: {:?}, {:?}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    message
                );
            }
        };
        ctx.next().await
    }

    async fn on_connect<'a>(
        &'a self,
        ctx: &mut ConnectionContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("[{}] Connected to relay", ctx.connection_id.as_str());
        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("[{}] Disconnected from relay", ctx.connection_id.as_str());
        ctx.next().await
    }
}
