mod utils;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use pretty_assertions::assert_eq;
use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use utils::{assert_proxy_response, create_websocket_client};
use websocket_builder::{
    InboundContext, MessageConverter, Middleware, OutboundContext, SendMessage, StateFactory,
    WebSocketBuilder, WebSocketHandler,
};

#[derive(Default, Debug, Clone)]
pub struct ClientState {
    inbound_count: u64,
    outbound_count: u64,
}

#[derive(Clone)]
pub struct Converter;

impl MessageConverter<String, String> for Converter {
    fn inbound_from_string(&self, payload: String) -> Result<Option<String>, anyhow::Error> {
        Ok(Some(payload))
    }

    fn outbound_to_string(&self, payload: String) -> Result<String, anyhow::Error> {
        Ok(payload)
    }
}

#[derive(Debug, Clone)]
pub struct OneMiddleware;

#[async_trait]
impl Middleware for OneMiddleware {
    type State = Arc<Mutex<ClientState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!(
            "OneMiddleware::process_inbound - Processing message: {}",
            ctx.message
        );
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("One({})", ctx.message);
        println!(
            "OneMiddleware::process_inbound - Modified message: {}",
            ctx.message
        );
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!(
            "OneMiddleware::process_outbound - Processing message: {}",
            ctx.message.as_ref().unwrap()
        );
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Uno({})", ctx.message.as_ref().unwrap()));
        println!(
            "OneMiddleware::process_outbound - Modified message: {}",
            ctx.message.as_ref().unwrap()
        );
        ctx.next().await
    }
}

#[derive(Debug, Clone)]
pub struct TwoMiddleware;

#[async_trait]
impl Middleware for TwoMiddleware {
    type State = Arc<Mutex<ClientState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!(
            "TwoMiddleware::process_inbound - Processing message: {}",
            ctx.message
        );
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("Two({})", ctx.message);
        println!(
            "TwoMiddleware::process_inbound - Modified message: {}",
            ctx.message
        );
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!(
            "TwoMiddleware::process_outbound - Processing message: {}",
            ctx.message.as_ref().unwrap()
        );
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Dos({})", ctx.message.as_ref().unwrap()));
        println!(
            "TwoMiddleware::process_outbound - Modified message: {}",
            ctx.message.as_ref().unwrap()
        );
        ctx.next().await
    }
}

#[derive(Debug, Clone)]
pub struct ThreeMiddleware;

#[async_trait]
impl Middleware for ThreeMiddleware {
    type State = Arc<Mutex<ClientState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!(
            "ThreeMiddleware::process_inbound - Processing message: {}",
            ctx.message
        );
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("Three({})", ctx.message);
        println!(
            "ThreeMiddleware::process_inbound - Modified message: {}",
            ctx.message
        );

        // Send the processed message back as a response
        println!("ThreeMiddleware::process_inbound - Sending response");
        ctx.send_message(ctx.message.clone()).await?;
        println!("ThreeMiddleware::process_inbound - Response sent");

        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!(
            "ThreeMiddleware::process_outbound - Processing message: {}",
            ctx.message.as_ref().unwrap()
        );
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Tres({})", ctx.message.as_ref().unwrap()));
        println!(
            "ThreeMiddleware::process_outbound - Modified message: {}",
            ctx.message.as_ref().unwrap()
        );
        ctx.next().await
    }
}

#[derive(Debug, Clone)]
pub struct FourMiddleware;

#[async_trait]
impl Middleware for FourMiddleware {
    type State = Arc<Mutex<ClientState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("Four({})", ctx.message);

        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Cuatro({})", ctx.message.as_ref().unwrap()));
        ctx.next().await
    }
}

#[derive(Debug, Clone)]
pub struct FloodMiddleware;

#[async_trait]
impl Middleware for FloodMiddleware {
    type State = Arc<Mutex<ClientState>>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        println!("FloodMiddleware: Starting to send 200 messages");
        // Send 200 messages (more than the channel size of 10)
        for i in 0..200 {
            println!("FloodMiddleware: Attempting to send message {i}");
            match ctx.send_message(format!("flood message {i}")).await {
                Ok(()) => {
                    println!("FloodMiddleware: Successfully sent message {i}");
                }
                Err(e) => {
                    println!("FloodMiddleware: Failed to send message {i}: {e}");
                    break;
                }
            }
        }
        println!("FloodMiddleware: Finished sending all messages");
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.message = Some(format!("Flood({})", ctx.message.as_ref().unwrap()));
        ctx.next().await
    }
}

