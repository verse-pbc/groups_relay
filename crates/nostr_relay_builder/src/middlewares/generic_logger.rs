//! Generic request/response logging middleware

use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::marker::PhantomData;
use std::time::Instant;
use tracing::{debug, info, info_span};
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, Middleware, OutboundContext,
};

/// Generic middleware that logs all incoming and outgoing messages
#[derive(Debug)]
pub struct GenericLoggerMiddleware<T> {
    metrics_handler: Option<Box<dyn super::LoggerMetricsHandler>>,
    _phantom: PhantomData<T>,
}

impl<T> Default for GenericLoggerMiddleware<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> GenericLoggerMiddleware<T> {
    pub fn new() -> Self {
        Self {
            metrics_handler: None,
            _phantom: PhantomData,
        }
    }

    pub fn with_metrics(metrics_handler: Box<dyn super::LoggerMetricsHandler>) -> Self {
        Self {
            metrics_handler: Some(metrics_handler),
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<T> Middleware for GenericLoggerMiddleware<T>
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
                let start_time = Instant::now();

                info!("> EVENT kind {}: {}", event_kind_u16, event_json);

                ctx.state.event_start_time = Some(start_time);
            }
            Some(ClientMessage::Req {
                subscription_id,
                filter,
            }) => {
                info!(
                    "> REQ {}: {}",
                    subscription_id,
                    serde_json::to_string(&filter).unwrap_or_default()
                );
            }
            Some(ClientMessage::ReqMultiFilter {
                subscription_id,
                filters,
            }) => {
                info!(
                    "> REQ_MULTI {}: {:?}",
                    subscription_id,
                    filters
                        .iter()
                        .map(|f| serde_json::to_string(f).unwrap_or_default())
                        .collect::<Vec<_>>()
                );
            }
            Some(ClientMessage::Close(subscription_id)) => {
                info!("> CLOSE {}", subscription_id);
            }
            Some(ClientMessage::Auth(auth)) => {
                let sig_slice = &auth.sig.to_string()[0..16];
                info!("> AUTH {}...{}", &auth.id.to_string()[0..16], sig_slice);
            }
            _ => {
                debug!("> OTHER: {:?}", ctx.message);
            }
        }

        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        match ctx.message.as_ref() {
            Some(RelayMessage::Event {
                subscription_id,
                event,
            }) => {
                let event_kind_u16 = event.as_ref().kind.as_u16();
                info!(
                    "< EVENT {} kind {}: {}",
                    subscription_id,
                    event_kind_u16,
                    &event.as_ref().id.to_string()[0..16]
                );
            }
            Some(RelayMessage::Ok {
                event_id,
                status,
                message,
            }) => {
                if let Some(start_time) = ctx.state.event_start_time.take() {
                    let latency = start_time.elapsed();
                    let latency_ms = latency.as_secs_f64() * 1000.0;

                    // Extract kind from event_id if possible
                    if let Some(handler) = &self.metrics_handler {
                        handler.record_event_latency(0, latency_ms);
                    }

                    info!(
                        "< OK {} {} {} ({}ms)",
                        &event_id.to_string()[0..16],
                        status,
                        message,
                        latency_ms
                    );
                } else {
                    info!(
                        "< OK {} {} {}",
                        &event_id.to_string()[0..16],
                        status,
                        message
                    );
                }
            }
            Some(RelayMessage::Notice(message)) => {
                info!("< NOTICE: {}", message);
            }
            Some(RelayMessage::EndOfStoredEvents(subscription_id)) => {
                info!("< EOSE {}", subscription_id);
            }
            _ => {
                debug!("< OTHER: {:?}", ctx.message);
            }
        }

        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let subdomain = match &ctx.state.subdomain {
            Scope::Named { name, .. } => Some(name.as_str()),
            Scope::Default => None,
        };

        info!(
            "WebSocket connected from {} to subdomain {:?}",
            ctx.connection_id, subdomain
        );

        if let Some(handler) = &self.metrics_handler {
            handler.increment_active_connections();
        }

        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let subdomain = match &ctx.state.subdomain {
            Scope::Named { name, .. } => Some(name.as_str()),
            Scope::Default => None,
        };

        info!(
            "WebSocket disconnected from {} (subdomain: {:?})",
            ctx.connection_id, subdomain
        );

        if let Some(handler) = &self.metrics_handler {
            handler.decrement_active_connections();
        }

        ctx.next().await
    }
}
