//! Optimized EventProcessor trait design
//! 
//! This module explores an optimized version of the EventProcessor trait
//! that minimizes allocations and cloning in hot paths.

use crate::error::Error;
use crate::state::NostrConnectionState;
use crate::subscription_service::StoreCommand;
use async_trait::async_trait;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;

/// Context for event visibility checks - minimal data needed
pub struct VisibilityContext<'a> {
    pub authed_pubkey: Option<&'a PublicKey>,
    pub subdomain: &'a Scope,
}

/// Optimized generic event processor trait
#[async_trait]
pub trait OptimizedEventProcessor<T = ()>: Send + Sync + std::fmt::Debug + 'static {
    /// Check if an event should be visible to this connection.
    /// 
    /// This optimized version receives only the custom state and minimal context,
    /// avoiding allocations in hot loops.
    fn can_see_event_fast(
        &self,
        event: &Event,
        custom_state: &T,
        context: &VisibilityContext,
        relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        // Default implementation: allow all events
        let _ = (event, custom_state, context, relay_pubkey);
        Ok(true)
    }

    /// Process an incoming event with mutable access to custom state only
    async fn handle_event_fast(
        &self,
        event: Event,
        custom_state: &mut T,
        context: &VisibilityContext,
    ) -> Result<Vec<StoreCommand>, Error>;

    /// Handle non-event messages with full state access (unchanged)
    async fn handle_message(
        &self,
        message: ClientMessage<'static>,
        state: &mut NostrConnectionState<T>,
    ) -> Result<Vec<RelayMessage<'static>>, Error> {
        // Default implementation
        Ok(vec![])
    }
}

/// Example implementation showing the optimized API
#[derive(Debug)]
pub struct OptimizedRateLimiter;

#[derive(Debug, Clone, Default)]
pub struct RateLimitState {
    pub tokens: f32,
    pub events_seen: u64,
}

#[async_trait]
impl OptimizedEventProcessor<RateLimitState> for OptimizedRateLimiter {
    fn can_see_event_fast(
        &self,
        _event: &Event,
        custom_state: &RateLimitState,
        _context: &VisibilityContext,
        _relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        // No allocations, direct field access
        Ok(custom_state.tokens > 0.0)
    }

    async fn handle_event_fast(
        &self,
        event: Event,
        custom_state: &mut RateLimitState,
        context: &VisibilityContext,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Direct mutation of custom state
        custom_state.tokens -= 1.0;
        custom_state.events_seen += 1;
        
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}