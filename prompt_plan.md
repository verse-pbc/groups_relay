# RelayBuilder Handler Improvements - Implementation Plan

## Overview

This plan improves the RelayBuilder's handler system to be more flexible and production-ready by:
1. Separating WebSocket/NIP-11 handling from HTML serving
2. Supporting custom cancellation tokens via builder pattern
3. Supporting optional connection counting via builder pattern
4. Making the builder usable for complex cases like groups_relay

## Design Principles

- **Separation of Concerns**: WebSocket/NIP-11 handling should be separate from frontend serving
- **Flexibility**: Support optional features (cancellation token, connection counting) via fluent interface
- **Production Ready**: Make the builder suitable for real production use cases
- **Idiomatic API**: Use builder pattern consistently throughout

## Implementation Prompts

### ✅ Prompt 1: Update RelayBuilder with Optional Features
**Task**: Add cancellation token and connection counter to RelayBuilder's fluent interface
**Steps**:
1. Update `RelayBuilder<T>` struct in `src/generic_builder.rs`:
   ```rust
   pub struct RelayBuilder<T = ()> {
       config: RelayConfig,
       middlewares: Vec<...>,
       state_factory: Option<...>,
       cancellation_token: Option<CancellationToken>,
       connection_counter: Option<Arc<AtomicUsize>>,
       _phantom: PhantomData<T>,
   }
   ```
2. Add builder methods:
   ```rust
   pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
       self.cancellation_token = Some(token);
       self
   }
   
   pub fn with_connection_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
       self.connection_counter = Some(counter);
       self
   }
   ```
3. Update `new()` to initialize these fields as None
4. Pass these to handlers when building
5. Commit: "Add cancellation token and connection counter to RelayBuilder"

### ✅ Prompt 2: Update RelayHandlers Structure
**Task**: Make RelayHandlers use the optional features from builder
**Steps**:
1. Update `RelayHandlers<T>` struct in `src/handlers.rs`:
   ```rust
   pub struct RelayHandlers<T = ()> {
       pub ws_handler: Arc<crate::RelayWebSocketHandler<T>>,
       pub relay_info: RelayInfo,
       cancellation_token: CancellationToken,
       connection_counter: Option<Arc<AtomicUsize>>,
   }
   ```
2. Update `new()` to accept these from builder:
   ```rust
   pub fn new(
       ws_handler: RelayWebSocketHandler<T>,
       relay_info: RelayInfo,
       cancellation_token: Option<CancellationToken>,
       connection_counter: Option<Arc<AtomicUsize>>,
   ) -> Self
   ```
3. Create default cancellation token if none provided
4. Add getter methods for these fields
5. Commit: "Update RelayHandlers to support optional features"

### ✅ Prompt 3: Update build_handlers Method
**Task**: Update build_handlers to use builder's optional features
**Steps**:
1. Update `build_handlers()` in RelayBuilder:
   ```rust
   pub async fn build_handlers<L: EventProcessor<T>>(
       self,
       processor: L,
       relay_info: RelayInfo,
   ) -> Result<RelayHandlers<T>, Error>
   ```
2. Pass `self.cancellation_token` and `self.connection_counter` to RelayHandlers::new
3. Remove `frontend_html` parameter (no longer needed)
4. Update all existing calls to build_handlers
5. Commit: "Update build_handlers to use builder's optional features"

### ✅ Prompt 4: Simplify axum_root_handler
**Task**: Remove HTML handling from axum_root_handler, only handle WebSocket and NIP-11
**Steps**:
1. Remove HTML response logic from `axum_root_handler()`
2. Return 404 Not Found for non-WebSocket, non-NIP-11 requests
3. Remove `frontend_html` field from RelayHandlers
4. Remove DEFAULT_HTML constant
5. Update the handler to:
   - Handle WebSocket upgrades with cancellation token
   - Handle application/nostr+json requests
   - Return 404 for other requests
6. Note: Users who want HTML at root should check request type before calling handler
7. Commit: "Simplify axum_root_handler to only handle WebSocket and NIP-11"

### ✅ Prompt 5: Create axum_ws_handler Method
**Task**: Create a dedicated WebSocket-only handler method
**Steps**:
1. Add `axum_ws_handler()` method that only handles WebSocket upgrades
2. This allows users to mount WebSocket handling at any route
3. Include connection counting if configured
4. Use the stored cancellation token
5. Return 404 if not a WebSocket upgrade request
6. Commit: "Add dedicated WebSocket handler method"

### ✅ Prompt 6: Implement Connection Counter
**Task**: Create connection counting helper
**Steps**:
1. Create `ConnectionCounter` struct in handlers module:
   ```rust
   struct ConnectionCounter {
       counter: Option<Arc<AtomicUsize>>,
   }
   
   impl ConnectionCounter {
       fn new(counter: Option<Arc<AtomicUsize>>) -> Self {
           if let Some(ref c) = counter {
               c.fetch_add(1, Ordering::Relaxed);
           }
           Self { counter }
       }
   }
   
   impl Drop for ConnectionCounter {
       fn drop(&mut self) {
           if let Some(ref c) = self.counter {
               c.fetch_sub(1, Ordering::Relaxed);
           }
       }
   }
   ```
2. Use in WebSocket handlers
3. Add `connection_count()` method to RelayHandlers
4. Commit: "Implement optional connection counting"

