# WebSocket Builder

A middleware-based WebSocket framework for building protocol-aware servers in Rust. Designed for building stateful connection pipelines with type-safe message processing.

## Core Features

- Bidirectional middleware pipeline for message processing
- Type-safe message conversion between wire format and application types
- Per-connection state management with automatic cleanup
- Built-in cancellation support via `CancellationToken`
- Configurable channel size with backpressure handling

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
websocket_builder = "0.1.0"
tokio = { version = "1.38", features = ["full"] }
tokio-util = { version = "0.7.1", features = ["rt"] }
axum = { version = "0.7", features = ["ws"] }
async-trait = "0.1"
```

## Quick Example

```rust
use websocket_builder::{WebSocketBuilder, StateFactory, MessageConverter, Middleware, WebSocketHandler};
use async_trait::async_trait;
use axum::{
    extract::{WebSocketUpgrade, ConnectInfo, State},
    response::IntoResponse,
};
use std::{net::SocketAddr, sync::Arc};
use tokio_util::sync::CancellationToken;

// 1. Define your state
#[derive(Default)]
struct MyState {
    message_count: u64,
}

// 2. Create a state factory
struct MyStateFactory;
impl StateFactory<MyState> for MyStateFactory {
    fn create_state(&self, _token: CancellationToken) -> MyState {
        MyState::default()
    }
}

// 3. Create a middleware
#[derive(Debug)]
struct LoggerMiddleware;

#[async_trait]
impl Middleware for LoggerMiddleware {
    type State = MyState;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(&self, ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>) -> Result<(), anyhow::Error> {
        println!("Received: {}", ctx.message);
        ctx.next().await
    }
}

// 4. Build handler
let ws_handler = WebSocketBuilder::new(MyStateFactory, JsonConverter)
    .with_middleware(LoggerMiddleware)
    .with_channel_size(100)
    .build();

// 5. Use with axum
async fn ws_route(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(handler): State<Arc<WebSocketHandler<MyState, String, String, JsonConverter, MyStateFactory>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        handler
            .start(socket, addr.to_string(), CancellationToken::new())
            .await
            .unwrap();
    })
}
```

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
