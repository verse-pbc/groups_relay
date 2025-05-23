#[cfg(test)]
mod utils;

use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use utils::{create_test_server, create_websocket_client};
use websocket_builder::{
    InboundContext, MessageConverter, Middleware, OutboundContext, SendMessage, StateFactory,
    WebSocketBuilder,
};

#[derive(Default, Debug, Clone)]
pub struct ErrorState {
    error_count: u64,
}

// A converter that can be configured to fail
#[derive(Clone)]
pub struct ErrorConverter {
    fail_inbound: bool,
    fail_outbound: bool,
}

impl ErrorConverter {
    fn new(fail_inbound: bool, fail_outbound: bool) -> Self {
        Self {
            fail_inbound,
            fail_outbound,
        }
    }
}

impl MessageConverter<String, String> for ErrorConverter {
    fn inbound_from_string(&self, message: String) -> Result<Option<String>, anyhow::Error> {
        if self.fail_inbound {
            Err(anyhow::anyhow!("Inbound conversion failed"))
        } else {
            // Since we can't use async/await here, we'll just return immediately
            Ok(Some(message))
        }
    }

    fn outbound_to_string(&self, message: String) -> Result<String, anyhow::Error> {
        if self.fail_outbound {
            Err(anyhow::anyhow!("Outbound conversion failed"))
        } else {
            Ok(message)
        }
    }
}

// A middleware that can be configured to fail
#[derive(Debug, Clone)]
pub struct ErrorMiddleware {
    should_fail_inbound: bool,
    should_fail_outbound: bool,
}

impl ErrorMiddleware {
    fn new(should_fail_inbound: bool, should_fail_outbound: bool) -> Self {
        Self {
            should_fail_inbound,
            should_fail_outbound,
        }
    }
}

type Error = Box<dyn std::error::Error + Send + Sync>;

