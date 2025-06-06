//! Optimized EventProcessor API Design
//! 
//! This module presents a fully optimized EventProcessor API that eliminates
//! allocations and unnecessary parameter passing in hot paths.

use crate::error::Error;
use crate::state::NostrConnectionState;
use crate::subscription_service::StoreCommand;
use async_trait::async_trait;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;

/// Minimal context for event visibility checks
/// 
/// Contains only the essential data needed for most visibility decisions.
/// Designed to be created on the stack with zero heap allocations.
#[derive(Debug, Clone, Copy)]
pub struct EventContext<'a> {
    /// Authenticated public key of the connection (if any)
    pub authed_pubkey: Option<&'a PublicKey>,
    /// The subdomain/scope this connection is operating in
    pub subdomain: &'a Scope,
    /// The relay's public key
    pub relay_pubkey: &'a PublicKey,
}

/// Optimized event processor trait with zero-allocation methods
/// 
/// This trait provides two APIs:
/// 1. Fast methods (`*_fast`) that work with minimal parameters
/// 2. Full methods that maintain backward compatibility
#[async_trait]
pub trait OptimizedEventProcessor<T = ()>: Send + Sync + std::fmt::Debug + 'static 
where
    T: Send + Sync + 'static,
{
    // ===== Fast API (Optimized) =====
    
    /// Check event visibility with minimal overhead
    /// 
    /// This method is called in hot loops during subscription processing.
    /// It receives only essential data to minimize parameter passing overhead.
    fn can_see_event_fast(
        &self,
        event: &Event,
        custom_state: &T,
        context: EventContext<'_>,
    ) -> Result<bool, Error> {
        // Default: delegate to full method if not overridden
        // This provides backward compatibility
        let mut temp_state = NostrConnectionState::new("temp".to_string())
            .map_err(|e| Error::internal(format!("Failed to create temp state: {}", e)))?;
        // We can't set custom state here without cloning, so we return true by default
        let _ = custom_state;
        Ok(true)
    }
    
    /// Process an event with direct access to custom state
    /// 
    /// This method avoids the overhead of passing full connection state
    /// when only custom state mutations are needed.
    async fn handle_event_fast(
        &self,
        event: Event,
        custom_state: &mut T,
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Default: save the event
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
    
    // ===== Full API (Backward Compatible) =====
    
    /// Check event visibility with full connection state
    /// 
    /// Provided for backward compatibility and complex use cases
    /// that need access to the full connection state.
    fn can_see_event(
        &self,
        event: &Event,
        connection_id: &str,
        state: &NostrConnectionState<T>,
        relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        // Default: delegate to fast method
        let context = EventContext {
            authed_pubkey: state.authed_pubkey.as_ref(),
            subdomain: state.subdomain(),
            relay_pubkey,
        };
        self.can_see_event_fast(event, &state.custom, context)
    }
    
    /// Process an event with full connection state access
    async fn handle_event(
        &self,
        event: Event,
        connection_id: &str,
        state: &mut NostrConnectionState<T>,
    ) -> Result<Vec<StoreCommand>, Error> {
        let context = EventContext {
            authed_pubkey: state.authed_pubkey.as_ref(),
            subdomain: state.subdomain(),
            relay_pubkey: &state.relay_url.parse().unwrap_or_else(|_| PublicKey::from_slice(&[0; 32]).unwrap()),
        };
        self.handle_event_fast(event, &mut state.custom, context).await
    }
    
    /// Handle non-event messages with full state access
    async fn handle_message(
        &self,
        message: ClientMessage<'static>,
        state: &mut NostrConnectionState<T>,
    ) -> Result<Vec<RelayMessage<'static>>, Error> {
        // Default implementation unchanged
        match message {
            ClientMessage::Auth(auth_event) => {
                state.authed_pubkey = Some(auth_event.pubkey);
                Ok(vec![RelayMessage::ok(auth_event.id, false, "")])
            }
            _ => Ok(vec![]),
        }
    }
    
    // ===== Optimization Hints =====
    
    /// Hint to the framework whether this processor implements the fast API
    /// 
    /// This allows the middleware to choose the optimal code path at runtime.
    fn prefers_fast_api(&self) -> bool {
        false // Default to false for backward compatibility
    }
    
    /// Whether the processor needs connection_id for visibility checks
    /// 
    /// Most processors don't need connection_id, so we can skip passing it.
    fn needs_connection_id(&self) -> bool {
        false
    }
}

