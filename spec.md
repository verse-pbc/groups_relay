# Custom State Support Specification

## Overview

This specification describes the addition of custom per-connection state support to the `nostr_relay_builder` crate. The goal is to allow users to attach their own state to WebSocket connections while maintaining a clean separation between framework internals and user code.

## Design Goals

1. **Clean Separation**: EventProcessor implementations should only access their custom state, not framework internals
2. **Zero-Cost Abstraction**: No allocations when passing context to EventProcessor
3. **Type Safety**: Full compile-time type checking for custom state
4. **Backward Compatibility**: Existing code continues working with minimal changes

## Architecture

### Core Components

#### 1. Generic NostrConnectionState
```rust
pub struct NostrConnectionState<T = ()> {
    // Framework internals (private)
    relay_url: RelayUrl,
    challenge: Option<String>,
    authed_pubkey: Option<PublicKey>,
    subscription_service: Option<SubscriptionService>,
    connection_token: CancellationToken,
    event_start_time: Option<Instant>,
    event_kind: Option<u16>,
    subdomain: Scope,
    
    // User's custom state (public)
    pub custom: T,
}
```

#### 2. EventContext (Zero-Allocation)
```rust
#[derive(Debug, Copy, Clone)]
pub struct EventContext<'a> {
    pub authed_pubkey: Option<&'a PublicKey>,
    pub subdomain: Option<&'a str>,
    pub connection_id: &'a str,
}
```

#### 3. EventProcessor Trait
```rust
#[async_trait]
pub trait EventProcessor<T = ()>: Send + Sync + Debug + 'static {
    async fn handle_event(
        &self,
        event: Event,
        context: EventContext<'_>,
        state: &mut T,
    ) -> Result<Vec<StoreCommand>>;
    
    // Other methods follow same pattern...
}
```

## Usage Example

```rust
// Define custom state
#[derive(Debug, Default)]
struct MyState {
    rate_limit_tokens: u32,
    last_activity: Instant,
}

// Implement EventProcessor
struct MyRelay;

#[async_trait]
impl EventProcessor<MyState> for MyRelay {
    async fn handle_event(
        &self,
        event: Event,
        context: EventContext<'_>,  // Read-only framework data
        state: &mut MyState,         // Mutable custom state
    ) -> Result<Vec<StoreCommand>> {
        // Check auth from context
        if context.authed_pubkey.is_none() {
            return Ok(vec![]);
        }
        
        // Use custom state
        if state.rate_limit_tokens == 0 {
            return Ok(vec![]);
        }
        state.rate_limit_tokens -= 1;
        
        // Store event
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain_scope(),
        )])
    }
}
```

## Implementation Plan

See `prompt_plan.md` for the step-by-step implementation plan.

## Benefits

1. **Flexibility**: Users can add any state they need
2. **Safety**: Framework internals are protected
3. **Performance**: Zero allocations for context passing
4. **Simplicity**: Clean API for common use cases