# WebSocket Builder

A low-level middleware-based WebSocket framework for building protocol-aware servers in Rust. This crate provides the foundation for protocol implementations like `nostr_relay_builder`. Designed for building stateful connection pipelines with type-safe message processing.

## Core Features

- Bidirectional middleware pipeline for message processing
- Type-safe message conversion between wire format and application types
- Per-connection state management with automatic cleanup
- Built-in cancellation support via `CancellationToken`
- Configurable channel size with backpressure handling

## Installation

Add this to your `Cargo.toml`. Use current stable versions for dependencies.

```toml
[dependencies]
websocket_builder = "0.1.0" # Or your specific version/path
tokio = { version = "1.x", features = ["full"] }
axum = { version = "0.7.x", features = ["ws"] }
async-trait = "0.1.x"
anyhow = "1.0.x" # For Result types
```

## Quick Example: Message Transformation Pipeline

This example demonstrates the core feature: processing messages through a bidirectional middleware pipeline.
An incoming message is transformed by inbound middleware stages. One middleware then echoes the transformed message back.
This echoed message then flows through the outbound middleware stages before being sent to the client.

```rust
use async_trait::async_trait;
use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;
use websocket_builder::{
    InboundContext, MessageConverter, Middleware, OutboundContext, SendMessage, StateFactory,
    WebSocketBuilder, WebSocketHandler,
};
use anyhow::Result;

// 1. Minimal State (not actively used in transformation for this example)
#[derive(Debug, Clone, Default)]
pub struct MinimalState;

// 2. Minimal State Factory
#[derive(Clone)]
pub struct MinimalStateFactory;

impl StateFactory<Arc<MinimalState>> for MinimalStateFactory {
    fn create_state(&self, _token: CancellationToken) -> Arc<MinimalState> {
        Arc::new(MinimalState::default())
    }
}

// 3. Simple String Message Converter
#[derive(Clone, Debug)]
pub struct StringConverter;

impl MessageConverter<String, String> for StringConverter {
    fn inbound_from_string(&self, payload: String) -> Result<Option<String>> {
        Ok(Some(payload))
    }
    fn outbound_to_string(&self, payload: String) -> Result<String> {
        Ok(payload)
    }
}

// 4. Middleware Stages

// StageOneMiddleware: First transformation for inbound and last for outbound.
#[derive(Debug, Clone)]
pub struct StageOneMiddleware;

#[async_trait]
impl Middleware for StageOneMiddleware {
    type State = Arc<MinimalState>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(&self, ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>) -> Result<()> {
        if let Some(msg) = ctx.message.take() { // Take ownership to modify
            ctx.message = Some(format!("S1_IN:{}", msg));
        }
        ctx.next().await
    }

    async fn process_outbound(&self, ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>) -> Result<()> {
        if let Some(msg) = ctx.message.take() { // Take ownership to modify
            ctx.message = Some(format!("S1_OUT:{}", msg));
        }
        ctx.next().await
    }
}

// StageTwoEchoMiddleware: Second transformation, then echoes the message back.
#[derive(Debug, Clone)]
pub struct StageTwoEchoMiddleware;

#[async_trait]
impl Middleware for StageTwoEchoMiddleware {
    type State = Arc<MinimalState>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(&self, ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>) -> Result<()> {
        let processed_message = if let Some(msg) = ctx.message.take() {
            format!("S2_IN:{}", msg)
        } else {
            return ctx.next().await; // Should not happen if previous middleware sets message
        };

        // Echo the fully inbound-processed message. This will go through the outbound pipeline.
        if let Err(e) = ctx.send_message(processed_message.clone()) {
            eprintln!("StageTwoEchoMiddleware: Failed to send echo: {:?}", e);
            // Decide if error should halt further processing or be passed, e.g. by returning Err(e)
        }

        // Set the current inbound message for any potential subsequent *inbound* middleware (none in this example)
        ctx.message = Some(processed_message);
        ctx.next().await
    }

    async fn process_outbound(&self, ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>) -> Result<()> {
        if let Some(msg) = ctx.message.take() { // Take ownership to modify
            ctx.message = Some(format!("S2_OUT:{}", msg));
        }
        ctx.next().await
    }
}

// Type alias for the WebSocketHandler
type PipelineDemoHandler = WebSocketHandler<
    Arc<MinimalState>,
    String,
    String,
    StringConverter,
    MinimalStateFactory,
>;

// 5. Build the WebSocket Handler with the middleware pipeline
fn build_pipeline_handler() -> Arc<PipelineDemoHandler> {
    Arc::new(
        WebSocketBuilder::new(MinimalStateFactory, StringConverter)
            .with_middleware(StageOneMiddleware)      // First in inbound, last in outbound
            .with_middleware(StageTwoEchoMiddleware)  // Second in inbound, first in outbound (for its echo)
            .build(),
    )
}

// 6. Axum Setup
async fn ws_axum_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(handler): State<Arc<PipelineDemoHandler>>,
) -> impl IntoResponse {
    let conn_token = CancellationToken::new();
    ws.on_upgrade(move |socket| async move {
        println!("Client {} connected.", addr);
        if let Err(e) = handler.start(socket, addr.to_string(), conn_token).await {
            eprintln!("Handler error for {}: {:?}", addr, e);
        }
        println!("Client {} disconnected.", addr);
    })
}

#[tokio::main]
async fn main() {
    let pipeline_handler = build_pipeline_handler();
    let app = Router::new()
        .route("/ws", get(ws_axum_handler))
        .with_state(pipeline_handler);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on ws://{}", addr);
    println!("Send a message (e.g., \"hello\") to see the pipeline transformation.");
    println!("Expected output for 'hello': S1_OUT:S2_OUT:S2_IN:S1_IN:hello");

    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

## Error Handling

Errors are propagated through the middleware chain with state preservation:

```rust
pub enum WebsocketError<State> {
    IoError(std::io::Error, State),
    WebsocketError(axum::Error, State),
    HandlerError(Box<dyn std::error::Error + Send + Sync>, State),
    // ...
}
```

## Status

Early-stage project under active development. Breaking changes should be expected.

## License

MIT

