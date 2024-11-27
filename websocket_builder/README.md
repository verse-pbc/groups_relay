# WebSocket Builder

A flexible middleware-based WebSocket handling framework for Rust applications. This library provides a clean and extensible way to build WebSocket servers with customizable message processing pipelines.

## Features

- Middleware-based architecture for processing incoming and outgoing messages
- Type-safe message conversion between wire format and application types
- Per-connection state management
- Graceful connection lifecycle handling
- Built-in cancellation support
- Async/await based design

## Usage

Here's a basic example of how to use the WebSocket Builder:

```rust
use websocket_builder::{WebSocketBuilder, StateFactory, MessageConverter, Middleware};
use async_trait::async_trait;

// 1. Define your connection state
#[derive(Default, Debug)]
struct ClientState {
    message_count: u64,
}

// 2. Create a state factory
struct TestStateFactory;

impl StateFactory<ClientState> for TestStateFactory {
    fn create_state(&self, _token: CancellationToken) -> ClientState {
        ClientState::default()
    }
}

// 3. Implement a message converter
struct MessageConverter;

impl MessageConverter<String, String> for MessageConverter {
    fn inbound_from_string(&self, payload: String) -> Result<Option<String>, anyhow::Error> {
        Ok(Some(payload))
    }

    fn outbound_to_string(&self, payload: String) -> Result<String, anyhow::Error> {
        Ok(payload)
    }
}

// 4. Create your middleware
struct LoggerMiddleware;

#[async_trait]
impl Middleware for LoggerMiddleware {
    type State = ClientState;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!("Received message: {}", ctx.message);
        ctx.next().await
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!("Sending message: {}", ctx.message.as_ref().unwrap());
        ctx.next().await
    }
}

// 5. Build and use the WebSocket handler
let ws_handler = WebSocketBuilder::new(TestStateFactory, MessageConverter)
    .with_middleware(LoggerMiddleware)
    .build();

// 6. Use with your web framework (example using axum)
async fn websocket_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(handler): State<Arc<WebSocketHandler<...>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        handler
            .start(socket, addr.to_string(), CancellationToken::new())
            .await
            .unwrap();
    })
}
```

## Architecture

The WebSocket Builder uses a layered architecture:

1. **State Management**: Each connection maintains its own state through the `StateFactory` trait
2. **Message Conversion**: The `MessageConverter` trait handles conversion between wire format and application types
3. **Middleware Pipeline**: A chain of middleware components that process messages in both directions
4. **Connection Lifecycle**: Handles connection establishment, message processing, and graceful shutdown

## Middleware

Middleware components can:
- Process incoming messages before they reach your application
- Process outgoing messages before they're sent to the client
- Modify the message content
- Access and modify connection state
- Send messages back to the client
- Short-circuit the middleware chain

## License

MIT License
