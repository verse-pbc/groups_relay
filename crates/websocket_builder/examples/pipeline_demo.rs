use anyhow::Result;
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

// 1. Minimal State (not actively used in transformation for this example)
#[derive(Debug, Clone, Default)]
pub struct MinimalState;

// 2. Minimal State Factory
#[derive(Clone)]
pub struct MinimalStateFactory;

impl StateFactory<Arc<MinimalState>> for MinimalStateFactory {
    fn create_state(&self, _token: CancellationToken) -> Arc<MinimalState> {
        Arc::new(MinimalState)
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

// TrimMiddleware: Trims whitespace on inbound. Passes message through on outbound.
#[derive(Debug, Clone)]
pub struct TrimMiddleware;

#[async_trait]
impl Middleware for TrimMiddleware {
    type State = Arc<MinimalState>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        if let Some(msg) = ctx.message.take() {
            ctx.message = Some(msg.trim().to_string());
        }
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        // Just pass the message through, no prefix added.
        // If ctx.message is None, it means a previous middleware consumed it or didn't set one,
        // which is fine, next() will handle it.
        // If Some(msg), it will be passed to the next outbound middleware or to the client.
        ctx.next().await
    }
}

// HelloEchoMiddleware: Prepends "Hello " on inbound (capitalizing name) and echoes. Appends emoji on outbound.
#[derive(Debug, Clone)]
pub struct HelloEchoMiddleware;

#[async_trait]
impl Middleware for HelloEchoMiddleware {
    type State = Arc<MinimalState>;
    type IncomingMessage = String;
    type OutgoingMessage = String;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        let processed_message = if let Some(name) = ctx.message.take() {
            // Name is already trimmed by TrimMiddleware
            let capitalized_name = if name.is_empty() {
                String::new()
            } else {
                let mut chars = name.chars();
                match chars.next() {
                    None => String::new(), // Should not happen if !name.is_empty()
                    Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
                }
            };
            format!("Hello {}", capitalized_name)
        } else {
            return ctx.next().await; // Should not happen if previous middleware sets message
        };

        // Echo the fully inbound-processed message. This will go through the outbound pipeline.
        if let Err(e) = ctx.send_message(processed_message.clone()) {
            eprintln!("HelloEchoMiddleware: Failed to send echo: {:?}", e);
        }

        // Set the current inbound message for any potential subsequent *inbound* middleware (none in this example)
        ctx.message = Some(processed_message);
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        if let Some(msg) = ctx.message.take() {
            ctx.message = Some(format!("{} 👋", msg));
        }
        ctx.next().await
    }
}

// Type alias for the WebSocketHandler
type PipelineDemoHandler =
    WebSocketHandler<Arc<MinimalState>, String, String, StringConverter, MinimalStateFactory>;

// 5. Build the WebSocket Handler with the middleware pipeline
fn build_pipeline_handler() -> Arc<PipelineDemoHandler> {
    Arc::new(
        WebSocketBuilder::new(MinimalStateFactory, StringConverter)
            .with_middleware(TrimMiddleware) // First in inbound, last in outbound
            .with_middleware(HelloEchoMiddleware) // Second in inbound, first in outbound (for its echo)
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
    println!("Run: websocat ws://127.0.0.1:3000/ws  # Then send: '  daniel  '  (expect: Hello Daniel 👋)");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
