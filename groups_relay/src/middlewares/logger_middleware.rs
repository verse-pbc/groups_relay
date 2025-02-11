use crate::metrics;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::{ClientMessage, JsonUtil, RelayMessage};
use tracing::{debug, info};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext,
};

#[derive(Debug)]
pub struct LoggerMiddleware;

impl Default for LoggerMiddleware {
    fn default() -> Self {
        Self
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
    ) -> Result<(), anyhow::Error> {
        match &ctx.message {
            ClientMessage::Event(event) => {
                info!("> event kind {}: {}", event.kind, event.as_json());
            }
            ClientMessage::Req {
                filter,
                subscription_id,
            } => {
                info!("> REQ {}: {}", subscription_id, filter.as_json());
            }
            ClientMessage::Close(subscription_id) => {
                info!("> CLOSE {}", subscription_id);
            }
            ClientMessage::Auth(event) => {
                info!("> AUTH {}", event.as_json());
            }
            _ => debug!("> {:?}", ctx.message),
        }
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(msg) = &ctx.message {
            info!("< {}", msg.as_json());
        }
        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("Connected to relay");
        metrics::active_connections().increment(1.0);
        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("Disconnected from relay");
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
        NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL")
    }

    fn create_middleware_chain() -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage,
                OutgoingMessage = RelayMessage,
            >,
        >,
    > {
        vec![Arc::new(LoggerMiddleware)]
    }

    #[tokio::test]
    async fn test_inbound_message_logging() {
        let chain = create_middleware_chain();
        let mut state = create_test_state();

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            ClientMessage::Close(SubscriptionId::new("test_sub")),
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_outbound_message_logging() {
        let chain = create_middleware_chain();
        let mut state = create_test_state();

        let mut ctx = OutboundContext::new(
            "test_connection".to_string(),
            RelayMessage::Notice("test notice".to_string()),
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        let result = chain[0].process_outbound(&mut ctx).await;
        assert!(result.is_ok());
    }
}
