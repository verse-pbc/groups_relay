//! Built-in server implementation for easy relay deployment
//!
//! This module provides a ready-to-use server implementation that handles:
//! - WebSocket connections
//! - Health checks
//! - Metrics endpoint (optional)
//! - CORS configuration
//! - Graceful shutdown
//! - Custom routes and middleware
//! - Static file serving
//! - Background tasks

use crate::state::CURRENT_REQUEST_HOST;
use crate::DefaultRelayWebSocketHandler;
use anyhow::Result;
use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use std::any::Any;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any as CorsAny, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{error, info};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type BackgroundTask = Box<dyn FnOnce(CancellationToken) -> BoxFuture<'static, ()> + Send>;

/// Builder for creating a Nostr relay server with custom configuration
pub struct RelayServerBuilder {
    handler: Arc<DefaultRelayWebSocketHandler>,
    bind_addr: String,
    static_dirs: Vec<(String, String)>,
    fallback_dir: Option<String>,
    cors_enabled: bool,
    health_enabled: bool,
    metrics_enabled: bool,
    state: Option<Box<dyn Any + Send + Sync>>,
    background_tasks: Vec<BackgroundTask>,
    shutdown_timeout: Duration,
    root_html: Option<String>,
    connection_counter: Option<Arc<AtomicUsize>>,
    on_startup: Option<Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>>,
    on_shutdown: Option<Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>>,
    metrics_handler: Option<Box<dyn Fn() -> String + Send + Sync>>,
}

impl RelayServerBuilder {
    /// Create a new server builder with the given handler
    pub fn new(handler: DefaultRelayWebSocketHandler) -> Self {
        Self {
            handler: Arc::new(handler),
            bind_addr: "0.0.0.0:8080".to_string(),
            static_dirs: Vec::new(),
            fallback_dir: None,
            cors_enabled: false,
            health_enabled: true,
            metrics_enabled: false,
            state: None,
            background_tasks: Vec::new(),
            shutdown_timeout: Duration::from_secs(5),
            root_html: None,
            connection_counter: None,
            on_startup: None,
            on_shutdown: None,
            metrics_handler: None,
        }
    }

    /// Set the bind address (default: "0.0.0.0:8080")
    pub fn bind(mut self, addr: impl Into<String>) -> Self {
        self.bind_addr = addr.into();
        self
    }

    /// Enable CORS with permissive settings
    pub fn enable_cors(mut self) -> Self {
        self.cors_enabled = true;
        self
    }

    /// Enable health check endpoint at /health
    pub fn enable_health(mut self) -> Self {
        self.health_enabled = true;
        self
    }

    /// Enable metrics endpoint at /metrics
    pub fn enable_metrics(mut self) -> Self {
        self.metrics_enabled = true;
        self
    }

    /// Set a custom metrics handler
    pub fn with_metrics_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn() -> String + Send + Sync + 'static,
    {
        self.metrics_enabled = true;
        self.metrics_handler = Some(Box::new(handler));
        self
    }

    /// Serve static files from a directory
    pub fn serve_static(mut self, path: impl Into<String>, dir: impl Into<String>) -> Self {
        self.static_dirs.push((path.into(), dir.into()));
        self
    }

    /// Set a fallback directory for serving static files (like a SPA)
    pub fn fallback_static(mut self, dir: impl Into<String>) -> Self {
        self.fallback_dir = Some(dir.into());
        self
    }

