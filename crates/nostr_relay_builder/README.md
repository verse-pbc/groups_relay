# Nostr Relay Builder

A framework for building custom Nostr relays with middleware support. Built on top of `websocket_builder`, this crate provides the Nostr protocol layer for creating relays with:

- 🔌 **Pluggable business logic** via the `EventProcessor` trait
- 🏗️ **Middleware-based message processing** for clean separation of concerns and easy extension
- 📋 **Built-in protocol support** (NIPs 09, 40, 42, 70) with ability to add more via custom middlewares
- 🌐 **WebSocket handling** via the underlying `websocket_builder`
- 💾 **Database abstraction** with async event storage
- 🏢 **Multi-tenant support** via subdomain-based data isolation
- 🎯 **Custom state support** for per-connection application data

## Features

- **Generic relay infrastructure** - Handle WebSocket connections, message routing, and subscriptions
- **Customizable business logic** - Implement your own rules for event acceptance, filtering, and visibility
- **Extensible middleware system** - Add custom protocol support by implementing the `Middleware` trait
- **Built-in protocol middleware** - Common NIPs included out of the box:
  - NIP-09: Event Deletion
  - NIP-40: Event Expiration
  - NIP-42: Authentication
  - NIP-70: Protected Events
- **Easy to add more NIPs** - Implement additional protocols as custom middlewares
- **Scope/subdomain isolation** - Optional multi-tenant support with data isolation
- **Async/await throughout** - Built on Tokio for high performance
- **Metrics support** - Optional metrics collection for monitoring

## Quick Start

Add this to your `Cargo.toml`:

```toml
[dependencies]
nostr_relay_builder = "0.1"
nostr-sdk = "0.32"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"

# Optional: Enable Axum integration for quick deployment
nostr_relay_builder = { version = "0.1", features = ["axum"] }
```

## Examples

### Basic Public Relay

```rust
use nostr_relay_builder::{RelayBuilder, RelayConfig, EventProcessor, NostrConnectionState, StoreCommand};
use nostr_sdk::prelude::*;

#[derive(Debug, Clone)]
struct PublicEventProcessor;

#[async_trait::async_trait]
impl EventProcessor for PublicEventProcessor {
    async fn handle_event(
        &self,
        event: Event,
        state: &mut NostrConnectionState,
    ) -> Result<Vec<StoreCommand>> {
        // Accept all events
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            state.subdomain().clone(),
        )])
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let keys = Keys::generate();
    let config = RelayConfig::new("wss://localhost:8080", "./relay.db", keys);
    let processor = PublicEventProcessor;

    let ws_handler = RelayBuilder::new(config)
        .with_middleware(Nip09Middleware::new(database.clone()))
        .with_middleware(Nip40ExpirationMiddleware::new())
        .with_middleware(Nip70Middleware)
        .build_server(processor)
        .await?;

    // Use ws_handler with your HTTP server
    Ok(())
}
```

### Authenticated Relay (NIP-42)

```rust
use nostr_relay_builder::{AuthConfig, RelayBuilder, RelayConfig, EventProcessor};

let config = RelayConfig::new("wss://auth.relay.com", "./relay.db", keys)
    .with_auth(AuthConfig {
        auth_url: "wss://auth.relay.com".to_string(),
        base_domain_parts: 2,
        validate_subdomains: false,
    });

// NIP-42 auth is automatically enabled when with_auth() is used
let ws_handler = RelayBuilder::new(config)
    .build_server(processor)
    .await?;
```

### Multi-tenant Relay with Subdomains

```rust
let config = RelayConfig::new("wss://relay.example.com", "./relay.db", keys)
    .with_subdomains(2);  // Extract subdomain from host

// alice.relay.example.com -> data stored in "alice" scope
// bob.relay.example.com -> data stored in "bob" scope
// relay.example.com -> data stored in default scope
```

### Custom State Support

The relay builder supports custom per-connection state, allowing you to track application-specific data:

```rust
use nostr_relay_builder::{RelayBuilder, EventProcessor, NostrConnectionState};

// Define your custom state
#[derive(Debug, Clone, Default)]
struct ConnectionMetrics {
    events_processed: u32,
    rate_limit_tokens: f32,
    user_reputation: u32,
}

// Implement EventProcessor with your state type
#[derive(Debug)]
struct MetricsProcessor;

#[async_trait::async_trait]
impl EventProcessor<ConnectionMetrics> for MetricsProcessor {
    async fn handle_event(
        &self,
        event: Event,
        connection_id: &str,
        state: &mut NostrConnectionState<ConnectionMetrics>,
    ) -> Result<Vec<StoreCommand>> {
        // Access and modify your custom state
        state.custom.events_processed += 1;
        
        // Use custom state for business logic
        if state.custom.rate_limit_tokens < 1.0 {
            return Err(Error::restricted("Rate limit exceeded"));
        }
        
        state.custom.rate_limit_tokens -= 1.0;
        
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            state.subdomain().clone(),
        )])
    }
}

// Build relay with custom state
let builder = RelayBuilder::<ConnectionMetrics>::new(config)
    .with_state_factory(|| ConnectionMetrics::default())
    .build_handler(MetricsProcessor)
    .await?;
```

