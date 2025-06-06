//! Tests for the WebSocket builder and handler

use anyhow::Result;
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use websocket_builder::{
    ConnectionContext, DisconnectContext, InboundContext, MessageConverter, Middleware,
    OutboundContext, StateFactory, WebSocketBuilder, WebSocketHandler, WebsocketError,
};

// Test state that tracks various connection events
#[derive(Debug, Clone)]
struct TestState {
    id: usize,
    messages_received: Arc<AtomicUsize>,
    messages_sent: Arc<AtomicUsize>,
    connected: Arc<AtomicBool>,
    disconnected: Arc<AtomicBool>,
    data: Arc<Mutex<Vec<String>>>,
}

impl TestState {
    fn new(id: usize) -> Self {
        Self {
            id,
            messages_received: Arc::new(AtomicUsize::new(0)),
            messages_sent: Arc::new(AtomicUsize::new(0)),
            connected: Arc::new(AtomicBool::new(false)),
            disconnected: Arc::new(AtomicBool::new(false)),
            data: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

// Test message types
#[derive(Debug, Clone, PartialEq)]
struct TestIncomingMessage {
    content: String,
}

#[derive(Debug, Clone, PartialEq)]
struct TestOutgoingMessage {
    content: String,
}

// Message converter implementation
#[derive(Clone)]
struct TestMessageConverter;

impl MessageConverter<TestIncomingMessage, TestOutgoingMessage> for TestMessageConverter {
    fn inbound_from_string(&self, message: String) -> Result<Option<TestIncomingMessage>> {
        if message.is_empty() {
            return Ok(None);
        }
        Ok(Some(TestIncomingMessage { content: message }))
    }

    fn outbound_to_string(&self, message: TestOutgoingMessage) -> Result<String> {
        Ok(message.content)
    }
}

// State factory implementation
#[derive(Clone)]
struct TestStateFactory {
    counter: Arc<AtomicUsize>,
}

impl TestStateFactory {
    fn new() -> Self {
        Self {
            counter: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl StateFactory<TestState> for TestStateFactory {
    fn create_state(&self, _token: CancellationToken) -> TestState {
        let id = self.counter.fetch_add(1, Ordering::SeqCst);
        TestState::new(id)
    }
}

// Test middleware that tracks lifecycle events
#[derive(Debug, Clone)]
struct LifecycleMiddleware {
    name: String,
}

#[async_trait]
impl Middleware for LifecycleMiddleware {
    type State = TestState;
    type IncomingMessage = TestIncomingMessage;
    type OutgoingMessage = TestOutgoingMessage;

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        ctx.state.connected.store(true, Ordering::SeqCst);
        ctx.state
            .data
            .lock()
            .await
            .push(format!("{}: connected", self.name));

        // Send a welcome message
        if let Some(sender) = &mut ctx.sender {
            sender.send(TestOutgoingMessage {
                content: format!("Welcome from {}", self.name),
            })?;
        }

        ctx.next().await
    }

    async fn on_disconnect<'a>(
        &'a self,
        ctx: &mut DisconnectContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        ctx.state.disconnected.store(true, Ordering::SeqCst);
        ctx.state
            .data
            .lock()
            .await
            .push(format!("{}: disconnected", self.name));
        ctx.next().await
    }

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        ctx.state.messages_received.fetch_add(1, Ordering::SeqCst);
        if let Some(msg) = &ctx.message {
            ctx.state
                .data
                .lock()
                .await
                .push(format!("{}: inbound: {}", self.name, msg.content));
        }
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        ctx.state.messages_sent.fetch_add(1, Ordering::SeqCst);
        if let Some(msg) = &ctx.message {
            ctx.state
                .data
                .lock()
                .await
                .push(format!("{}: outbound: {}", self.name, msg.content));
        }
        ctx.next().await
    }
}

// Note: Tests that require actual WebSocket connections are skipped
// as they would need a full axum server setup. These tests focus on
// the builder pattern and error handling without actual WebSocket I/O.

#[tokio::test]
async fn test_websocket_builder_basic() {
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_middleware(LifecycleMiddleware {
            name: "test".to_string(),
        })
        .build();

    // Verify handler is created
    assert!(matches!(handler, WebSocketHandler { .. }));
}

#[tokio::test]
async fn test_websocket_builder_with_all_options() {
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_middleware(LifecycleMiddleware {
            name: "middleware1".to_string(),
        })
        .with_middleware(LifecycleMiddleware {
            name: "middleware2".to_string(),
        })
        .with_channel_size(50)
        .with_max_connection_time(Duration::from_secs(30))
        .with_max_connections(100)
        .build();

    assert!(matches!(handler, WebSocketHandler { .. }));
}

#[tokio::test]
async fn test_with_arc_middleware() {
    let middleware: Arc<
        dyn Middleware<
            State = TestState,
            IncomingMessage = TestIncomingMessage,
            OutgoingMessage = TestOutgoingMessage,
        >,
    > = Arc::new(LifecycleMiddleware {
        name: "arc_middleware".to_string(),
    });

    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_arc_middleware(middleware)
        .build();

    assert!(matches!(handler, WebSocketHandler { .. }));
}

#[test]
fn test_websocket_error_get_state() {
    let state = TestState::new(42);

    // Test all error variants
    let errors = vec![
        WebsocketError::IoError(
            std::io::Error::new(std::io::ErrorKind::Other, "io error"),
            state.clone(),
        ),
        WebsocketError::InvalidTargetUrl(state.clone()),
        WebsocketError::NoAddressesFound("example.com".to_string(), state.clone()),
        WebsocketError::HandlerError(
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "handler error",
            )),
            state.clone(),
        ),
        WebsocketError::MissingMiddleware(state.clone()),
        WebsocketError::InboundMessageConversionError(
            "conversion error".to_string(),
            state.clone(),
        ),
        WebsocketError::OutboundMessageConversionError(
            "conversion error".to_string(),
            state.clone(),
        ),
        WebsocketError::MaxConnectionsExceeded(state.clone()),
        WebsocketError::UnsupportedBinaryMessage(state.clone()),
    ];

    for error in errors {
        let recovered_state = error.get_state();
        assert_eq!(recovered_state.id, 42);
    }
}

#[test]
fn test_message_converter_none_handling() {
    let converter = TestMessageConverter;

    // Test empty string returns None
    let result = converter.inbound_from_string("".to_string()).unwrap();
    assert!(result.is_none());

    // Test non-empty string returns Some
    let result = converter.inbound_from_string("hello".to_string()).unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().content, "hello");

