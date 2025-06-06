//! Optimized middleware implementation that uses the fast API when available
//! 
//! This middleware intelligently chooses between the fast and full APIs
//! based on processor capabilities, maximizing performance while maintaining
//! backward compatibility.

use crate::database::RelayDatabase;
use crate::error::Error;
use crate::optimized_processor::{EventContext, OptimizedEventProcessor};
use crate::state::NostrConnectionState;
use crate::subscription_service::SubscriptionService;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use websocket_builder::{Middleware, MiddlewareContext};

/// Optimized relay middleware that adapts to processor capabilities
pub struct OptimizedRelayMiddleware<P, T = ()> {
    processor: Arc<P>,
    relay_pubkey: PublicKey,
    database: Arc<RelayDatabase>,
    /// Cached flag for whether to use fast API
    use_fast_api: bool,
    /// Cached flag for whether processor needs connection_id
    needs_connection_id: bool,
}

impl<P, T> OptimizedRelayMiddleware<P, T>
where
    P: OptimizedEventProcessor<T>,
    T: Clone + Send + Sync + std::fmt::Debug + 'static,
{
    pub fn new(processor: P, relay_pubkey: PublicKey, database: Arc<RelayDatabase>) -> Self {
        // Cache optimization hints
        let use_fast_api = processor.prefers_fast_api();
        let needs_connection_id = processor.needs_connection_id();
        
        Self {
            processor: Arc::new(processor),
            relay_pubkey,
            database,
            use_fast_api,
            needs_connection_id,
        }
    }
    
    /// Handle subscription using the most efficient API available
    async fn handle_subscription_optimized(
        &self,
        connection_state: &mut NostrConnectionState<T>,
        connection_id: &str,
        subscription_id: String,
        filters: Vec<Filter>,
    ) -> Result<Vec<RelayMessage<'static>>, Error> {
        let subscription_service = connection_state
            .subscription_service()
            .ok_or_else(|| Error::internal("No subscription service available"))?;
        
        let processor = Arc::clone(&self.processor);
        let relay_pubkey = self.relay_pubkey;
        let subdomain = connection_state.subdomain().clone();
        
        if self.use_fast_api {
            // ===== FAST PATH: Zero allocations =====
            
            // Create a thin wrapper that captures minimal state
            struct FastFilterState<T> {
                custom_state: T,
                processor: Arc<dyn OptimizedEventProcessor<T>>,
                relay_pubkey: PublicKey,
            }
            
            // Move custom state out temporarily (no clone!)
            let filter_state = FastFilterState {
                custom_state: std::mem::take(&mut connection_state.custom),
                processor: processor as Arc<dyn OptimizedEventProcessor<T>>,
                relay_pubkey,
            };
            
            // Create filter function with zero allocations in the hot path
            let filter_fn = move |event: &Event, scope: &nostr_lmdb::Scope, authed_pk: Option<&PublicKey>| -> bool {
                let context = EventContext {
                    authed_pubkey: authed_pk,
                    subdomain: scope,
                    relay_pubkey: &filter_state.relay_pubkey,
                };
                
                filter_state.processor
                    .can_see_event_fast(event, &filter_state.custom_state, context)
                    .unwrap_or(false)
            };
            
            // Process subscription
            let result = subscription_service
                .handle_req(
                    subscription_id,
                    filters,
                    connection_state.authed_pubkey,
                    &subdomain,
                    filter_fn,
                )
                .await;
            
            // In a real implementation, we'd need a way to recover the custom state
            // This would require modifying SubscriptionService to return the filter_fn
            // or use a different pattern
            
            result
        } else {
            // ===== COMPATIBILITY PATH: Use full API =====
            
            let custom_state = connection_state.custom.clone();
            let conn_id = if self.needs_connection_id {
                connection_id.to_string()
            } else {
                String::new() // Empty string for processors that don't need it
            };
            
            let filter_fn = move |event: &Event, scope: &nostr_lmdb::Scope, authed_pk: Option<&PublicKey>| -> bool {
                // Still need to create state for compatibility
                let mut minimal_state = match NostrConnectionState::<T>::with_custom(
                    "ws://minimal".to_string(),
                    custom_state.clone(),
                ) {
                    Ok(state) => state,
                    Err(_) => return false,
                };
                minimal_state.authed_pubkey = authed_pk.cloned();
                minimal_state.subdomain = scope.clone();
                
                processor
                    .can_see_event(event, &conn_id, &minimal_state, &relay_pubkey)
                    .unwrap_or(false)
            };
            
            subscription_service
                .handle_req(
                    subscription_id,
                    filters,
                    connection_state.authed_pubkey,
                    &subdomain,
                    filter_fn,
                )
                .await
        }
    }
}