    /// Add custom state that will be available in route handlers
    pub fn with_state<S: Send + Sync + 'static>(mut self, state: S) -> Self {
        self.state = Some(Box::new(state));
        self
    }

    /// Add a background task that runs for the lifetime of the server
    pub fn with_background_task<F, Fut>(mut self, task: F) -> Self
    where
        F: FnOnce(CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.background_tasks
            .push(Box::new(move |token| Box::pin(task(token))));
        self
    }

    /// Set custom HTML for the root endpoint
    pub fn with_root_html(mut self, html: impl Into<String>) -> Self {
        self.root_html = Some(html.into());
        self
    }

    /// Enable connection counting
    pub fn with_connection_counter(mut self) -> Self {
        self.connection_counter = Some(Arc::new(AtomicUsize::new(0)));
        self
    }

    /// Get the connection counter (if enabled)
    pub fn connection_counter(&self) -> Option<Arc<AtomicUsize>> {
        self.connection_counter.clone()
    }

    /// Set shutdown timeout (default: 5 seconds)
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }

    /// Set a callback to run on server startup
    pub fn on_startup<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_startup = Some(Box::new(move || Box::pin(f())));
        self
    }

    /// Set a callback to run on server shutdown
    pub fn on_shutdown<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.on_shutdown = Some(Box::new(move || Box::pin(f())));
        self
    }

    /// Build the router without running the server
    pub async fn into_router(self) -> Result<(Router<()>, CancellationToken, Vec<JoinHandle<()>>)> {
        let cancellation_token = CancellationToken::new();
        let mut background_handles = Vec::new();

        // Run startup callback if provided
        if let Some(startup) = self.on_startup {
            startup().await?;
        }

        // Start background tasks
        for task in self.background_tasks {
            let token = cancellation_token.clone();
            let handle = tokio::spawn(async move {
                task(token).await;
            });
            background_handles.push(handle);
        }

        // Create server state
        let server_state = Arc::new(ServerState {
            ws_handler: self.handler.clone(),
            cancellation_token: cancellation_token.clone(),
            root_html: self.root_html.clone(),
            connection_counter: self.connection_counter.clone(),
            _custom_state: self.state,
            metrics_handler: self.metrics_handler,
        });

        // Build the router
        let mut router = Router::new().route("/", get(handle_websocket));

        // Add health endpoint
        if self.health_enabled {
            router = router.route("/health", get(handle_health));
        }

        // Add metrics endpoint
        if self.metrics_enabled {
            router = router.route("/metrics", get(handle_metrics));
        }

        // Add static file serving
        for (path, dir) in self.static_dirs {
            router = router.nest_service(&path, ServeDir::new(dir));
        }

        // Add fallback static serving
        if let Some(dir) = self.fallback_dir {
            router = router.fallback_service(ServeDir::new(dir));
        }

        // Note: We can't add custom routes here because they expect no state
        // while our handlers expect Arc<ServerState>. This is a limitation
        // of mixing stateful and stateless handlers.

        // Add state to router - this converts Router<Arc<ServerState>> to Router<()>
        let mut router = router.with_state(server_state);

        // Add CORS if enabled
        if self.cors_enabled {
            let cors = CorsLayer::new()
                .allow_origin(CorsAny)
                .allow_methods(CorsAny)
                .allow_headers(CorsAny);
            router = router.layer(cors);
        }

        Ok((router, cancellation_token, background_handles))
    }

    /// Run the server
    pub async fn run(mut self) -> Result<()> {
        let bind_addr = self.bind_addr.clone();
        let shutdown_timeout = self.shutdown_timeout;
        let on_shutdown = self.on_shutdown.take();

        let (router, cancellation_token, background_handles) = self.into_router().await?;

        // Parse bind address
        let addr: SocketAddr = bind_addr.parse()?;

        // Setup graceful shutdown
        let handle = axum_server::Handle::new();
        let handle_clone = handle.clone();
        let shutdown_token = cancellation_token.clone();

        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            info!("Shutdown signal received");

            // Cancel all background tasks
            shutdown_token.cancel();

            // Graceful HTTP shutdown
            handle_clone.graceful_shutdown(Some(shutdown_timeout.into()));
        });

        info!("Starting Nostr relay server on {}", addr);

        // Run the server
        let app = router.into_make_service_with_connect_info::<SocketAddr>();
        axum_server::bind(addr).handle(handle).serve(app).await?;

        // Wait for background tasks to complete
        for handle in background_handles {
            let _ = handle.await;
        }

        // Run shutdown callback if provided
        if let Some(shutdown) = on_shutdown {
            shutdown().await;
        }

        info!("Server shutdown complete");
        Ok(())
    }
}

/// Server state shared across handlers
pub struct ServerState {
    ws_handler: Arc<DefaultRelayWebSocketHandler>,
    cancellation_token: CancellationToken,
    root_html: Option<String>,
    connection_counter: Option<Arc<AtomicUsize>>,
    _custom_state: Option<Box<dyn Any + Send + Sync>>,
    metrics_handler: Option<Box<dyn Fn() -> String + Send + Sync>>,
}

/// Connection counter guard
struct ConnectionCounterGuard {
    counter: Arc<AtomicUsize>,
}

impl ConnectionCounterGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        let count = counter.fetch_add(1, Ordering::SeqCst) + 1;
        info!("New connection. Total active: {}", count);
        Self { counter }
    }
}

impl Drop for ConnectionCounterGuard {
    fn drop(&mut self) {
        let count = self.counter.fetch_sub(1, Ordering::SeqCst) - 1;
        info!("Connection closed. Total active: {}", count);
    }
}

