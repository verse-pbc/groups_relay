mod utils;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    extract::{ConnectInfo, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use utils::{assert_proxy_response, create_websocket_client};
use websocket_builder::message_handler::MessageConverter;
use websocket_builder::middleware::Middleware;
use websocket_builder::middleware_context::{InboundContext, OutboundContext, SendMessage};
use websocket_builder::{StateFactory, WebSocketBuilder, WebSocketHandler};

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

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("One({})", ctx.message);
        ctx.next().await
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
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

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("Two({})", ctx.message);

        ctx.next().await
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
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

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.inbound_count += 1;
        ctx.message = format!("Three({})", ctx.message);

        ctx.send_message(ctx.message.clone()).await?;

        // ctx.next().await would be a no-op here because this is the last inbound middleware
        Ok(())
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.state.lock().await.outbound_count += 1;
        let payload = ctx.message.as_ref().unwrap();
        ctx.message = Some(format!("Tres({})", payload));
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
async fn test_stateful_message_processing() -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting websocket test");

    let addr = SocketAddr::from(([127, 0, 0, 1], 8082));

    let ws_handler = WebSocketBuilder::new(TestStateFactory, Converter)
        .with_middleware(OneMiddleware)
        .with_middleware(TwoMiddleware)
        .with_middleware(ThreeMiddleware)
        .build();

    let server = TestServer::start(addr, ws_handler).await?;

    // Run test cases
    let mut client1 = create_websocket_client(addr.to_string().as_str()).await?;
    println!("Testing client 1");
    assert_proxy_response(
        &mut client1,
        "hello",
        "Uno(Dos(Tres(Three(Two(One(hello))))))",
    )
    .await?;

    let mut client2 = create_websocket_client(addr.to_string().as_str()).await?;
    println!("Testing client 2");
    assert_proxy_response(
        &mut client2,
        "world",
        "Uno(Dos(Tres(Three(Two(One(world))))))",
    )
    .await?;

    // Test concurrent clients
    println!("Testing concurrent clients");
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