#[async_trait]
impl<P, T> Middleware for OptimizedRelayMiddleware<P, T>
where
    P: OptimizedEventProcessor<T>,
    T: Clone + Send + Sync + std::fmt::Debug + 'static,
{
    type State = NostrConnectionState<T>;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;
    
    async fn process_inbound(
        &self,
        message: Self::IncomingMessage,
        ctx: &mut MiddlewareContext<Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<Option<Self::IncomingMessage>, Box<dyn std::error::Error + Send + Sync>> {
        let connection_id = ctx.connection_id();
        let state = ctx.state_mut();
        
        match message {
            ClientMessage::Event(event) => {
                // Handle EVENT messages with optimized API
                let commands = if self.use_fast_api {
                    // Fast path: direct custom state access
                    let context = EventContext {
                        authed_pubkey: state.authed_pubkey.as_ref(),
                        subdomain: state.subdomain(),
                        relay_pubkey: &self.relay_pubkey,
                    };
                    
                    self.processor
                        .handle_event_fast(*event.clone(), &mut state.custom, context)
                        .await?
                } else {
                    // Compatibility path: full state access
                    self.processor
                        .handle_event(*event.clone(), connection_id, state)
                        .await?
                };
                
                // Execute commands...
                for command in commands {
                    // Process store commands
                }
                
                ctx.send_outbound(RelayMessage::ok(event.id, true, ""))
                    .await?;
                Ok(None)
            }
            
            ClientMessage::Req { subscription_id, filters } => {
                // Handle REQ with optimized subscription processing
                let messages = self.handle_subscription_optimized(
                    state,
                    connection_id,
                    subscription_id.to_string(),
                    filters,
                ).await?;
                
                for msg in messages {
                    ctx.send_outbound(msg).await?;
                }
                Ok(None)
            }
            
            // Other messages delegate to processor
            _ => {
                let messages = self.processor.handle_message(message, state).await?;
                for msg in messages {
                    ctx.send_outbound(msg).await?;
                }
                Ok(None)
            }
        }
    }
}

/// Performance comparison helper
pub mod benchmarks {
    use super::*;
    use std::time::Instant;
    
    /// Measure the performance difference between APIs
    pub async fn compare_apis<T: Clone + Default>(
        processor: &impl OptimizedEventProcessor<T>,
        events: Vec<Event>,
    ) {
        let mut custom_state = T::default();
        let context = EventContext {
            authed_pubkey: None,
            subdomain: &nostr_lmdb::Scope::Global,
            relay_pubkey: &PublicKey::from_slice(&[1; 32]).unwrap(),
        };
        
        // Benchmark fast API
        let start = Instant::now();
        for event in &events {
            let _ = processor.can_see_event_fast(event, &custom_state, context);
        }
        let fast_time = start.elapsed();
        
        // Benchmark full API (with allocation overhead)
        let connection_state = NostrConnectionState::with_custom(
            "ws://bench".to_string(),
            custom_state.clone(),
        ).unwrap();
        
        let start = Instant::now();
        for event in &events {
            let _ = processor.can_see_event(
                event,
                "bench",
                &connection_state,
                context.relay_pubkey,
            );
        }
        let full_time = start.elapsed();
        
        println!("Fast API: {:?}", fast_time);
        println!("Full API: {:?}", full_time);
        println!("Speedup: {:.2}x", full_time.as_nanos() as f64 / fast_time.as_nanos() as f64);
    }
}