### Quick Server Deployment (with "axum" feature)

The easiest way to run a relay - no manual HTTP server setup required:

```rust
use nostr_relay_builder::{RelayBuilder, RelayConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let keys = Keys::generate();
    let config = RelayConfig::new("ws://localhost:8080", "./relay.db", keys);
    
    // Run the server with one line!
    RelayBuilder::new(config)
        .run_server(MyEventProcessor)
        .await?;
    Ok(())
}
```

For more control over server configuration:

```rust
// Run server with custom configuration
RelayBuilder::new(config)
    .run_server_with_config(MyEventProcessor, |server| {
        server
            .bind("0.0.0.0:3000")
            .enable_cors()
            .enable_metrics()
            .with_connection_counter()
            .serve_static("/assets", "./static")
            .with_root_html("<h1>My Custom Relay</h1>")
            .with_background_task(|token| async move {
                // Your background task here
            })
            .on_startup(|| async {
                println!("Server started!");
                Ok(())
            })
    })
    .await?;
```

Or build the handler and integrate with an existing application:

```rust
use nostr_relay_builder::RelayWebSocketHandlerExt;

// Build your handler
let handler = RelayBuilder::new(config)
    .with_middleware(Nip09Middleware::new(database.clone()))
    .with_middleware(Nip40ExpirationMiddleware::new())
    .with_middleware(Nip70Middleware)
    .build_handler(processor)
    .await?;

// Option 1: Run as standalone server
handler
    .into_server()
    .enable_cors()
    .run()
    .await?;

// Option 2: Get router for integration
let (router, token, _) = handler
    .into_server()
    .into_router()
    .await?;

// Merge with your existing axum app
let app = Router::new()
    .route("/api/custom", get(my_handler))
    .merge(router);
```

## Architecture

The relay builder is built on top of `websocket_builder` and follows a layered architecture:

```
┌─────────────────────────────────────┐
│         WebSocket Client            │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│   websocket_builder Layer           │
│  - WebSocket transport              │
│  - Middleware pipeline              │
│  - Connection management            │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  nostr_relay_builder Layer          │
│  ┌─────────────────────────────────┐│
│  │ Core Middlewares (Always)       ││
│  │ - Error Handling                ││
│  │ - Logger                        ││
│  │ - Event Verifier                ││
│  └──────────────┬──────────────────┘│
│                 │                    │
│  ┌──────────────▼──────────────────┐│
│  │Protocol Middlewares (Optional)  ││
│  │ - NIP-09 Deletion               ││
│  │ - NIP-40 Expiration             ││
│  │ - NIP-42 Auth                   ││
│  │ - NIP-70 Protected              ││
│  └──────────────┬──────────────────┘│
│                 │                    │
│  ┌──────────────▼──────────────────┐│
│  │     RelayMiddleware             ││
│  │  (Wraps your EventProcessor)    ││
│  └──────────────┬──────────────────┘│
└──────────────────┼──────────────────┘
                   │
┌──────────────────▼──────────────────┐
│    Your EventProcessor impl         │
│  - handle_event()                   │
│  - can_see_event()                  │
│  - verify_filters()                 │
│  - handle_message()                 │
└─────────────────────────────────────┘
```

## EventProcessor vs Middleware

This crate provides two ways to customize your relay:

1. **EventProcessor** (recommended) - High-level API for business logic
2. **Middleware** - Low-level API for protocol extensions

See the [Middleware Guide](docs/middleware_guide.md) for detailed comparison and when to use each.

## EventProcessor Trait

The core of relay customization is the `EventProcessor` trait:

```rust
#[async_trait]
pub trait EventProcessor: Send + Sync + Debug + 'static {
    /// Process incoming events - decide whether to accept/reject
    async fn handle_event(
        &self,
        event: Event,
        state: &mut NostrConnectionState,
    ) -> Result<Vec<StoreCommand>>;

    /// Control event visibility per connection (optional)
    fn can_see_event(
        &self,
        event: &Event,
        state: &NostrConnectionState,
        relay_pubkey: &PublicKey,
    ) -> Result<bool> {
        Ok(true)  // Default: all stored events visible
    }

    /// Validate subscription filters (optional)
    fn verify_filters(
        &self,
        filters: &[Filter],
        authed_pubkey: Option<PublicKey>,
        state: &NostrConnectionState,
    ) -> Result<()> {
        Ok(())  // Default: all filters allowed
    }

    /// Handle custom protocol messages (optional)
    async fn handle_message(
        &self,
        message: ClientMessage,
        state: &mut NostrConnectionState,
    ) -> Result<Vec<RelayMessage>> {
        Ok(vec![])  // Default: use standard handling
    }
}
```