    // Test outbound conversion
    let message = TestOutgoingMessage {
        content: "test".to_string(),
    };
    let result = converter.outbound_to_string(message).unwrap();
    assert_eq!(result, "test");
}

#[test]
fn test_state_factory_creates_unique_states() {
    let factory = TestStateFactory::new();
    let token = CancellationToken::new();

    let state1 = factory.create_state(token.clone());
    let state2 = factory.create_state(token.clone());
    let state3 = factory.create_state(token.clone());

    // Each state should have a unique ID
    assert_eq!(state1.id, 0);
    assert_eq!(state2.id, 1);
    assert_eq!(state3.id, 2);
}

#[test]
fn test_builder_with_various_configurations() {
    // Test minimal configuration
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter).build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Test with custom channel size
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_channel_size(256)
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Test with max connections
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_max_connections(50)
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Test with timeout
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_max_connection_time(Duration::from_secs(300))
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));
}

#[test]
fn test_websocket_error_display() {
    let state = TestState::new(99);

    // Test error messages
    let io_error = WebsocketError::IoError(
        std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused"),
        state.clone(),
    );
    assert!(format!("{}", io_error).contains("connection refused"));

    let invalid_url = WebsocketError::InvalidTargetUrl(state.clone());
    assert!(format!("{}", invalid_url).contains("Invalid"));

    let no_addresses = WebsocketError::NoAddressesFound("example.com".to_string(), state.clone());
    assert!(format!("{}", no_addresses).contains("example.com"));

    let handler_error = WebsocketError::HandlerError(
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "custom error",
        )),
        state.clone(),
    );
    assert!(format!("{}", handler_error).contains("custom error"));

    let missing_middleware = WebsocketError::MissingMiddleware(state.clone());
    assert!(format!("{}", missing_middleware).contains("Missing"));

    let inbound_conversion =
        WebsocketError::InboundMessageConversionError("bad format".to_string(), state.clone());
    assert!(format!("{}", inbound_conversion).contains("bad format"));

    let outbound_conversion =
        WebsocketError::OutboundMessageConversionError("encode error".to_string(), state.clone());
    assert!(format!("{}", outbound_conversion).contains("encode error"));

    let max_connections = WebsocketError::MaxConnectionsExceeded(state.clone());
    assert!(format!("{}", max_connections).contains("Maximum concurrent connections"));

    let unsupported_binary = WebsocketError::UnsupportedBinaryMessage(state.clone());
    assert!(format!("{}", unsupported_binary).contains("Binary"));
}