### ✅ Prompt 7: Update Examples
**Task**: Update examples to show new patterns
**Steps**:
1. Update `minimal_relay.rs`:
   ```rust
   let handlers = RelayBuilder::new(config)
       .with_standard_nips()?
       .build_handlers(processor, relay_info)
       .await?;
   
   // Handler for root that serves either WebSocket/NIP-11 or HTML
   async fn root_handler(
       ws: Option<WebSocketUpgrade>,
       headers: HeaderMap,
       handlers: Arc<RelayHandlers>,
   ) -> impl IntoResponse {
       if ws.is_some() || headers.get(ACCEPT).map_or(false, |v| v == "application/nostr+json") {
           handlers.axum_root_handler()(ws, headers).await
       } else {
           Html("<h1>My Relay</h1>").into_response()
       }
   }
   
   let app = Router::new()
       .route("/", get(move |ws, headers| root_handler(ws, headers, handlers.clone())));
   // No fallback needed - other routes get default 404
   ```
2. Update `advanced_relay.rs` with cancellation token:
   ```rust
   let cancellation_token = CancellationToken::new();
   let handlers = RelayBuilder::new(config)
       .with_cancellation_token(cancellation_token.clone())
       .build_handlers(processor, relay_info)
       .await?;
   ```
3. Show connection counting in another example
4. Commit: "Update examples for new handler patterns"

### ✅ Prompt 8: Create Production Example
**Task**: Create example showing production patterns
**Steps**:
1. Create `examples/production_relay.rs` showing:
   - Graceful shutdown with cancellation token
   - Connection counting for metrics
   - Separate WebSocket endpoint
   - Custom HTML only at root path
   - Static assets at specific routes (e.g., /assets/)
   - Health check endpoint using connection count
2. Show proper error handling and logging
3. Document production considerations
4. Commit: "Add production relay example"

### ✅ Prompt 9: Update Groups Relay Integration
**Task**: Evaluate if groups_relay can use updated builder
**Steps**:
1. Check if groups_relay can now use RelayBuilder with:
   ```rust
   RelayBuilder::new(relay_config)
       .with_middleware(ValidationMiddleware::new(relay_keys.public_key))
       .with_cancellation_token(cancellation_token)
       .with_connection_counter(connection_counter)
       .build_handlers(groups_processor, relay_info)
       .await?
   ```
2. If yes, refactor to use builder handlers
3. Document any remaining limitations
4. Test thoroughly
5. Commit: "Update groups_relay to use improved builder"

### ❌ Prompt 10: Add Handler Documentation
**Task**: Document the new handler patterns and best practices
**Steps**:
1. Create `docs/handler_guide.md` documenting:
   - WebSocket vs HTML separation philosophy
   - Using cancellation tokens for graceful shutdown
   - Connection counting for metrics/monitoring
   - Mounting handlers at different routes
   - Frontend serving strategies with examples
2. Update README with handler section
3. Add comprehensive inline documentation
4. Include migration guide from old patterns
5. Commit: "Add comprehensive handler documentation"

### ❌ Prompt 11: Add Handler Tests
**Task**: Add comprehensive tests for handler functionality
**Steps**:
1. Test RelayBuilder with/without optional features
2. Test connection counting increment/decrement
3. Test cancellation token propagation
4. Test WebSocket upgrade handling
5. Test NIP-11 response
6. Test 404 responses for non-WebSocket/NIP-11
7. Add integration tests with axum
8. Commit: "Add comprehensive handler tests"

### ❌ Prompt 12: Performance and Migration
**Task**: Validate performance and create migration guide
**Steps**:
1. Benchmark handler with/without connection counting
2. Ensure cancellation token adds no overhead when not cancelled
3. Create migration guide showing:
   - Old: handlers served HTML at root
   - New: handlers only serve WebSocket/NIP-11, use fallback for HTML
   - Benefits of new approach
4. Document in CHANGELOG.md
5. Commit: "Add performance validation and migration guide"

## Expected API Usage

After implementation, the API will be:

```rust
// Simple case - defaults for everything
let handlers = RelayBuilder::new(config)
    .with_standard_nips()?
    .build_handlers(processor, relay_info)
    .await?;

// Production case - with monitoring and graceful shutdown
let cancellation_token = CancellationToken::new();
let connection_counter = Arc::new(AtomicUsize::new(0));

let handlers = RelayBuilder::new(config)
    .with_standard_nips()?
    .with_cancellation_token(cancellation_token.clone())
    .with_connection_counter(connection_counter.clone())
    .build_handlers(processor, relay_info)
    .await?;

// Flexible routing
let app = Router::new()
    .route("/ws", get(handlers.axum_ws_handler()))      // WebSocket only
    .route("/", get({
        let handlers = handlers.clone();
        move |ws, headers| async move {
            // If WebSocket or NIP-11 request, handle it
            if ws.is_some() || wants_nip11(&headers) {
                handlers.axum_root_handler()(ws, headers).await
            } else {
                // Otherwise serve custom HTML for root only
                Html(include_str!("../static/index.html")).into_response()
            }
        }
    }))
    .route("/metrics", get(move || async move {
        format!("connections: {}", connection_counter.load(Ordering::Relaxed))
    }));
    // No fallback - other routes get 404 automatically
```

## Benefits

1. **Clean API**: Optional features use builder pattern, not function parameters
2. **Flexible**: Can mount handlers anywhere, serve frontend separately
3. **Production Ready**: Supports graceful shutdown and monitoring
4. **Simple Defaults**: Basic usage remains simple
5. **Composable**: Each handler method has single responsibility

## Notes

- Keep backward compatibility where reasonable
- Default behavior should be sensible (create own cancellation token)
- Document patterns thoroughly
- Test with real usage (groups_relay)