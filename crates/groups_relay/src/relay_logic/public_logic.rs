use async_trait::async_trait;
use nostr_relay_builder::{EventContext, EventProcessor, Result, StoreCommand};
use nostr_sdk::prelude::*;
use tracing::debug;

/// Simple public event processor that accepts all events and makes them visible to all users.
///
/// This implementation provides a basic relay behavior suitable for:
/// - Public relays with no access control
/// - Testing and development
/// - Base implementation for more complex relay types
#[derive(Debug, Clone)]
pub struct PublicRelayProcessor;

impl PublicRelayProcessor {
    /// Create a new public event processor instance
    pub fn new() -> Self {
        Self
    }
}

impl Default for PublicRelayProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventProcessor for PublicRelayProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>> {
        debug!(
            target: "public_relay",
            "Processing event: kind={}, id={}",
            event.kind,
            event.id
        );

        // Public relay: save all events to the subdomain scope
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}
