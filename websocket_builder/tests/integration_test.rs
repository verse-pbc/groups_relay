mod utils;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    extract::{ConnectInfo, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
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

#[derive(Default, Debug)]
pub struct ClientState {
    inbound_count: u64,
    outbound_count: u64,
}

pub struct Converter;
impl MessageConverter<String, String> for Converter {
    fn inbound_from_string(&self, payload: String) -> Result<Option<String>, anyhow::Error> {
        Ok(Some(payload))
    }

    fn outbound_to_string(&self, payload: String) -> Result<String, anyhow::Error> {
        Ok(payload)
    }
}

#[derive(Debug)]
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
        ctx.message = format!("One({})", ctx.message);
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.outbound_count += 1;
        ctx.message = Some(format!("Uno({})", ctx.message.as_ref().unwrap()));

        // ctx.next().await would be a no-op here because this is the last outbound middleware
        Ok(())
    }
}

#[derive(Debug)]
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
        ctx.message = format!("Two({})", ctx.message);

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

#[derive(Debug)]
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
        ctx.message = format!("Three({})", ctx.message);

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

#[derive(Debug)]
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

#[derive(Debug)]
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
            println!("FloodMiddleware: Attempting to send message {}", i);
            // We could use capacity() here, but let's let it fail
            // to demonstrate the deadlock scenario is not triggered
            // by the channel being full.

            // if ctx.capacity() == 0 {
            //     println!("FloodMiddleware: Channel is full, skipping message {}", i);
            //     break;
            // }

            match ctx.send_message(format!("flood message {}", i)).await {
                Ok(_) => {
                    println!("FloodMiddleware: Successfully sent message {}", i);
                }
                Err(e) => {
                    println!("FloodMiddleware: Failed to send message {}: {}", i, e);
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

struct TestStateFactory;

impl StateFactory<Arc<Mutex<ClientState>>> for TestStateFactory {
    fn create_state(&self, _token: CancellationToken) -> Arc<Mutex<ClientState>> {
        Arc::new(Mutex::new(ClientState::default()))
    }
}

struct ServerState<
    T: Send + Sync + 'static + std::fmt::Debug,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
    TestStateFactory: StateFactory<T> + Send + Sync + 'static,
> {
    ws_handler: WebSocketHandler<T, I, O, Converter, TestStateFactory>,
    shutdown: CancellationToken,
}

async fn websocket_handler<
    T: Send + Sync + 'static + std::fmt::Debug,
    I: Send + Sync + 'static,
    O: Send + Sync + 'static,
    Converter: MessageConverter<I, O> + Send + Sync + 'static,
    TestStateFactory: StateFactory<T> + Send + Sync + 'static,
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
    server_task: tokio::task::JoinHandle<()>,
    shutdown: CancellationToken,
}

impl TestServer {
    pub async fn start<
        T: Send + Sync + 'static + std::fmt::Debug,
        I: Send + Sync + 'static,
        O: Send + Sync + 'static,
        Converter: MessageConverter<I, O> + Send + Sync + 'static,
        TestStateFactory: StateFactory<T> + Send + Sync + 'static,
    >(
        addr: SocketAddr,
        ws_handler: WebSocketHandler<T, I, O, Converter, TestStateFactory>,
    ) -> Result<Self> {
        let cancellation_token = CancellationToken::new();
        let server_state = ServerState {
            ws_handler,
            shutdown: cancellation_token.clone(),
        };

        let app = Router::new()
            .route("/", get(websocket_handler))
            .with_state(Arc::new(server_state))
            .layer(tower_http::trace::TraceLayer::new_for_http());

        println!("Binding to {}", addr);
        let listener = tokio::net::TcpListener::bind(addr).await?;

        let token = cancellation_token.clone();
        let server_task = tokio::spawn(async move {
            println!("Starting server");
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async move {
                token.cancelled().await;
                println!("Server shutdown triggered");
            })
            .await
            .unwrap();
        });

        // Wait a bit for the server to start
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(Self {
            server_task,
            shutdown: cancellation_token,
        })
    }

    pub async fn shutdown(self) -> Result<(), tokio::task::JoinError> {
        self.shutdown.cancel();
        self.server_task.await
    }
}

