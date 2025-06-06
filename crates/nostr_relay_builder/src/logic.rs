//! Event processor trait for handling Nostr events with zero-allocation performance
//!
//! This module provides the [`EventProcessor`] trait, which is the core abstraction
//! for implementing custom relay business logic. The API is designed to minimize
//! allocations and maximize performance in hot paths like subscription processing.

use crate::error::Result;
use crate::state::NostrConnectionState;
use crate::subscription_service::StoreCommand;
use async_trait::async_trait;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use tracing::debug;

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

/// Core trait defining relay business logic with optimized performance.
///
/// This trait uses a generic type parameter `T` for custom per-connection state,
/// enabling type-safe access to application-specific data without runtime overhead.
///
/// ## Design Principles
///
/// 1. **Zero-allocation hot paths**: Methods like `can_see_event` receive only references
/// 2. **Direct state access**: Custom state is passed by reference, avoiding cloning
/// 3. **Minimal context**: EventContext is stack-allocated with only essential data
/// 4. **Type safety**: Generic parameter ensures compile-time type checking
///
/// ## Example
///
/// ```rust,no_run
/// use nostr_relay_builder::{EventProcessor, EventContext, StoreCommand};
/// use nostr_sdk::prelude::*;
/// use async_trait::async_trait;
///
/// #[derive(Debug, Clone, Default)]
/// struct RateLimitState {
///     tokens: f32,
///     events_processed: u64,
/// }
///
/// #[derive(Debug)]
/// struct RateLimitedProcessor {
///     tokens_per_event: f32,
/// }
///
/// #[async_trait]
/// impl EventProcessor<RateLimitState> for RateLimitedProcessor {
///     fn can_see_event(
///         &self,
///         _event: &Event,
///         custom_state: &RateLimitState,
///         _context: EventContext<'_>,
///     ) -> Result<bool, nostr_relay_builder::Error> {
///         Ok(custom_state.tokens > 0.0)
///     }
///
///     async fn handle_event(
///         &self,
///         event: Event,
///         custom_state: &mut RateLimitState,
///         context: EventContext<'_>,
///     ) -> Result<Vec<StoreCommand>, nostr_relay_builder::Error> {
///         if custom_state.tokens < self.tokens_per_event {
///             return Err(nostr_relay_builder::Error::restricted("Rate limit exceeded"));
///         }
///
///         custom_state.tokens -= self.tokens_per_event;
///         custom_state.events_processed += 1;
///
///         Ok(vec![StoreCommand::SaveSignedEvent(
///             Box::new(event),
///             context.subdomain.clone(),
///         )])
///     }
/// }
/// ```
#[async_trait]
pub trait EventProcessor<T = ()>: Send + Sync + std::fmt::Debug + 'static
where
    T: Send + Sync + 'static,
{
    /// Check if an event should be visible to this connection.
    ///
    /// This method is called in hot loops during subscription processing,
    /// so it must be synchronous for maximum performance with zero allocations.
    /// Any necessary async operations (like database lookups) should be done
    /// during connection setup and cached in the custom state.
    ///
    /// Note: The state is immutable here because this method may be called
    /// concurrently from multiple threads. If you need mutable state, use
    /// interior mutability patterns like `RefCell` or `Mutex`.
    ///
    /// # Arguments
    /// * `event` - The event to check visibility for
    /// * `custom_state` - Custom per-connection state with cached auth data
    /// * `context` - Minimal context with auth info and relay details
    ///
    /// # Returns
    /// * `Ok(true)` - Event is visible to this connection
    /// * `Ok(false)` - Event should be filtered out
    /// * `Err(Error)` - Processing error (will be logged)
    fn can_see_event(
        &self,
        event: &Event,
        custom_state: &T,
        context: EventContext<'_>,
    ) -> Result<bool> {
        // Default implementation: allow all events (public relay behavior)
        let _ = (event, custom_state, context);
        Ok(true)
    }

    /// Verify if filters are allowed for this connection.
    ///
    /// This method validates subscription filters before processing.
    ///
    /// # Arguments
    /// * `filters` - The filters from the REQ message
    /// * `custom_state` - Custom per-connection state (read-only)
    /// * `context` - Minimal context with auth info
    ///
    /// # Returns
    /// * `Ok(())` - Filters are allowed
    /// * `Err(Error)` - Filter validation failed (will be converted to NOTICE)
    fn verify_filters(
        &self,
        filters: &[Filter],
        custom_state: &T,
        context: EventContext<'_>,
    ) -> Result<()> {
        // Default implementation: allow all filters
        let _ = (filters, custom_state, context);
        Ok(())
    }

    /// Process an incoming event and return database commands.
    ///
    /// This method handles EVENT messages with direct access to mutable custom state,
    /// avoiding the overhead of passing full connection state.
    ///
    /// # Arguments
    /// * `event` - The event to process
    /// * `custom_state` - Custom per-connection state (mutable)
    /// * `context` - Minimal context with auth info
    ///
    /// # Returns
    /// * `Ok(commands)` - List of database commands to execute
    /// * `Err(Error)` - Processing failed (will be converted to NOTICE)
    async fn handle_event(
        &self,
        event: Event,
        custom_state: &mut T,
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>>;

    /// Handle non-event messages with full connection state access.
    ///
    /// This method provides full state access for complex operations like
    /// authentication protocols and subscription management.
    ///
    /// ## Message Handling Priority
    ///
    /// **REQ and ReqMultiFilter**: Return empty vec to use default subscription handling
    /// **Close and Event**: Always handled by middleware - return empty vec
    /// **All others (AUTH, COUNT, NIP-45)**: Handled entirely by your implementation
    ///
    /// # Arguments
    /// * `message` - The client message to handle
    /// * `state` - Full connection state (read/write access)
    ///
    /// # Returns
    /// * `Ok(messages)` - Relay messages to send (empty = use defaults)
    /// * `Err(Error)` - Message handling failed (will be converted to NOTICE)
    async fn handle_message(
        &self,
        message: ClientMessage<'static>,
        state: &mut NostrConnectionState<T>,
    ) -> Result<Vec<RelayMessage<'static>>> {
        // Default implementation: handle common message types with basic responses
        match message {
            ClientMessage::Auth(auth_event) => {
                // Basic NIP-42 auth: verify and store pubkey
                state.authed_pubkey = Some(auth_event.pubkey);
                Ok(vec![RelayMessage::ok(auth_event.id, false, "")])
            }

            ClientMessage::Count {
                subscription_id,
                filter: _,
            } => {
                // Basic COUNT: return zero (override for actual implementation)
                Ok(vec![RelayMessage::count(subscription_id.into_owned(), 0)])
            }

            ClientMessage::Req { .. } | ClientMessage::ReqMultiFilter { .. } => {
                // Return empty to use middleware's generic subscription handling
                debug!("Using generic subscription handling");
                Ok(vec![])
            }

            // Close and Event messages are always handled by middleware
            _ => {
                debug!("Message handled generically by RelayMiddleware");
                Ok(vec![])
            }
        }
    }
}

/// Simple public relay processor - stores all valid events
#[derive(Debug, Clone, Default)]
pub struct PublicRelayProcessor;

#[async_trait]
impl EventProcessor<()> for PublicRelayProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>> {
        // Note: Event verification is already done by EventVerifierMiddleware
        // Simply store the event
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}
