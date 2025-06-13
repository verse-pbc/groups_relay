//! Request/response logging middleware

use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use tracing::{debug, info, info_span};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext,
};

/// Middleware that logs all incoming and outgoing messages
#[derive(Debug)]
pub struct LoggerMiddleware<T = ()> {
    _phantom: std::marker::PhantomData<T>,
}

impl<T> Default for LoggerMiddleware<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LoggerMiddleware<T> {
    pub fn new() -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T: Clone + Send + Sync + std::fmt::Debug + 'static> Middleware for LoggerMiddleware<T> {
    type State = NostrConnectionState<T>;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        // Extract subdomain from connection state
        let subdomain = match &ctx.state.subdomain {
            Scope::Named { name, .. } => Some(name.as_str()),
            Scope::Default => None,
        };

        // Create a span with connection ID and subdomain to ensure logs always have context
        let connection_span = info_span!(
            parent: None,
            "websocket_connection",
            ip = %ctx.connection_id,
            subdomain = ?subdomain
        );
        let _guard = connection_span.enter();

        match ctx.message.as_ref() {
            Some(ClientMessage::Event(event)) => {
                let event_kind_u16 = event.as_ref().kind.as_u16();
                let event_json = event.as_ref().as_json();

                debug!("> EVENT kind {}: {}", event_kind_u16, event_json);
            }
            Some(ClientMessage::Req {
                subscription_id,
                filter,
            }) => {
                let sub_id_clone = subscription_id.clone();
                let filter_json_clone = filter.as_json();

                debug!("> REQ {}: {}", sub_id_clone, filter_json_clone);
            }
            Some(ClientMessage::ReqMultiFilter {
                subscription_id,
                filters,
            }) => {
                let sub_id_clone = subscription_id.clone();
                let filters_json_clone =
                    filters.iter().map(|f| f.as_json()).collect::<Vec<String>>();
                debug!("> REQ {}: {:?}", sub_id_clone, filters_json_clone);
            }
            Some(ClientMessage::Close(subscription_id)) => {
                let sub_id_clone = subscription_id.clone();
                debug!("> CLOSE {}", sub_id_clone);
            }
            Some(ClientMessage::Auth(event)) => {
                debug!("> AUTH {}", event.as_ref().as_json());
            }
            _ => {
                let message_clone_for_debug = format!("{:?}", ctx.message);
                debug!("> {}", message_clone_for_debug);
            }
        }

        // Continue with the middleware chain
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        // Extract subdomain from connection state
        let subdomain = match &ctx.state.subdomain {
            Scope::Named { name, .. } => Some(name.as_str()),
            Scope::Default => None,
        };

        // Create a span with connection ID and subdomain to ensure logs always have context
        let connection_span = info_span!(
            parent: None,
            "websocket_connection",
            ip = %ctx.connection_id,
            subdomain = ?subdomain
        );
        let _guard = connection_span.enter();

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

                    debug!("< OK {} {} {}", event_id_clone, status_clone, message_clone);
                }
                RelayMessage::Event {
                    subscription_id,
                    event,
                } => {
                    let sub_id_clone = subscription_id.clone();
                    let event_json_clone = event.as_ref().as_json();

                    debug!("< EVENT {} {}", sub_id_clone, event_json_clone);
                }
                RelayMessage::Notice(message) => {
                    let message_clone = message.clone();
                    debug!("< NOTICE {}", message_clone);
                }
                RelayMessage::EndOfStoredEvents(subscription_id) => {
                    let sub_id_clone = subscription_id.clone();
                    debug!("< EOSE {}", sub_id_clone);
                }
                RelayMessage::Auth { challenge } => {
                    let challenge_clone = challenge.clone();
                    debug!("< AUTH {}", challenge_clone);
                }
                _ => {
                    let msg_clone_for_debug = format!("{:?}", ctx.message);
                    debug!("< {}", msg_clone_for_debug);
                }
            }
        }

        // Continue with the middleware chain
        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        // Extract subdomain from connection state
        let subdomain = match &ctx.state.subdomain {
            Scope::Named { name, .. } => Some(name.as_str()),
            Scope::Default => None,
        };

        // Create a span with connection ID and subdomain to ensure logs always have context
        let connection_span = info_span!(
            parent: None,
            "websocket_connection",
            ip = %ctx.connection_id,
            subdomain = ?subdomain
        );
        let _guard = connection_span.enter();

        info!("Disconnected from relay");

        // Continue with the middleware chain
        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        // Extract subdomain from connection state
        let subdomain = match &ctx.state.subdomain {
            Scope::Named { name, .. } => Some(name.as_str()),
            Scope::Default => None,
        };

        // Create a span with connection ID and subdomain to ensure logs always have context
        let connection_span = info_span!(
            parent: None,
            "websocket_connection",
            ip = %ctx.connection_id,
            subdomain = ?subdomain
        );
        let _guard = connection_span.enter();

        debug!("Connected to relay");

        // Continue with the middleware chain
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_state() -> NostrConnectionState<()> {
        NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL")
    }

    fn create_middleware_chain() -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState<()>,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > {
        vec![Arc::new(LoggerMiddleware::<()>::new())]
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
