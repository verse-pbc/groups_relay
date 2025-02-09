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
use groups_relay::{
    app_state, config,
    groups::Groups,
    handler, metrics,
    middlewares::{
        EventStoreMiddleware, EventVerifierMiddleware, LoggerMiddleware, Nip29Middleware,
        Nip42Middleware, Nip70Middleware, NostrMessageConverter, ValidationMiddleware,
    },
    nostr_database::NostrDatabase,
    nostr_session_state::{NostrConnectionFactory, NostrConnectionState},
};
use metrics_exporter_prometheus::PrometheusHandle;
use nostr_sdk::{ClientMessage, RelayMessage};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{debug, error, info};
use websocket_builder::{WebSocketBuilder, WebSocketHandler};

/// A RAII guard for tracking active WebSocket connections
#[derive(Debug)]
struct ConnectionCounter {
    counter: Arc<AtomicUsize>,
    connection_id: String,
}

impl ConnectionCounter {
    fn new(counter: Arc<AtomicUsize>, connection_id: String) -> Self {
        let prev = counter.fetch_add(1, Ordering::SeqCst);
        info!(
            "[{}] New connection. Total active connections: {}",
            connection_id,
            prev + 1
        );
        Self {
            counter,
            connection_id,
        }
    }
}

impl Drop for ConnectionCounter {
    fn drop(&mut self) {
        let prev = self.counter.fetch_sub(1, Ordering::SeqCst);
        info!(
            "[{}] Connection closed. Total active connections: {}",
            self.connection_id,
            prev - 1
        );
    }
}

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

    /// Override authentication URL
    #[arg(short, long)]
    auth_url: Option<String>,
}

#[derive(Clone)]
pub struct Settings {
    pub relay_url: String,
    pub local_addr: String,
    pub auth_url: String,
    pub admin_keys: Vec<String>,
    pub websocket: config::WebSocketSettings,
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
    metrics_handle: PrometheusHandle,
    connection_counter: Arc<AtomicUsize>,
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

impl FromRef<AppState> for PrometheusHandle {
    fn from_ref(state: &AppState) -> Self {
        state.metrics_handle.clone()
    }
}

fn get_real_ip(headers: &axum::http::HeaderMap, socket_addr: SocketAddr) -> String {
    // Try to get the real client IP from X-Forwarded-For header
    let ip = if let Some(forwarded_for) = headers.get("x-forwarded-for") {
        if let Ok(forwarded_str) = forwarded_for.to_str() {
            // Get the first IP in the list (original client IP)
            if let Some(real_ip) = forwarded_str.split(',').next() {
                real_ip.trim().to_string()
            } else {
                socket_addr.ip().to_string()
            }
        } else {
            socket_addr.ip().to_string()
        }
    } else {
        socket_addr.ip().to_string()
    };

    // Always append the port from the socket address to ensure uniqueness
    format!("{}:{}", ip, socket_addr.port())
}

async fn http_websocket_handler(
    ws: Option<WebSocketUpgrade>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> impl IntoResponse {
    // 1. WebSocket upgrade: if the upgrade header is present, upgrade the connection.
    if let Some(ws) = ws {
        let real_ip = get_real_ip(&headers, addr);
        info!("WebSocket upgrade requested from {}", real_ip);

        return ws.on_upgrade(move |socket| async move {
            // Create the connection counter guard - it will be automatically dropped when the connection ends
            let _counter =
                ConnectionCounter::new(state.connection_counter.clone(), real_ip.clone());

            let result = state
                .ws_handler
                .start(socket, real_ip.clone(), state.cancellation_token.clone())
                .await;
            match result {
                Ok(_) => debug!("WebSocket connection closed for {}", real_ip),
                Err(e) => error!("WebSocket error for {}: {:?}", real_ip, e),
            }
        });
    }

    // 2. API endpoint: handle groups if the path matches "/api/groups".
    if uri.path() == "/api/groups" {
        debug!("Handling API request for groups");
        return handler::handle_get_groups(State(state.http_state.clone()))
            .await
            .into_response();
    }

    // 3. Nostr JSON: if the Accept header is "application/nostr+json", serve Nostr JSON.
    if let Some(accept_header) = headers.get(axum::http::header::ACCEPT) {
        if let Ok(value) = accept_header.to_str() {
            if value == "application/nostr+json" {
                debug!("Handling Nostr JSON request");
                return handler::handle_nostr_json(State(state.http_state.clone()))
                    .await
                    .into_response();
            }
        }
    }

    // 4. Fallback: serve the static HTML (Vite frontend).
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
        Ok(response) => response.into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response(),
    }
}

async fn metrics_handler(State(metrics_handle): State<PrometheusHandle>) -> impl IntoResponse {
    metrics_handle.render()
}

fn setup_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,groups_relay=debug,websocket_builder=debug"));