### Quick Example:

```rust
// Simple private relay that only accepts events from whitelisted pubkeys
#[derive(Debug, Clone)]
struct PrivateRelay {
    allowed: HashSet<PublicKey>,
}

#[async_trait]
impl EventProcessor for PrivateRelay {
    async fn handle_event(
        &self,
        event: Event,
        state: &mut NostrConnectionState,
    ) -> Result<Vec<StoreCommand>> {
        if self.allowed.contains(&event.pubkey) {
            Ok(vec![StoreCommand::SaveSignedEvent(
                Box::new(event),
                state.subdomain().clone(),
            )])
        } else {
            Ok(vec![])  // Reject silently
        }
    }
}
```

## Examples

See the `examples/` directory for complete, runnable relay implementations:

### Core Examples (requires "axum" feature)

- **`minimal_relay.rs`** - The simplest possible relay in ~60 lines
  - Shows the minimal code needed to run a Nostr relay
  - Accepts all events without filtering
  - Includes basic server setup with Axum

- **`advanced_relay.rs`** - Feature-rich relay with authentication and moderation
  - NIP-42 authentication required for posting
  - Content moderation with banned words
  - Custom HTML frontend
  - Metrics endpoint, CORS, graceful shutdown
  - Background tasks

- **`private_relay.rs`** - Access-controlled relay with multiple modes
  - Whitelist mode: Only specific pubkeys can post
  - Public read mode: Anyone can read, authenticated users can write
  - Shows different access control strategies

- **`subdomain_relay.rs`** - Multi-tenant relay using subdomains
  - Each subdomain gets isolated data storage
  - Per-tenant access control
  - Demonstrates data isolation patterns

### Extension Examples

- **`custom_middleware.rs`** - How to implement custom protocol extensions
  - Shows how to create custom middleware
  - Example: Rate limiting middleware
  - Can be used to implement new NIPs

### Custom State Examples

- **`custom_state_relay.rs`** - Demonstration of EventProcessor with custom state
  - Shows how to use custom state for rate limiting
  - Per-connection event counting
  - Progressive authentication requirements

- **`advanced_relay_with_state.rs`** - Advanced features with custom state
  - Rate limiting with time windows
  - Event metrics by kind
  - Content filtering with state tracking

- **`custom_state_relay.rs`** - Using RelayBuilder with custom state
  - Connection statistics tracking
  - Session duration monitoring
  - Custom state initialization

- **`custom_state_relay.rs`** - Complete relay with reputation system
  - User reputation tracking (0-100)
  - Dynamic rate limits based on reputation
  - Content filtering with blocked words
  - Subscription limits per user

- **`generic_middleware_demo.rs`** - Generic middleware with custom state
  - Shows how middleware can access custom state
  - Request metrics tracking
  - Error handling with custom state

## Integration with HTTP Servers

The relay builder produces a WebSocket handler that can be integrated with any async HTTP server. 

### Handler-Only Mode (Default)

Build just the WebSocket handler for integration with your existing web framework:

```rust
let handler = RelayBuilder::new(config)
    .with_middleware(Nip09Middleware::new(database.clone()))
    .with_middleware(Nip40ExpirationMiddleware::new())
    .with_middleware(Nip70Middleware)
    .build_handler(logic)
    .await?;

// Now integrate with your web framework of choice
```

### Framework Integration (with "axum" feature)

With the "axum" feature enabled, you get pre-built Axum integrations:

#### Axum Integration

```rust
use nostr_relay_builder::{RelayInfo, RelayBuilder};

// Option 1: Get an Axum-compatible handler function
let root_handler = RelayBuilder::new(config)
    .with_middleware(Nip09Middleware::new(database.clone()))
    .with_middleware(Nip40ExpirationMiddleware::new())
    .with_middleware(Nip70Middleware)
    .build_axum_handler(logic, relay_info, None)
    .await?;

let app = Router::new()
    .route("/", get(root_handler))
    .route("/health", get(|| async { "OK" }));

// Option 2: Build handlers with more control
let handlers = RelayBuilder::new(config)
    .with_middleware(Nip09Middleware::new(database.clone()))
    .with_middleware(Nip40ExpirationMiddleware::new())
    .with_middleware(Nip70Middleware)
    .build_handlers(logic, relay_info, Some(custom_html))
    .await?;

let handlers = Arc::new(handlers);
let app = Router::new()
    .route("/", get(handlers.axum_root_handler()))
    .route("/metrics", get(|| async { "# Metrics\n" }));

// Start your server
axum_server::bind(addr)
    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
    .await?;
```

The handler automatically handles:
- WebSocket upgrades for Nostr clients
- NIP-11 relay information when Accept: application/nostr+json
- HTML response for browsers (customizable)

## License

Licensed under MIT license.