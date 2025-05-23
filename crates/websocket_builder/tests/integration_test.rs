#[cfg(test)]
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
use utils::{assert_proxy_response, create_test_server, create_websocket_client};
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
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = Some(format!("One({})", ctx.message.as_ref().unwrap()));
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Uno({})", ctx.message.as_ref().unwrap()));
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
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = Some(format!("Two({})", ctx.message.as_ref().unwrap()));
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Dos({})", ctx.message.as_ref().unwrap()));
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
        ctx.state.lock().await.inbound_count += 1;
        let message = format!("Three({})", ctx.message.as_ref().unwrap());
        ctx.message = Some(message.clone());

        // Send the processed message back as a response
        ctx.send_message(message)?;
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Tres({})", ctx.message.as_ref().unwrap()));
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
        ctx.message = Some(format!("Four({})", ctx.message.as_ref().unwrap()));

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
        // Send 200 messages (more than the channel size of 10)
        for i in 0..200 {
            if let Err(_e) = ctx.send_message(format!("flood message {i}")) {
                break;
            }
        }
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
        let cancellation_token = CancellationToken::new();
        let server_state = ServerState {
            ws_handler,
            shutdown: cancellation_token.clone(),
        };

        let app = Router::new()
            .route("/", get(test_websocket_handler))
            .with_state(Arc::new(server_state))
            .layer(tower_http::trace::TraceLayer::new_for_http());

        let listener = tokio::net::TcpListener::bind(addr).await?;

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
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Test basic message flow
    assert_proxy_response(&mut client, "test", "Uno(Dos(Tres(Three(Two(One(test))))))").await?;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_basic_message_processing() -> Result<(), anyhow::Error> {
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Test message processing
    assert_proxy_response(&mut client, "test", "Uno(Dos(Tres(Three(Two(One(test))))))").await?;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_multiple_client_connections() -> Result<(), anyhow::Error> {
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Create multiple clients
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
    }

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_message_processing() -> Result<(), anyhow::Error> {
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client1 = create_websocket_client(addr.to_string().as_str()).await?;
    let mut client2 = create_websocket_client(addr.to_string().as_str()).await?;

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

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_channel_size_limit() -> Result<(), anyhow::Error> {
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .with_channel_size(1)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;

    // Wait a bit for the server to be ready
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    // Send multiple messages rapidly
    for i in 0..5 {
        let message = format!("test{}", i);
        let expected = format!("Uno(Dos(Tres(Three(Two(One({}))))))", message);
        assert_proxy_response(&mut client, &message, &expected).await?;
    }

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_middleware_chain_format() -> Result<(), anyhow::Error> {
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(&addr.to_string()).await?;

    // Send a message and capture the exact format
    client
        .send(Message::Text("test".to_string().into()))
        .await?;

    client.next().await;
    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_flood_middleware_with_backpressure() -> Result<(), anyhow::Error> {
    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_channel_size(10)
        .with_middleware(FloodMiddleware)
        .build();

    let (server, addr) = create_test_server(ws_handler).await?;
    let mut client = create_websocket_client(&addr.to_string()).await?;

    // This message triggers FloodMiddleware to send 200 messages through a channel of size 10
    client
        .send(Message::Text("trigger flood".to_string().into()))
        .await?;

    // We should receive exactly 10 messages (the channel capacity) before the middleware starts dropping messages
    let mut received_count = 0;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(100), client.next()).await
    {
        match msg {
            Ok(Message::Text(msg)) => {
                received_count += 1;
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