#[async_trait]
impl Middleware for ErrorMiddleware {
    type State = Arc<Mutex<ErrorState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.error_count += 1;
        if self.should_fail_inbound {
            Err(anyhow::anyhow!("Simulated inbound processing error"))
        } else {
            let message = format!("Error({})", ctx.message.clone().unwrap());
            ctx.message = Some(message.clone());
            ctx.send_message(message)?;
            ctx.next().await
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if self.should_fail_outbound {
            Err(anyhow::anyhow!("Simulated outbound processing error"))
        } else {
            match &ctx.message {
                Some(message) => {
                    ctx.message = Some(format!("ErrorOut({})", message));
                    ctx.next().await
                }
                None => {
                    // Handle the case when ctx.message is None
                    // For example, log a warning and skip processing
                    println!("Warning: Outbound message is None, skipping processing");
                    ctx.next().await
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct ErrorStateFactory;
impl StateFactory<Arc<Mutex<ErrorState>>> for ErrorStateFactory {
    fn create_state(&self, _token: CancellationToken) -> Arc<Mutex<ErrorState>> {
        Arc::new(Mutex::new(ErrorState::default()))
    }
}

/// A middleware that generates a flood of messages when triggered
#[derive(Debug)]
struct FloodMiddleware;

#[async_trait]
impl Middleware for FloodMiddleware {
    type State = Arc<Mutex<ErrorState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if ctx.message == Some("trigger_flood".to_string()) {
            // Generate a flood of messages to test overflow
            for i in 0..1000 {
                // Add a small delay to ensure messages build up
                tokio::time::sleep(Duration::from_micros(30)).await;
                ctx.send_message(format!("flood_message_{}", i))?;
            }
            Ok(())
        } else {
            ctx.next().await
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }
}

#[tokio::test]
async fn test_message_conversion_error() -> Result<(), anyhow::Error> {
    // Test inbound conversion error
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(true, false))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message - it should fail at conversion
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    // We should receive an error message or connection close
    if let Some(msg) = client.next().await {
        assert!(msg.is_err() || matches!(msg.unwrap(), Message::Close(_)));
    }

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_middleware_error() -> Result<(), anyhow::Error> {
    // Test middleware processing error
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(true, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message - it should fail in middleware
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    // We should receive an error message or connection close
    if let Some(msg) = client.next().await {
        assert!(msg.is_err() || matches!(msg.unwrap(), Message::Close(_)));
    }

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_channel_capacity() -> Result<(), Error> {
    // Create a handler with very small channel size
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(false, false))
        .with_channel_size(1)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Rapidly send multiple messages to test channel capacity
    for _ in 0..5 {
        client
            .send(Message::Text("test".to_string().into()))
            .await?;
    }

    // Wait a bit to let messages process
    tokio::time::sleep(Duration::from_millis(30)).await;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_cancellation() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    // Immediately trigger shutdown
    server.shutdown().await?;

    // Wait a bit for the shutdown to take effect
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;

    // Try to send another message - should fail
    let result = client.send(Message::Text("test".to_string().into())).await;
    assert!(
        result.is_err() || {
            // If send succeeded, the next receive should fail
            match client.next().await {
                Some(Ok(msg)) => msg.is_close(),
                _ => true,
            }
        },
        "Expected send to fail or connection to close after shutdown"
    );

    Ok(())
}

#[tokio::test]
async fn test_outbound_message_conversion_error() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, true))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message - it should fail at outbound conversion
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    // We should receive an error message or connection close
    if let Some(msg) = client.next().await {
        assert!(msg.is_err() || matches!(msg.unwrap(), Message::Close(_)));
    }

    server.shutdown().await?;
    Ok(())
}

/// Tests that the WebSocket handler properly handles message processing deadlock scenarios.
///
/// A deadlock can occur in the following situation:
/// 1. The outgoing message channel is full (or nearly full)
/// 2. A middleware is processing an incoming message and tries to send responses via ctx.send_message()
/// 3. If we keep processing incoming messages without draining the outgoing queue,
///    the middleware will be blocked trying to send responses while holding the incoming message lock
///
/// To prevent this deadlock:
/// - We use a small channel size to force the outgoing queue to fill up quickly
/// - We send messages rapidly to ensure incoming processing keeps running
/// - The handler should detect this condition and reset the connection rather than deadlock
#[tokio::test]
async fn test_channel_overflow() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(FloodMiddleware)
        .with_channel_size(10) // Small channel size to force overflow
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message to trigger the flood middleware
    client
        .send(Message::Text("trigger_flood".to_string().into()))
        .await?;

    let mut received_count = 0;
    let mut error_count = 0;
    let mut connection_reset = false;
    let timeout = Duration::from_millis(100);

    // Try to receive messages until timeout or connection reset
    loop {
        match tokio::time::timeout(timeout, client.next()).await {
            Ok(Some(Ok(_msg))) => {
                received_count += 1;
                if received_count >= 200 {
                    break;
                }
            }
            Ok(Some(Err(e))) => {
                if e.to_string().contains("Connection reset") {
                    println!("Connection reset detected (expected behavior)");
                    connection_reset = true;
                }
                error_count += 1;
                break;
            }
            Ok(None) => break,
            Err(_) => {
                println!("Response timeout reached");
                break;
            }
        }
    }

    println!(
        "Test complete: {} messages received, {} errors",
        received_count, error_count
    );

    // The test passes if either:
    // 1. We get send errors because the outgoing queue is full
    // 2. The connection is reset because the handler detected potential deadlock
    // 3. We received fewer messages than were sent due to overflow
    assert!(
        error_count > 0 || connection_reset || received_count < 200,
        "Expected either send errors, connection reset, or message loss due to overflow"
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_connection_handling() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;

    // Create multiple clients
    let mut clients = vec![];
    for _ in 0..3 {
        let client = create_websocket_client(addr.to_string().as_str()).await?;
        clients.push(client);
    }

    // Close some clients abruptly
    clients.remove(0); // This should trigger a disconnect

    // Send messages with remaining clients
    for (i, client) in clients.iter_mut().enumerate() {
        client
            .send(Message::Text(format!("test from client {}", i + 1).into()))
            .await?;
    }

    // Wait a bit to let messages process
    tokio::time::sleep(Duration::from_millis(30)).await;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_graceful_shutdown() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message
    client
        .send(Message::Text("test before shutdown".to_string().into()))
        .await?;

    // Start shutdown
    println!("Starting server shutdown");
    let shutdown_handle = tokio::spawn(async move { server.shutdown().await });

    // Try to send messages during shutdown
    for i in 0..3 {
        if let Err(e) = client
            .send(Message::Text(format!("test during shutdown {}", i).into()))
            .await
        {
            println!("Send failed during shutdown: {e}");
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    // Wait for shutdown to complete
    shutdown_handle.await??;
    Ok(())
}

#[tokio::test]
async fn test_error_handling_in_subscription_management() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(true, false))
        .with_channel_size(1) // Small channel size to force errors
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send multiple messages to trigger subscription errors
    for i in 0..5 {
        client
            .send(Message::Text(format!("test{}", i).into()))
            .await?;
    }

    // We should receive error responses or connection reset
    let mut error_received = false;
    let mut connection_reset = false;

    for _ in 0..5 {
        match client.next().await {
            Some(Ok(msg)) => {
                if msg.to_string().contains("error") {
                    error_received = true;
                    break;
                }
            }
            Some(Err(e)) => {
                if e.to_string().contains("Connection reset") {
                    connection_reset = true;
                    break;
                }
            }
            None => break,
        }
    }

    assert!(
        error_received || connection_reset,
        "Expected either error response or connection reset"
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_error_handling_in_message_conversion() -> Result<(), Error> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(true, true))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message that should fail conversion
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    // We should receive an error response or connection close
    let mut error_received = false;
    let mut connection_closed = false;

    for _ in 0..3 {
        match client.next().await {
            Some(Ok(msg)) => {
                if msg.is_close() {
                    connection_closed = true;
                    break;
                }
                if msg.to_string().contains("error") {
                    error_received = true;
                    break;
                }
            }
            Some(Err(_)) => {
                connection_closed = true;
                break;
            }
            None => break,
        }
    }

    assert!(
        error_received || connection_closed,
        "Expected either error response or connection close"
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_error_handling_in_middleware_chain() -> Result<(), Box<dyn std::error::Error>> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(true, false))
        .with_middleware(ErrorMiddleware::new(false, true))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send a message that should trigger middleware errors
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    // We should see the error propagate through the middleware chain
    let mut error_count = 0;
    let mut connection_closed = false;

    for _ in 0..3 {
        match client.next().await {
            Some(Ok(msg)) => {
                if msg.is_close() {
                    connection_closed = true;
                    break;
                }
                if msg.to_string().contains("error") {
                    error_count += 1;
                }
            }
            Some(Err(_)) => {
                connection_closed = true;
                break;
            }
            None => break,
        }
    }

    assert!(
        error_count > 0 || connection_closed,
        "Expected errors or connection close from middleware chain"
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_error_handling_in_event_store() -> Result<(), Box<dyn std::error::Error>> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(true, false)) // Force middleware errors
        .with_channel_size(1) // Small channel to force buffer errors
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send messages rapidly to trigger event store errors
    for i in 0..10 {
        if let Err(e) = client
            .send(Message::Text(format!("store_event_{i}").into()))
            .await
        {
            println!("Send error: {e}");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }

    // Check for error responses
    let mut error_count = 0;
    let mut connection_closed = false;
    let timeout = std::time::Duration::from_secs(1);

    while error_count == 0 && !connection_closed {
        match tokio::time::timeout(timeout, client.next()).await {
            Ok(Some(Ok(msg))) => {
                println!("Received message: {msg}");
                if msg.is_close() {
                    connection_closed = true;
                } else if msg.to_string().contains("error") {
                    error_count += 1;
                }
            }
            Ok(Some(Err(e))) => {
                println!("Connection error: {e}");
                connection_closed = true;
            }
            Ok(None) | Err(_) => break,
        }
    }

    assert!(
        error_count > 0 || connection_closed,
        "Expected errors or connection close from event store"
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_error_handling_in_replaceable_events() -> Result<(), Box<dyn std::error::Error>> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(true, false)) // Force middleware errors
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send replaceable events rapidly
    for i in 0..5 {
        if let Err(e) = client
            .send(Message::Text(format!("replaceable_event_{}", i).into()))
            .await
        {
            println!("Send error: {e}");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }

    // Check responses
    let mut response_count = 0;
    let mut error_count = 0;
    let timeout = std::time::Duration::from_secs(1);

    while response_count == 0 && error_count == 0 {
        match tokio::time::timeout(timeout, client.next()).await {
            Ok(Some(Ok(msg))) => {
                println!("Received message: {msg}");
                let msg_str = msg.to_string();
                if msg_str.contains("error") {
                    error_count += 1;
                } else {
                    response_count += 1;
                }
            }
            Ok(Some(Err(e))) => {
                println!("Connection error: {e}");
                error_count += 1;
            }
            Ok(None) | Err(_) => break,
        }
    }

    // We should see either responses or error handling
    assert!(
        response_count > 0 || error_count > 0,
        "Expected either responses or error handling"
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_error_handling_in_connection_state() -> Result<(), Box<dyn std::error::Error>> {
    let ws_handler = WebSocketBuilder::new(ErrorStateFactory, ErrorConverter::new(false, false))
        .with_middleware(ErrorMiddleware::new(false, false))
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Trigger connection state changes
    client
        .send(Message::Text("init".to_string().into()))
        .await?;
    client.send(Message::Close(None)).await?;

    // Attempt to send after close
    let send_result = client
        .send(Message::Text("after_close".to_string().into()))
        .await;
    assert!(send_result.is_err(), "Expected send after close to fail");

    server.shutdown().await?;
    Ok(())
}