    fmt()
        .with_env_filter(env_filter)
        .with_timer(fmt::time::SystemTime)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_file(false)
        .with_line_number(false)
        .with_level(true)
        .init();
}

#[tracing::instrument]
#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing();

    let args = Args::parse();
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

    if let Some(auth_url) = args.auth_url {
        settings.auth_url = auth_url;
    }

    let relay_keys = settings.relay_keys()?;
    let database = NostrDatabase::new(settings.db_path.clone(), relay_keys.clone())?;
    let database = Arc::new(database);

    let groups = Arc::new(Groups::load_groups(database.clone(), relay_keys.public_key()).await?);

    // Setup metrics
    let metrics_handle = groups_relay::metrics::setup_metrics()?;

    let http_state = Arc::new(app_state::HttpServerState::new(groups.clone()));

    info!(
        "Listening for websocket connections at: {}",
        settings.local_addr
    );
    info!("Frontend URL: {}", settings.local_addr);
    info!("Proxied relay URL: {}", settings.relay_url);
    info!("Auth requests must match this URL: {}", settings.auth_url);

    let logger = LoggerMiddleware::new();
    let event_verifier = EventVerifierMiddleware;
    let nip_42 = Nip42Middleware::new(settings.auth_url.clone());
    let nip_70 = Nip70Middleware;
    let nip_29 = Nip29Middleware::new(groups.clone(), relay_keys.public_key);
    let event_store = EventStoreMiddleware::new(database.clone());
    let connection_state_factory = NostrConnectionFactory::new(settings.relay_url.clone());
    let validation_middleware = ValidationMiddleware::new(relay_keys.public_key);

    let mut websocket_builder =
        WebSocketBuilder::new(connection_state_factory, NostrMessageConverter);

    // Apply WebSocket settings from configuration
    websocket_builder = websocket_builder.with_channel_size(settings.websocket.channel_size());
    if let Some(max_time) = settings.websocket.max_connection_time() {
        websocket_builder = websocket_builder.with_max_connection_time(max_time);
    }

    if let Some(max_conns) = settings.websocket.max_connections() {
        websocket_builder = websocket_builder.with_max_connections(max_conns);
    }

    let websocket_handler = websocket_builder
        .with_middleware(logger)
        .with_middleware(nip_42)
        .with_middleware(validation_middleware)
        .with_middleware(event_verifier)
        .with_middleware(nip_70)
        .with_middleware(nip_29)
        .with_middleware(event_store)
        .build();

    let cancellation_token = CancellationToken::new();
    let app_state = AppState {
        http_state: http_state.clone(),
        ws_handler: Arc::new(websocket_handler),
        cancellation_token: cancellation_token.clone(),
        metrics_handle: metrics_handle.clone(),
        connection_counter: Arc::new(AtomicUsize::new(0)),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = Router::new()
        .route("/", get(http_websocket_handler))
        .route("/health", get(handler::handle_health))
        .route("/api/groups", get(handler::handle_get_groups))
        .route("/metrics", get(metrics_handler))
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

    // Start metrics loop
    let groups_for_metrics = Arc::clone(&groups);
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;

            // Update total groups by privacy settings
            for (private, closed, count) in groups_for_metrics.count_groups_by_privacy() {
                metrics::groups_by_privacy(private, closed).set(count as f64);
            }

            // Update active groups by privacy settings
            match groups_for_metrics.count_active_groups_by_privacy().await {
                Ok(counts) => {
                    for (private, closed, count) in counts {
                        metrics::active_groups_by_privacy(private, closed).set(count as f64);
                    }
                }
                Err(e) => error!("Failed to update active groups metrics: {}", e),
            }
        }
    });

    info!("Starting server on {}", addr);
    axum_server::bind(addr)
        .handle(handle.clone())
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();

    Ok(())
}