#[derive(Debug, Clone)]
pub struct TestStateFactory;

impl StateFactory<Arc<Mutex<ClientState>>> for TestStateFactory {
    fn create_state(&self, _token: CancellationToken) -> Arc<Mutex<ClientState>> {
        Arc::new(Mutex::new(ClientState::default()))
    }
}

#[derive(Clone)]
#[allow(dead_code)]
struct TestState {
    counter: Arc<AtomicU64>,
}

#[allow(dead_code)]
impl TestState {
    fn new() -> Self {
        Self {
            counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[derive(Clone)]
struct TestConverter;

#[allow(dead_code)]
impl TestConverter {
    fn new() -> Self {
        Self
    }
}

#[derive(Clone)]
pub struct ServerState<T, I, O, Converter, Factory>
where
    T: Send + Sync + Clone + 'static,
    I: Send + Sync + Clone + 'static,
    O: Send + Sync + Clone + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    Factory: StateFactory<T> + Send + Sync + Clone + 'static,
{
    ws_handler: WebSocketHandler<T, I, O, Converter, Factory>,
    shutdown: CancellationToken,
}

#[allow(clippy::type_complexity)]
async fn test_websocket_handler<
    T: Send + Sync + Clone + 'static + std::fmt::Debug,
    I: Send + Sync + Clone + 'static,
    O: Send + Sync + Clone + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + Clone + 'static,
    TestStateFactory: StateFactory<T> + Send + Sync + Clone + 'static,
>(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    server_state: State<Arc<ServerState<T, I, O, Converter, TestStateFactory>>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        server_state
            .ws_handler
            .start(socket, addr.to_string(), server_state.shutdown.clone())
            .await
            .unwrap();
    })
}

pub struct TestServer {
    _server_task: tokio::task::JoinHandle<()>,
    shutdown: CancellationToken,
}

impl TestServer {
    pub async fn start<
        T: Send + Sync + Clone + 'static + std::fmt::Debug,
        I: Send + Sync + Clone + 'static,
        O: Send + Sync + Clone + 'static,
        Converter: MessageConverter<I, O> + Send + Sync + Clone + 'static,
        TestStateFactory: StateFactory<T> + Send + Sync + Clone + 'static,
    >(
        addr: SocketAddr,
        ws_handler: WebSocketHandler<T, I, O, Converter, TestStateFactory>,
    ) -> Result<Self, anyhow::Error> {
        println!("TestServer::start - Creating server state");
        let cancellation_token = CancellationToken::new();
        let server_state = ServerState {
            ws_handler,
            shutdown: cancellation_token.clone(),
        };

        println!("TestServer::start - Creating router");
        let app = Router::new()
            .route("/", get(test_websocket_handler))
            .with_state(Arc::new(server_state))
            .layer(tower_http::trace::TraceLayer::new_for_http());

        println!("TestServer::start - Binding to {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await?;
        println!("TestServer::start - Successfully bound to {}", addr);

        let server_task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Ok(Self {
            _server_task: server_task,
            shutdown: cancellation_token,
        })
    }

    pub async fn shutdown(&self) -> Result<(), anyhow::Error> {
        self.shutdown.cancel();
        Ok(())
    }
}

#[tokio::test]
async fn test_basic_message_flow() -> Result<(), anyhow::Error> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8080));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    println!("Starting server on {}", addr);
    let server = utils::TestServer::start(addr.to_string(), ws_handler).await?;
    println!("Server started, connecting client");

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;
    println!("Client connected");

    // Test basic message flow
    assert_proxy_response(&mut client, "test", "Uno(Dos(Tres(Three(Two(One(test))))))").await?;
    println!("Message flow test completed");

    server.shutdown().await?;
    println!("Server shut down");
    Ok(())
}

#[tokio::test]
async fn test_basic_message_processing() -> Result<(), anyhow::Error> {
    println!("Testing basic message processing");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8082));
    println!("Using address: {}", addr);

    println!("Creating WebSocket handler");
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    println!("Starting test server");
    let server = utils::TestServer::start(addr.to_string(), ws_handler).await?;
    println!("Test server started successfully");

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    println!("Creating WebSocket client");
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;
    println!("WebSocket client connected");

    // Test message processing
    assert_proxy_response(&mut client, "test", "Uno(Dos(Tres(Three(Two(One(test))))))").await?;
    println!("Message processing test completed");

    server.shutdown().await?;
    println!("Server shut down");
    Ok(())
}

