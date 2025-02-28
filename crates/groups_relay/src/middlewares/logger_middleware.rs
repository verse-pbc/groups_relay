use crate::metrics;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::time::Instant;
use tracing::{debug, info};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext,
};

#[derive(Debug)]
pub struct LoggerMiddleware;

impl Default for LoggerMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl LoggerMiddleware {
    pub fn new() -> Self {
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
            Some(ClientMessage::Event(event)) => {
                let start_time = Instant::now();
                info!("> EVENT kind {}: {}", event.kind.as_u16(), event.as_json());

                // Store start time and kind in state for outbound processing
                ctx.state.event_start_time = Some(start_time);
                ctx.state.event_kind = Some(event.kind.as_u16());
                ctx.next().await
            }
            Some(ClientMessage::Req {
                subscription_id,
                filter,
            }) => {
                info!("> REQ {}: {}", subscription_id, filter.as_json());
                ctx.next().await
            }
            Some(ClientMessage::ReqMultiFilter {
                subscription_id,
                filters,
            }) => {
                info!(
                    "> REQ {}: {:?}",
                    subscription_id,
                    filters.iter().map(|f| f.as_json()).collect::<Vec<String>>()
                );
                ctx.next().await
            }
            Some(ClientMessage::Close(subscription_id)) => {
                info!("> CLOSE {}", subscription_id);
                ctx.next().await
            }
            Some(ClientMessage::Auth(event)) => {
                info!("> AUTH {}", event.as_json());
                ctx.next().await
            }
            _ => {
                debug!("> {:?}", ctx.message);
                ctx.next().await
            }
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(msg) = &ctx.message {
            match msg {
                RelayMessage::Ok {
                    event_id,
                    status,
                    message,
                } => {
                    // Calculate latency if we have a start time
                    if let Some(start_time) = ctx.state.event_start_time.take() {
                        let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;

                        // Get the event kind from the state
                        if let Some(kind) = ctx.state.event_kind.take() {
                            metrics::event_latency(kind as u32).record(latency_ms);
                        }

                        info!(
                            "< OK {} {} {} took {:?}ms",
                            event_id, status, message, latency_ms
                        );
                    } else {
                        info!("< OK {} {} {}", event_id, status, message);
                    }
                }
                RelayMessage::Event {
                    subscription_id,
                    event,
                } => {
                    info!("< EVENT {} {}", subscription_id, event.as_json());
                }
                RelayMessage::Notice(message) => {
                    info!("< NOTICE {}", message);
                }
                RelayMessage::EndOfStoredEvents(subscription_id) => {
                    info!("< EOSE {}", subscription_id);
                }
                RelayMessage::Auth { challenge } => {
                    info!("< AUTH {}", challenge);
                }
                _ => {
                    debug!("< {:?}", msg);
                }
            }
        }
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

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        info!("Connected to relay");
        metrics::active_connections().increment(1.0);
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            Some(ClientMessage::Close(SubscriptionId::new("test_sub"))),
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
