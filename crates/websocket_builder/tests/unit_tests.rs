use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use websocket_builder::{
    InboundContext, MessageConverter, Middleware, StateFactory, WebSocketBuilder,
};

#[derive(Debug, Clone)]
struct TestState {
    counter: usize,
}

#[derive(Debug, Clone)]
struct TestMessage {
    id: u64,
    content: String,
}

#[derive(Clone)]
struct TestConverter;

impl MessageConverter<TestMessage, TestMessage> for TestConverter {
    fn inbound_from_string(&self, message: String) -> Result<Option<TestMessage>, anyhow::Error> {
        Ok(Some(TestMessage {
            id: 0,
            content: message,
        }))
    }

    fn outbound_to_string(&self, message: TestMessage) -> Result<String, anyhow::Error> {
        Ok(format!("{}:{}", message.id, message.content))
    }
}

#[derive(Clone)]
struct TestStateFactory;

impl StateFactory<TestState> for TestStateFactory {
    fn create_state(&self, _token: CancellationToken) -> TestState {
        TestState { counter: 0 }
    }
}

#[derive(Debug)]
struct CounterMiddleware {
    processed: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl Middleware for CounterMiddleware {
    type State = TestState;
    type IncomingMessage = TestMessage;
    type OutgoingMessage = TestMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.counter += 1;
        self.processed.fetch_add(1, Ordering::Relaxed);
        ctx.next().await
    }
}

#[test]
fn test_builder_creates_handler() {
    let handler = WebSocketBuilder::new(TestStateFactory, TestConverter)
        .with_middleware(CounterMiddleware {
            processed: Arc::new(AtomicU64::new(0)),
        })
        .with_channel_size(100)
        .with_max_connection_time(Duration::from_secs(60))
        .with_max_connections(1000)
        .build();

    // Handler should be created successfully
    let _ = handler; // Just ensure it compiles
}

#[test]
fn test_middleware_order() {
    let middleware1 = Arc::new(AtomicU64::new(0));
    let middleware2 = Arc::new(AtomicU64::new(0));

    let _handler = WebSocketBuilder::new(TestStateFactory, TestConverter)
        .with_middleware(CounterMiddleware {
            processed: middleware1.clone(),
        })
        .with_middleware(CounterMiddleware {
            processed: middleware2.clone(),
        })
        .build();

    // Middleware should be added in order
    // Actual execution test would require WebSocket
}

#[test]
fn test_actor_builder() {
    use websocket_builder::ActorWebSocketBuilder;

    let _handler = ActorWebSocketBuilder::new(TestStateFactory, TestConverter)
        .with_middleware(CounterMiddleware {
            processed: Arc::new(AtomicU64::new(0)),
        })
        .with_channel_size(50)
        .build();

    // Actor-based handler should be created successfully
}