#[tokio::test]
async fn test_multiple_client_connections() -> Result<(), anyhow::Error> {
    println!("Testing multiple client connections");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8083));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = utils::TestServer::start(addr.to_string(), ws_handler).await?;
    println!("Server started");

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Create multiple clients
    println!("Creating multiple clients");
    let mut clients = Vec::new();
    for i in 0..3 {
        let client = create_websocket_client(addr.to_string().as_str()).await?;
        clients.push(client);
        println!("Client {} connected", i + 1);
    }

    // Test message processing for each client
    for (i, client) in clients.iter_mut().enumerate() {
        let message = format!("test{}", i);
        let expected = format!("Uno(Dos(Tres(Three(Two(One({}))))))", message);
        assert_proxy_response(client, &message, &expected).await?;
        println!("Client {} message processed successfully", i + 1);
    }

    server.shutdown().await?;
    println!("Server shut down");
    Ok(())
}

#[tokio::test]
async fn test_concurrent_message_processing() -> Result<(), anyhow::Error> {
    println!("Testing concurrent message processing");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8084));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = utils::TestServer::start(addr.to_string(), ws_handler).await?;
    println!("Server started");

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client1 = create_websocket_client(addr.to_string().as_str()).await?;
    let mut client2 = create_websocket_client(addr.to_string().as_str()).await?;
    println!("Clients connected");

    // Send messages concurrently
    let (result1, result2) = tokio::join!(
        assert_proxy_response(
            &mut client1,
            "test1",
            "Uno(Dos(Tres(Three(Two(One(test1))))))"
        ),
        assert_proxy_response(
            &mut client2,
            "test2",
            "Uno(Dos(Tres(Three(Two(One(test2))))))"
        )
    );

    result1?;
    result2?;
    println!("Concurrent message processing completed");

    server.shutdown().await?;
    println!("Server shut down");
    Ok(())
}

#[tokio::test]
async fn test_channel_size_limit() -> Result<(), anyhow::Error> {
    println!("Testing channel size limit");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8085));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .with_channel_size(1)
        .build();

    let server = utils::TestServer::start(addr.to_string(), ws_handler).await?;
    println!("Server started");

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;
    println!("Client connected");

    // Send multiple messages rapidly
    for i in 0..5 {
        let message = format!("test{}", i);
        let expected = format!("Uno(Dos(Tres(Three(Two(One({}))))))", message);
        assert_proxy_response(&mut client, &message, &expected).await?;
        println!("Message {} processed successfully", i + 1);
    }

    server.shutdown().await?;
    println!("Server shut down");
    Ok(())
}

#[tokio::test]
async fn test_middleware_chain_format() -> Result<(), anyhow::Error> {
    println!("Testing middleware chain format");
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 8086));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = utils::TestServer::start(addr.to_string(), ws_handler).await?;
    println!("Server started");

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;
    println!("Client connected");

    // Send a message and capture the exact format
    client.send(Message::Text("test".to_string())).await?;

    if let Some(Ok(Message::Text(response))) = client.next().await {
        println!("Actual response format: {}", response);
        // Now we can use this exact format in our other tests
    }

    server.shutdown().await?;
    println!("Server shut down");
    Ok(())
}

#[tokio::test]
async fn test_flood_middleware_with_backpressure() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting flood middleware test");
    let addr = "127.0.0.1:8089";

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_channel_size(10)
        .with_middleware(FloodMiddleware)
        .build();

    let server = utils::TestServer::start(addr, ws_handler).await?;
    let mut client = create_websocket_client(addr).await?;

    // This message triggers FloodMiddleware to send 200 messages through a channel of size 10
    client
        .send(Message::Text("trigger flood".to_string()))
        .await?;

    // We should receive exactly 10 messages (the channel capacity) before the middleware starts dropping messages
    let mut received_count = 0;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(100), client.next()).await
    {
        match msg {
            Ok(Message::Text(msg)) => {
                received_count += 1;
                println!("Received message: {}", msg);
                assert!(
                    msg.starts_with("flood message "),
                    "Expected message to start with 'flood message', got: {}",
                    msg
                );
                assert!(
                    msg.split_whitespace()
                        .last()
                        .unwrap()
                        .parse::<usize>()
                        .is_ok(),
                    "Expected message to end with a number"
                );
            }
            _ => {
                panic!("Received unexpected message: {:?}", msg);
            }
        }
    }

    assert_eq!(
        received_count, 10,
        "Expected to receive exactly 10 messages (channel capacity) before messages start being dropped, got {}",
        received_count
    );

    server.shutdown().await?;
    Ok(())
}