#[tokio::test]
async fn test_basic_message_processing() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing basic message processing");

    let addr = SocketAddr::from(([127, 0, 0, 1], 8082));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = TestServer::start(addr, ws_handler).await?;

    let mut client = create_websocket_client(addr.to_string().as_str()).await?;
    assert_proxy_response(
        &mut client,
        "hello",
        "Uno(Dos(Tres(Three(Two(One(hello))))))",
    )
    .await?;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_multiple_client_connections() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing multiple client connections");

    let addr = SocketAddr::from(([127, 0, 0, 1], 8083));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = TestServer::start(addr, ws_handler).await?;

    let mut client1 = create_websocket_client(addr.to_string().as_str()).await?;
    assert_proxy_response(
        &mut client1,
        "hello",
        "Uno(Dos(Tres(Three(Two(One(hello))))))",
    )
    .await?;

    let mut client2 = create_websocket_client(addr.to_string().as_str()).await?;
    assert_proxy_response(
        &mut client2,
        "world",
        "Uno(Dos(Tres(Three(Two(One(world))))))",
    )
    .await?;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_message_processing() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing concurrent message processing");

    let addr = SocketAddr::from(([127, 0, 0, 1], 8084));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = TestServer::start(addr, ws_handler).await?;

    let mut client1 = create_websocket_client(addr.to_string().as_str()).await?;
    let mut client2 = create_websocket_client(addr.to_string().as_str()).await?;

    let (response1, response2) = tokio::join!(
        assert_proxy_response(
            &mut client1,
            "test1",
            "Uno(Dos(Tres(Three(Two(One(test1))))))",
        ),
        assert_proxy_response(
            &mut client2,
            "test2",
            "Uno(Dos(Tres(Three(Two(One(test2))))))",
        ),
    );
    response1?;
    response2?;

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_channel_size_limit() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing channel size limit");
    let addr = SocketAddr::from(([127, 0, 0, 1], 8085));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_channel_size(10)
        .with_middleware(FloodMiddleware)
        .build();

    let server = TestServer::start(addr, ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    client
        .send(Message::Text("trigger flood".to_string()))
        .await?;

    let mut received_count = 0;
    while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(100), client.next()).await
    {
        match msg {
            Ok(Message::Text(_)) => {
                received_count += 1;
            }
            _ => {
                panic!("Received unexpected message: {:?}", msg);
            }
        }
    }

    assert_eq!(
        received_count, 10,
        "Expected to receive exactly 10 messages (channel capacity) got {}",
        received_count
    );

    server.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_message_timeout() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing message timeout");
    let addr = SocketAddr::from(([127, 0, 0, 1], 8086));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_channel_size(10)
        .with_middleware(FloodMiddleware)
        .build();

    let server = TestServer::start(addr, ws_handler).await?;
    let mut client = create_websocket_client(addr.to_string().as_str()).await?;

    client
        .send(Message::Text("trigger flood".to_string()))
        .await?;

    // Receive messages until we get a timeout
    let mut received_count = 0;
    loop {
        let timeout_result = tokio::time::timeout(Duration::from_millis(500), client.next()).await;
        match timeout_result {
            Ok(Some(_)) => {
                received_count += 1;
            }
            Ok(None) => break, // Connection closed
            Err(_) => break,   // Timeout
        }
    }

    // Verify we received at least the channel capacity worth of messages
    assert!(
        received_count >= 10,
        "Expected to receive at least 10 messages, got {}",
        received_count
    );

    // Verify that we timeout when no more messages are available
    let timeout_result = tokio::time::timeout(Duration::from_millis(500), client.next()).await;
    assert!(
        timeout_result.is_err(),
        "Expected timeout error but got {:?}",
        timeout_result
    );

    server.shutdown().await?;
    Ok(())
}