/// Handle WebSocket connections with optional WebSocket upgrade
async fn handle_websocket(
    ws: Option<WebSocketUpgrade>,
    State(state): State<Arc<ServerState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Extract host from headers for subdomain support
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .map(String::from);

    // If no WebSocket upgrade, serve the root page
    if let Some(ws) = ws {
        let _counter = state
            .connection_counter
            .as_ref()
            .map(|c| ConnectionCounterGuard::new(c.clone()));

        ws.on_upgrade(move |socket| async move {
            let connection_id = uuid::Uuid::new_v4().to_string();
            info!(
                "New WebSocket connection from {} (id: {})",
                addr, connection_id
            );

            // Set the host in task-local storage for subdomain extraction
            let ws_handler = state.ws_handler.clone();
            let cancellation_token = state.cancellation_token.clone();

            CURRENT_REQUEST_HOST
                .scope(host, async move {
                    if let Err(e) = ws_handler
                        .start(socket, connection_id.clone(), cancellation_token)
                        .await
                    {
                        error!("WebSocket error for connection {}: {}", connection_id, e);
                    }
                })
                .await;
        })
    } else {
        // Handle non-WebSocket requests to root
        handle_root(State(state)).await.into_response()
    }
}

/// Handle root endpoint
async fn handle_root(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    if let Some(html) = &state.root_html {
        Html(html.clone())
    } else {
        Html(DEFAULT_ROOT_HTML.to_string())
    }
}

/// Handle health check endpoint
async fn handle_health() -> impl IntoResponse {
    StatusCode::OK
}

/// Handle metrics endpoint
async fn handle_metrics(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    if let Some(handler) = &state.metrics_handler {
        handler()
    } else {
        "# Nostr Relay Metrics\n# No metrics handler configured\n".to_string()
    }
}

const DEFAULT_ROOT_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Nostr Relay</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            max-width: 800px;
            margin: 0 auto;
            padding: 2rem;
            background: #f5f5f5;
        }
        .container {
            background: white;
            border-radius: 8px;
            padding: 2rem;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }
        h1 { color: #333; }
        .status { color: #27ae60; font-weight: bold; }
        code {
            background: #f0f0f0;
            padding: 0.2rem 0.4rem;
            border-radius: 3px;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🚀 Nostr Relay</h1>
        <p>Status: <span class="status">Running</span></p>
        <p>Connect with any Nostr client using WebSocket.</p>
    </div>
</body>
</html>
"#;

/// Extension trait for RelayWebSocketHandler
pub trait RelayWebSocketHandlerExt {
    /// Convert this handler into a server builder
    fn into_server(self) -> RelayServerBuilder;
}

impl RelayWebSocketHandlerExt for DefaultRelayWebSocketHandler {
    fn into_server(self) -> RelayServerBuilder {
        RelayServerBuilder::new(self)
    }
}

// Generic implementation for custom state types - converts to default type for server
impl<T> RelayWebSocketHandlerExt for crate::RelayWebSocketHandler<T>
where
    T: Clone + Send + Sync + std::fmt::Debug + 'static,
{
    fn into_server(self) -> RelayServerBuilder {
        // For now, we can't easily support custom state in the server builder
        // This is a limitation that could be addressed in the future
        todo!("Custom state server integration not yet implemented")
    }
}

/// Simple server runner (backwards compatibility)
pub struct RelayServer;

impl RelayServer {
    /// Run a relay server with simple configuration
    pub async fn run(handler: DefaultRelayWebSocketHandler, config: ServerConfig) -> Result<()> {
        use RelayWebSocketHandlerExt;

        handler
            .into_server()
            .bind(config.bind_addr)
            .enable_cors()
            .with_root_html(config.root_html.unwrap_or_default())
            .with_shutdown_timeout(config.shutdown_timeout)
            .run()
            .await
    }

    /// Create a router for integration with existing apps
    pub async fn router(
        handler: DefaultRelayWebSocketHandler,
        _config: ServerConfig,
    ) -> Result<(Router<()>, CancellationToken)> {
        use RelayWebSocketHandlerExt;

        let (router, token, _) = handler.into_server().enable_cors().into_router().await?;
        Ok((router, token))
    }
}

/// Simple server configuration (backwards compatibility)
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub enable_metrics: bool,
    pub enable_health: bool,
    pub root_html: Option<String>,
    pub shutdown_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".to_string(),
            enable_metrics: true,
            enable_health: true,
            root_html: None,
            shutdown_timeout: Duration::from_secs(5),
        }
    }
}
