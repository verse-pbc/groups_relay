use crate::metrics;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::{ClientMessage, JsonUtil, RelayMessage};
use tracing::info;
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext,
};

#[derive(Debug)]
pub struct LoggerMiddleware;

impl LoggerMiddleware {
    pub fn new() -> Self {
        Self
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

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        match &ctx.message {
            ClientMessage::Event(event) => {
                info!(
                    "[{}] > event kind {}: {}",
                    ctx.connection_id.as_str(),
                    event.kind,
                    event.as_json()
                );
            }
            ClientMessage::Req {
                subscription_id,
                filters,
            } => {
                info!(
                    "[{}] > request {}: {}",
                    ctx.connection_id.as_str(),
                    subscription_id,
                    filters
                        .iter()
                        .map(|f| f.as_json())
                        .collect::<Vec<String>>()
                        .join(", ")
                );
            }
            ClientMessage::Auth(event) => {
                info!("[{}] > auth: {}", ctx.connection_id.as_str(), event.id);
            }
            ClientMessage::Close(subscription_id) => {
                info!(
                    "[{}] > close: {}",
                    ctx.connection_id.as_str(),
                    subscription_id
                );
            }
            _ => {
                info!(
                    "[{}] > {}",
                    ctx.connection_id.as_str(),
                    ctx.message.as_json()
                );
            }
        }
        Ok(())
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
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
            RelayMessage::Auth { challenge } => {
                info!("[{}] < auth: {}", ctx.connection_id.as_str(), challenge);
            }
            RelayMessage::Ok {
                event_id,
                status: _,
                message,
            } => {
                info!(
                    "[{}] < ok: {}, {}",
                    ctx.connection_id.as_str(),
                    event_id,
                    message
                );
            }
            _ => {
                info!(
                    "[{}] < {}",
                    ctx.connection_id.as_str(),
                    outbound_message.as_json()
                );
            }
        }
        Ok(())
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("[{}] Connected to relay", ctx.connection_id.as_str());
        metrics::active_connections().increment(1.0);
        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("[{}] Disconnected from relay", ctx.connection_id.as_str());
        metrics::active_connections().decrement(1.0);
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::SubscriptionId;
    use std::sync::Arc;

    fn create_test_state() -> NostrConnectionState {
        NostrConnectionState::new("wss://test.relay".to_string())
    }

    #[tokio::test]
    async fn test_inbound_message_logging() {
        let middleware = LoggerMiddleware::new();
        let chain: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware)];
        let mut state = create_test_state();

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            ClientMessage::Close(SubscriptionId::new("test_sub")),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_outbound_message_logging() {
        let middleware = LoggerMiddleware::new();
        let chain: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware)];
        let mut state = create_test_state();

        let mut ctx = OutboundContext::new(
            "test_connection".to_string(),
            RelayMessage::Notice {
                message: "test notice".to_string(),
            },
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_outbound(&mut ctx).await;
        assert!(result.is_ok());
    }
}