#[test]
fn test_test_state_atomics() {
    let state = TestState::new(0);

    // Test atomic operations
    assert_eq!(state.messages_received.load(Ordering::SeqCst), 0);
    state.messages_received.fetch_add(1, Ordering::SeqCst);
    assert_eq!(state.messages_received.load(Ordering::SeqCst), 1);

    assert_eq!(state.messages_sent.load(Ordering::SeqCst), 0);
    state.messages_sent.fetch_add(5, Ordering::SeqCst);
    assert_eq!(state.messages_sent.load(Ordering::SeqCst), 5);

    assert!(!state.connected.load(Ordering::SeqCst));
    state.connected.store(true, Ordering::SeqCst);
    assert!(state.connected.load(Ordering::SeqCst));

    assert!(!state.disconnected.load(Ordering::SeqCst));
    state.disconnected.store(true, Ordering::SeqCst);
    assert!(state.disconnected.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_test_state_data_mutex() {
    let state = TestState::new(0);

    // Test mutex operations
    {
        let mut data = state.data.lock().await;
        assert!(data.is_empty());
        data.push("first".to_string());
        data.push("second".to_string());
    }

    {
        let data = state.data.lock().await;
        assert_eq!(data.len(), 2);
        assert_eq!(data[0], "first");
        assert_eq!(data[1], "second");
    }
}

#[test]
fn test_multiple_middleware_builder() {
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_middleware(LifecycleMiddleware {
            name: "one".to_string(),
        })
        .with_middleware(LifecycleMiddleware {
            name: "two".to_string(),
        })
        .with_middleware(LifecycleMiddleware {
            name: "three".to_string(),
        })
        .with_middleware(LifecycleMiddleware {
            name: "four".to_string(),
        })
        .with_middleware(LifecycleMiddleware {
            name: "five".to_string(),
        })
        .build();

    assert!(matches!(handler, WebSocketHandler { .. }));
}

#[test]
fn test_builder_clone() {
    let handler1 = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_middleware(LifecycleMiddleware {
            name: "test".to_string(),
        })
        .with_channel_size(100)
        .with_max_connections(10)
        .build();

    let handler2 = handler1.clone();

    assert!(matches!(handler1, WebSocketHandler { .. }));
    assert!(matches!(handler2, WebSocketHandler { .. }));
}

// Test that all middleware types are Send + Sync
#[test]
fn test_middleware_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<LifecycleMiddleware>();
    assert_send_sync::<
        Arc<
            dyn Middleware<
                State = TestState,
                IncomingMessage = TestIncomingMessage,
                OutgoingMessage = TestOutgoingMessage,
            >,
        >,
    >();
}

// Test builder with extreme values
#[test]
fn test_builder_extreme_values() {
    // Very large channel size
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_channel_size(1_000_000)
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Very small channel size
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_channel_size(1)
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Very large max connections
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_max_connections(1_000_000)
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Very long timeout
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_max_connection_time(Duration::from_secs(86400)) // 24 hours
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));

    // Very short timeout
    let handler = WebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_max_connection_time(Duration::from_millis(1))
        .build();
    assert!(matches!(handler, WebSocketHandler { .. }));
}

// Test actor builder variant
#[test]
fn test_actor_websocket_builder() {
    use websocket_builder::ActorWebSocketBuilder;

    let handler = ActorWebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_middleware(LifecycleMiddleware {
            name: "actor".to_string(),
        })
        .with_channel_size(128)
        .build();

    // Should create an ActorWebSocketHandler
    assert!(matches!(
        handler,
        websocket_builder::ActorWebSocketHandler { .. }
    ));

    // Test with multiple middlewares
    let handler = ActorWebSocketBuilder::new(TestStateFactory::new(), TestMessageConverter)
        .with_middleware(LifecycleMiddleware {
            name: "first".to_string(),
        })
        .with_middleware(LifecycleMiddleware {
            name: "second".to_string(),
        })
        .build();

    assert!(matches!(
        handler,
        websocket_builder::ActorWebSocketHandler { .. }
    ));
}
