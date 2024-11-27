mod app_state;
mod config;
mod error;
mod group;
mod handler;
mod middlewares;
mod nostr_session_state;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::{
    extract::{ConnectInfo, FromRef, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use clap::Parser;
use dashmap::DashMap;
use middlewares::{
    EventVerifierMiddleware, LoggerMiddleware, Nip29Middleware, Nip42Middleware, Nip70Middleware,
    NostrMessageConverter, RelayForwarder,
};
use nostr_sdk::{ClientMessage, RelayMessage};
use nostr_session_state::{NostrConnectionFactory, NostrConnectionState};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{debug, error, info};
use websocket_builder::{WebSocketBuilder, WebSocketHandler};

#[derive(Parser, Debug)]
#[command(
    name = "Nip 29",
    version = "0.1.0",
    about = "Adds nip 29 functionality to the provided Nostr relay"
)]
struct Args {
    /// Path to config directory
    #[arg(short, long, default_value = "config")]
    config_dir: String,

    /// Override target WebSocket URL
    #[arg(short, long)]
    relay_url: Option<String>,

    /// Override source address
    #[arg(short, long)]
    local_addr: Option<String>,
}

#[derive(Clone)]
pub struct Settings {
    pub relay_url: String,
    pub local_addr: String,
    pub auth_url: String,
    pub admin_keys: Vec<String>,
}

#[derive(Clone)]
struct AppState {
    http_state: Arc<app_state::HttpServerState>,
    ws_handler: Arc<
        WebSocketHandler<
            NostrConnectionState,
            ClientMessage,
            RelayMessage,
            NostrMessageConverter,
            NostrConnectionFactory,
        >,
    >,
    cancellation_token: CancellationToken,
}

impl FromRef<AppState> for Arc<app_state::HttpServerState> {
    fn from_ref(state: &AppState) -> Self {
        state.http_state.clone()
    }
}

impl FromRef<AppState> for CancellationToken {
    fn from_ref(state: &AppState) -> Self {
        state.cancellation_token.clone()
    }
}

impl FromRef<AppState>
    for Arc<
        WebSocketHandler<
            NostrConnectionState,
            ClientMessage,
            RelayMessage,
            NostrMessageConverter,
            NostrConnectionFactory,
        >,
    >
{
    fn from_ref(state: &AppState) -> Self {
        state.ws_handler.clone()
    }
}

// TODO: This needs refactor. Try similar tool as webscoket_builder integration test setup
async fn http_websocket_handler(
    ws: Option<WebSocketUpgrade>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    if let Some(ws) = ws {
        debug!("WebSocket upgrade requested from {}", addr);
        ws.on_upgrade(move |socket| async move {
            match state
                .ws_handler
                .start(socket, addr.to_string(), state.cancellation_token.clone())
                .await
            {
                Ok(_) => debug!("WebSocket connection closed for {}", addr),
                Err(e) => error!("WebSocket error for {}: {:?}", addr, e),
            }
        })
    } else if uri.path() == "/api/groups" {
        debug!("Handling API request for groups");
        handler::handle_get(State(state.http_state.clone()))
            .await
            .into_response()
    } else if let Some(accept_header) = headers.get(axum::http::header::ACCEPT) {
        match accept_header.to_str().unwrap_or_default() {
            "application/nostr+json" => {
                debug!("Handling Nostr JSON request");
                handler::handle_nostr_json(State(state.http_state.clone()))
                    .await
                    .into_response()
            }
            _ => {
                debug!("Serving Vite frontend");
                match ServeDir::new("frontend/dist")
                    .oneshot(
                        Request::builder()
                            .method(Method::GET)
                            .uri("/")
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                {
                    Ok(res) => res.into_response(),
                    Err(_) => {
                        (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
                    }
                }
            }
        }
    } else {
        debug!("Serving Vite frontend");
        match ServeDir::new("frontend/dist")
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
        {
            Ok(res) => res.into_response(),
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response(),
        }
    }
}

#[cfg(feature = "console")]
fn setup_tracing() {
    use tracing_subscriber::prelude::*;
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .compact();

    tracing_subscriber::registry()
        .with(console_subscriber::spawn())
        .with(fmt_layer)
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();
}

#[cfg(not(feature = "console"))]
fn setup_tracing() {
    tracing_subscriber::fmt()
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .compact()
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing();

    let args = Args::parse();
    let cancellation_token = CancellationToken::new();
    let shared_groups = Arc::new(DashMap::new());
    let http_state = Arc::new(app_state::HttpServerState::new(shared_groups.clone()));
    let config = config::Config::new(&args.config_dir).context("Failed to load configuration")?;
    let mut settings = config
        .get_settings()
        .context("Failed to get relay settings")?;

    if let Some(target_url) = args.relay_url {
        settings.relay_url = target_url;
    }
    if let Some(local_addr) = args.local_addr {
        settings.local_addr = local_addr;
    }

    info!("Starting Relay proxy");
    info!("Listening on: {}", settings.local_addr);
    info!("Relay URL: {}", settings.relay_url);

    let relay_keys = settings.relay_keys()?;
    let logger = LoggerMiddleware::new();
    // TODO: this is a temporary solution to verify events while we forward requests to the relay
    let event_verifier = EventVerifierMiddleware;
    let nip_42 = Nip42Middleware::new(settings.auth_url.clone());
    let nip_70 = Nip70Middleware;
    let nip_29 = Nip29Middleware::new(shared_groups.clone(), relay_keys.public_key);
    let relay_connector = RelayForwarder::new(relay_keys);
    let connection_state_factory = NostrConnectionFactory::new(settings.relay_url.clone());

    let websocket_handler = WebSocketBuilder::new(connection_state_factory, NostrMessageConverter)
        .with_middleware(logger)
        .with_middleware(event_verifier)
        .with_middleware(nip_42)
        .with_middleware(nip_70)
        .with_middleware(nip_29)
        .with_middleware(relay_connector)
        .build();

    let app_state = AppState {
        http_state: http_state.clone(),
        ws_handler: Arc::new(websocket_handler),
        cancellation_token: cancellation_token.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = Router::new()
        .route("/", get(http_websocket_handler))
        .route("/api/groups", get(handler::handle_get))
        .nest_service("/assets", ServeDir::new("frontend/dist/assets"))
        .fallback_service(ServeDir::new("frontend/dist"))
        .layer(cors)
        .with_state(app_state);

    let addr = settings.local_addr.parse::<SocketAddr>()?;
    let handle = axum_server::Handle::new();
    let handle_clone = handle.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        info!("Shutdown signal received");
        handle_clone.graceful_shutdown(Some(std::time::Duration::from_secs(5)));
        cancellation_token.cancel();
    });

    info!("Starting server on {}", addr);
    axum_server::bind(addr)
        .handle(handle.clone())
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();

    Ok(())
}