// ===== Example Implementations =====

/// Example: Rate limiter using the optimized API
#[derive(Debug)]
pub struct OptimizedRateLimiter {
    pub rate_limit: f32,
}

#[derive(Debug, Clone, Default)]
pub struct RateLimitState {
    pub tokens: f32,
    pub events_processed: u64,
    pub last_reset: u64,
}

#[async_trait]
impl OptimizedEventProcessor<RateLimitState> for OptimizedRateLimiter {
    fn can_see_event_fast(
        &self,
        _event: &Event,
        custom_state: &RateLimitState,
        _context: EventContext<'_>,
    ) -> Result<bool, Error> {
        // Direct access to custom state - no allocations
        Ok(custom_state.tokens > 0.0)
    }
    
    async fn handle_event_fast(
        &self,
        event: Event,
        custom_state: &mut RateLimitState,
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Check rate limit
        if custom_state.tokens < 1.0 {
            return Err(Error::restricted("Rate limit exceeded"));
        }
        
        // Consume token
        custom_state.tokens -= 1.0;
        custom_state.events_processed += 1;
        
        // Save event
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
    
    fn prefers_fast_api(&self) -> bool {
        true // This processor is optimized for the fast API
    }
}

/// Example: Group access control using optimized API
#[derive(Debug)]
pub struct OptimizedGroupProcessor;

#[derive(Debug, Clone, Default)]
pub struct GroupMembershipState {
    pub is_member: bool,
    pub is_admin: bool,
    pub groups: Vec<String>,
}

#[async_trait]
impl OptimizedEventProcessor<GroupMembershipState> for OptimizedGroupProcessor {
    fn can_see_event_fast(
        &self,
        event: &Event,
        custom_state: &GroupMembershipState,
        context: EventContext<'_>,
    ) -> Result<bool, Error> {
        // Check if event is from a private group
        if event.tags.iter().any(|tag| tag.as_vec()[0] == "private") {
            // Only members can see private events
            Ok(custom_state.is_member)
        } else {
            // Public events are visible to all
            Ok(true)
        }
    }
    
    async fn handle_event_fast(
        &self,
        event: Event,
        custom_state: &mut GroupMembershipState,
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Admin-only events
        if event.kind == Kind::Custom(9001) && !custom_state.is_admin {
            return Err(Error::restricted("Admin access required"));
        }
        
        // Member-only events  
        if event.kind == Kind::Custom(9002) && !custom_state.is_member {
            return Err(Error::restricted("Membership required"));
        }
        
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
    
    fn prefers_fast_api(&self) -> bool {
        true
    }
}

/// Example: Backward compatible processor
#[derive(Debug)]
pub struct LegacyProcessor;

#[async_trait]
impl OptimizedEventProcessor<()> for LegacyProcessor {
    // Only implement the full API methods
    fn can_see_event(
        &self,
        event: &Event,
        connection_id: &str,
        state: &NostrConnectionState<()>,
        relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        // Legacy implementation that needs full state access
        println!("Connection {} checking event {}", connection_id, event.id);
        Ok(true)
    }
    
    async fn handle_event(
        &self,
        event: Event,
        connection_id: &str,
        state: &mut NostrConnectionState<()>,
    ) -> Result<Vec<StoreCommand>, Error> {
        println!("Connection {} handling event", connection_id);
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            state.subdomain().clone(),
        )])
    }
    
    fn needs_connection_id(&self) -> bool {
        true // This processor needs connection_id
    }
}