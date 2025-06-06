//! RelayMiddleware for processing Nostr messages with optimized performance
//!
//! This module provides middleware that processes Nostr protocol messages while
//! delegating business logic to EventProcessor implementations. The implementation
//! is optimized for zero-allocation in hot paths like subscription processing.

use crate::database::RelayDatabase;
use crate::error::Error;
use crate::event_processor::{EventContext, EventProcessor};
use crate::state::NostrConnectionState;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tracing::{debug, error};
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

/// Relay middleware that processes messages with zero-allocation performance.
///
/// This middleware provides protocol handling (EVENT, REQ, CLOSE, AUTH) while
/// delegating business logic to EventProcessor implementations. The design
/// minimizes allocations in hot paths for maximum performance.
///
/// ## Key Features
///
/// - Zero-allocation event visibility checks during subscription processing
/// - Direct access to custom state without cloning
/// - Type-safe generic state management
/// - Backward compatible with full state access where needed
#[derive(Debug, Clone)]
pub struct RelayMiddleware<P, T = ()>
where
    P: EventProcessor<T>,
    T: Send + Sync + 'static,
{
    processor: Arc<P>,
    relay_pubkey: PublicKey,
    database: Arc<RelayDatabase>,
    _phantom: std::marker::PhantomData<T>,
}

impl<P, T> RelayMiddleware<P, T>
where
    P: EventProcessor<T>,
    T: Clone + Send + Sync + std::fmt::Debug + Default + 'static,
{
    /// Create a new relay middleware with the specified processor.
    ///
    /// # Arguments
    /// * `processor` - The business logic implementation
    /// * `relay_pubkey` - The relay's public key
    /// * `database` - Database for storing events
    pub fn new(processor: P, relay_pubkey: PublicKey, database: Arc<RelayDatabase>) -> Self {
        Self {
            processor: Arc::new(processor),
            relay_pubkey,
            database,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Get a reference to the event processor
    pub fn processor(&self) -> &Arc<P> {
        &self.processor
    }

    /// Handle EVENT messages with optimized performance
    async fn handle_event(
        &self,
        event: Event,
        connection_state: &mut NostrConnectionState<T>,
    ) -> Result<Vec<RelayMessage<'static>>, Error> {
        connection_state.event_start_time = Some(std::time::Instant::now());
        connection_state.event_kind = Some(event.kind.as_u16());

        // Extract necessary state before calling handle_event
        let subdomain = connection_state.subdomain().clone();
        let authed_pubkey = connection_state.authed_pubkey.as_ref().cloned();

        // Process with direct custom state access
        let context = EventContext {
            authed_pubkey: authed_pubkey.as_ref(),
            subdomain: &subdomain,
            relay_pubkey: &self.relay_pubkey,
        };

        let commands = self
            .processor
            .handle_event(event.clone(), &mut connection_state.custom, context)
            .await?;

        // Execute database commands via subscription service
        let subscription_service = connection_state
            .subscription_service()
            .ok_or_else(|| Error::internal("No subscription service available"))?;

        for command in commands {
            // Route all commands through the subscription service to utilize buffering and proper logging
            subscription_service
                .save_and_broadcast(command)
                .await
                .map_err(|e| Error::database(e.to_string()))?;
        }

        Ok(vec![RelayMessage::ok(event.id, true, "")])
    }

    /// Handle subscription with optimized event filtering
    async fn handle_subscription(
        &self,
        connection_state: &mut NostrConnectionState<T>,
        subscription_id: String,
        filters: Vec<Filter>,
    ) -> Result<Vec<RelayMessage<'static>>, Error> {
        // First verify filters are allowed
        let context = EventContext {
            authed_pubkey: connection_state.authed_pubkey.as_ref(),
            subdomain: connection_state.subdomain(),
            relay_pubkey: &self.relay_pubkey,
        };

        self.processor
            .verify_filters(&filters, &connection_state.custom, context)?;

        // Clone the custom state for use in the filter function
        let custom_state = connection_state.custom.clone();
        let subdomain = connection_state.subdomain().clone();
        let authed_pubkey = connection_state.authed_pubkey;
        let processor = Arc::clone(&self.processor);
        let relay_pubkey = self.relay_pubkey;

        // Create filter function with cloned state - no async needed
        let filter_fn =
            move |event: &Event, scope: &nostr_lmdb::Scope, auth_pk: Option<&PublicKey>| -> bool {
                // Create context on stack - zero heap allocations
                let context = EventContext {
                    authed_pubkey: auth_pk,
                    subdomain: scope,
                    relay_pubkey: &relay_pubkey,
                };

                processor
                    .can_see_event(event, &custom_state, context)
                    .unwrap_or(false)
            };

        // Get subscription service and process
        let subscription_service = connection_state
            .subscription_service()
            .ok_or_else(|| Error::internal("No subscription service available"))?;

        subscription_service
            .handle_req(
                SubscriptionId::new(subscription_id),
                filters,
                authed_pubkey,
                &subdomain,
                filter_fn,
            )
            .await?;

        // Subscription service sends messages directly
        Ok(vec![])
    }
}

