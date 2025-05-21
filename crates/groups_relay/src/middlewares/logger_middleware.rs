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
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        match ctx.message.as_ref() {
            Some(ClientMessage::Event(event)) => {
                let event_kind_u16 = event.as_ref().kind.as_u16();
                let event_json = event.as_ref().as_json();
                let start_time = Instant::now();

                info!("> EVENT kind {}: {}", event_kind_u16, event_json);

                ctx.state.event_start_time = Some(start_time);
                ctx.state.event_kind = Some(event_kind_u16);
                ctx.next().await
            }
            Some(ClientMessage::Req {
                subscription_id,
                filter,
            }) => {
                let sub_id_clone = subscription_id.clone();
                let filter_json_clone = filter.as_json();
                info!("> REQ {}: {}", sub_id_clone, filter_json_clone);
                ctx.next().await
            }
            Some(ClientMessage::ReqMultiFilter {
                subscription_id,
                filters,
            }) => {
                let sub_id_clone = subscription_id.clone();
                let filters_json_clone =
                    filters.iter().map(|f| f.as_json()).collect::<Vec<String>>();
                info!("> REQ {}: {:?}", sub_id_clone, filters_json_clone);
                ctx.next().await
            }
            Some(ClientMessage::Close(subscription_id)) => {
                let sub_id_clone = subscription_id.clone();
                info!("> CLOSE {}", sub_id_clone);
                ctx.next().await
            }
            Some(ClientMessage::Auth(event)) => {
                let event_json_clone = event.as_ref().as_json();
                info!("> AUTH {}", event_json_clone);
                ctx.next().await
            }
            _ => {
                let message_clone_for_debug = format!("{:?}", ctx.message);
                debug!("> {}", message_clone_for_debug);
                ctx.next().await
            }
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(msg_ref) = ctx.message.as_ref() {
            match msg_ref {
                RelayMessage::Ok {
                    event_id,
                    status,
                    message,
                } => {
                    let event_id_clone = *event_id;
                    let status_clone = *status;
                    let message_clone = message.clone();

                    if let Some(start_time) = ctx.state.event_start_time.take() {
                        let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                        if let Some(kind) = ctx.state.event_kind.take() {
                            metrics::event_latency(kind as u32).record(latency_ms);
                        }
                        info!(
                            "< OK {} {} {} took {:?}ms",
                            event_id_clone, status_clone, message_clone, latency_ms
                        );
                    } else {
                        info!("< OK {} {} {}", event_id_clone, status_clone, message_clone);
                    }
                }
                RelayMessage::Event {
                    subscription_id,
                    event,
                } => {
                    let sub_id_clone = subscription_id.clone();
                    let event_json_clone = event.as_ref().as_json();
                    info!("< EVENT {} {}", sub_id_clone, event_json_clone);
                }
                RelayMessage::Notice(message) => {
                    let message_clone = message.clone();
                    info!("< NOTICE {}", message_clone);
                }
                RelayMessage::EndOfStoredEvents(subscription_id) => {
                    let sub_id_clone = subscription_id.clone();
                    info!("< EOSE {}", sub_id_clone);
                }
                RelayMessage::Auth { challenge } => {
                    let challenge_clone = challenge.clone();
                    info!("< AUTH {}", challenge_clone);
                }
                _ => {
                    let msg_clone_for_debug = format!("{:?}", ctx.message);
                    debug!("< {}", msg_clone_for_debug);
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
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
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
            Some(ClientMessage::close(SubscriptionId::new("test_sub"))),
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
            RelayMessage::notice("test notice".to_string()),
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        let result = chain[0].process_outbound(&mut ctx).await;
        assert!(result.is_ok());
    }
}