#[async_trait]
impl<P, T> Middleware for RelayMiddleware<P, T>
where
    P: EventProcessor<T>,
    T: Clone + Send + Sync + std::fmt::Debug + Default + 'static,
{
    type State = NostrConnectionState<T>;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn on_connect(
        &self,
        ctx: &mut websocket_builder::ConnectionContext<
            '_,
            Self::State,
            Self::IncomingMessage,
            Self::OutgoingMessage,
        >,
    ) -> anyhow::Result<()> {
        debug!("RelayMiddleware: Setting up connection");

        // Initialize the subscription service for this connection
        if let Some(ref sender) = ctx.sender {
            ctx.state
                .setup_connection(self.database.clone(), sender.clone())
                .await
                .map_err(|e| anyhow::anyhow!("Failed to setup connection: {}", e))?;
            debug!("RelayMiddleware: Connection setup complete");
        } else {
            error!("RelayMiddleware: No message sender available for connection setup");
        }

        ctx.next().await
    }

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> anyhow::Result<()> {
        let Some(message) = ctx.message.take() else {
            return ctx.next().await;
        };
        let state = &mut ctx.state;

        match message {
            ClientMessage::Event(boxed_event) => {
                // Handle EVENT message
                match self.handle_event(boxed_event.into_owned(), state).await {
                    Ok(responses) => {
                        for response in responses {
                            ctx.send_message(response)?;
                        }
                    }
                    Err(e) => {
                        error!("Event processing error: {}", e);
                        // Propagate the error up the chain so ErrorHandlingMiddleware can format it properly
                        return Err(e.into());
                    }
                }
                ctx.next().await
            }

            ClientMessage::Req {
                subscription_id,
                filter,
            } => {
                // First check if processor wants to handle it
                let processor_response = self
                    .processor
                    .handle_message(
                        ClientMessage::Req {
                            subscription_id: subscription_id.clone(),
                            filter: filter.clone(),
                        },
                        state,
                    )
                    .await?;

                if !processor_response.is_empty() {
                    // Processor handled it
                    for msg in processor_response {
                        ctx.send_message(msg)?;
                    }
                } else {
                    // Use generic subscription handling
                    match self
                        .handle_subscription(
                            state,
                            subscription_id.to_string(),
                            vec![filter.into_owned()],
                        )
                        .await
                    {
                        Ok(messages) => {
                            for msg in messages {
                                ctx.send_message(msg)?;
                            }
                        }
                        Err(e) => {
                            error!("Subscription error: {}", e);
                            // Propagate the error up the chain so ErrorHandlingMiddleware can format it properly
                            return Err(e.into());
                        }
                    }
                }
                ctx.next().await
            }

            ClientMessage::Close(subscription_id) => {
                // Handle CLOSE message
                if let Some(subscription_service) = state.subscription_service() {
                    let subscription_id = subscription_id.into_owned();
                    let _ = subscription_service.remove_subscription(subscription_id.clone());
                    debug!("Closed subscription: {}", subscription_id);
                }
                ctx.next().await
            }

            // Delegate all other messages to processor
            message => {
                let responses = self.processor.handle_message(message, state).await?;
                for response in responses {
                    ctx.send_message(response)?;
                }
                ctx.next().await
            }
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> anyhow::Result<()> {
        let Some(message) = ctx.message.take() else {
            return ctx.next().await;
        };

        // For broadcast events, check visibility before sending
        if let RelayMessage::Event { event, .. } = &message {
            let state = &mut ctx.state;
            let subdomain = state.subdomain().clone();
            let authed_pubkey = state.authed_pubkey;
            let context = EventContext {
                authed_pubkey: authed_pubkey.as_ref(),
                subdomain: &subdomain,
                relay_pubkey: &self.relay_pubkey,
            };

            if !self
                .processor
                .can_see_event(event, &state.custom, context)?
            {
                return ctx.next().await; // Filter out
            }
        }

        ctx.message = Some(message);
        ctx.next().await
    }
}
