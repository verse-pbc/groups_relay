//! Production-ready Nostr relay with comprehensive features
//!
//! This example demonstrates production patterns including:
//! - Graceful shutdown with cancellation token
//! - Connection counting for metrics and health checks
//! - Separate WebSocket endpoint from HTML
//! - Static asset serving
//! - Health and metrics endpoints
//! - Proper error handling and logging
//!
//! Run with: cargo run --example production_relay --features axum

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use nostr_relay_builder::{
    middlewares::{Nip09Middleware, Nip40ExpirationMiddleware, Nip70Middleware},
    CryptoWorker, EventContext, EventProcessor, RelayBuilder, RelayConfig, RelayInfo,
    Result as RelayResult, StoreCommand, WebSocketConfig,
};
use nostr_sdk::prelude::*;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tracing::{error, info, warn};

/// Production event processor with comprehensive features
#[derive(Debug, Clone)]
struct ProductionProcessor {
    /// Maximum events per second per IP
    #[allow(dead_code)]
    rate_limit: u32,
    /// Maximum payload size in bytes
    max_payload_size: usize,
    /// Require authentication for certain operations
    require_auth_for_deletion: bool,
}

#[async_trait::async_trait]
impl EventProcessor for ProductionProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        // Check payload size
        let event_size = event.as_json().len();
        if event_size > self.max_payload_size {
            warn!(
                "Rejecting oversized event {} ({} bytes) from {}",
                event.id,
                event_size,
                event.pubkey.to_bech32().unwrap_or_default()
            );
            return Err(nostr_relay_builder::Error::restricted(
                "Event payload too large",
            ));
        }

        // Check authentication for deletions
        if event.kind == Kind::EventDeletion
            && self.require_auth_for_deletion
            && context.authed_pubkey.is_none()
        {
            warn!(
                "Rejecting deletion event {} - authentication required",
                event.id
            );
            return Err(nostr_relay_builder::Error::restricted(
                "Authentication required for deletions",
            ));
        }

        info!(
            "Processing event {} (kind {}) from {}",
            event.id,
            event.kind.as_u16(),
            event.pubkey.to_bech32().unwrap_or_default()
        );

        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}

/// Application state for shared resources
#[derive(Clone)]
struct AppState {
    handlers: Arc<RelayHandlers>,
    connection_counter: Arc<AtomicUsize>,
    cancellation_token: CancellationToken,
    start_time: std::time::Instant,
}

/// Production HTML with monitoring dashboard
const PRODUCTION_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Production Nostr Relay</title>
    <style>
        body {
            font-family: -apple-system, system-ui, sans-serif;
            background: #f8f9fa;
            margin: 0;
            padding: 0;
        }
        .header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 2rem;
            text-align: center;
        }
        .container {
            max-width: 1200px;
            margin: 0 auto;
            padding: 2rem;
        }
        .stats-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 1rem;
            margin-bottom: 2rem;
        }
        .stat-card {
            background: white;
            border-radius: 8px;
            padding: 1.5rem;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }
        .stat-card h3 {
            margin: 0 0 0.5rem 0;
            color: #667eea;
        }
        .stat-card .value {
            font-size: 2rem;
            font-weight: bold;
            color: #1a202c;
        }
        .endpoints {
            background: white;
            border-radius: 8px;
            padding: 1.5rem;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }
        .endpoints h2 {
            margin-top: 0;
            color: #1a202c;
        }
        .endpoint-list {
            list-style: none;
            padding: 0;
        }
        .endpoint-list li {
            padding: 0.5rem 0;
            border-bottom: 1px solid #e2e8f0;
        }
        .endpoint-list li:last-child {
            border-bottom: none;
        }
        code {
            background: #f7fafc;
            padding: 0.2rem 0.4rem;
            border-radius: 3px;
            font-family: 'Consolas', 'Monaco', monospace;
            color: #667eea;
        }
        .status-indicator {
            display: inline-block;
            width: 12px;
            height: 12px;
            border-radius: 50%;
            background: #48bb78;
            margin-right: 0.5rem;
            animation: pulse 2s infinite;
        }
        @keyframes pulse {
            0% { opacity: 1; }
            50% { opacity: 0.5; }
            100% { opacity: 1; }
        }
    </style>
    <script>
        // Auto-refresh stats
        async function updateStats() {
            try {
                const response = await fetch('/api/stats');
                const stats = await response.json();
                document.getElementById('connections').textContent = stats.connections;
                document.getElementById('uptime').textContent = formatUptime(stats.uptime_seconds);
            } catch (e) {
                console.error('Failed to update stats:', e);
            }
        }

        function formatUptime(seconds) {
            const days = Math.floor(seconds / 86400);
            const hours = Math.floor((seconds % 86400) / 3600);
            const minutes = Math.floor((seconds % 3600) / 60);
            
            if (days > 0) return `${days}d ${hours}h`;
            if (hours > 0) return `${hours}h ${minutes}m`;
            return `${minutes}m`;
        }

        // Update every 5 seconds
        setInterval(updateStats, 5000);
        
        // Initial update
        window.addEventListener('load', updateStats);
    </script>
</head>
<body>
    <div class="header">
        <h1>⚡ Production Nostr Relay</h1>
        <p>High-performance relay with monitoring and graceful shutdown</p>
    </div>
    
    <div class="container">
        <div class="stats-grid">
            <div class="stat-card">
                <h3><span class="status-indicator"></span>Status</h3>
                <div class="value">Operational</div>
            </div>
            <div class="stat-card">
                <h3>Active Connections</h3>
                <div class="value" id="connections">-</div>
            </div>
            <div class="stat-card">
                <h3>Uptime</h3>
                <div class="value" id="uptime">-</div>
            </div>
        </div>

        <div class="endpoints">
            <h2>Available Endpoints</h2>
            <ul class="endpoint-list">
                <li>
                    <strong>WebSocket:</strong> <code>ws://localhost:8080/</code>
                    <br>Main Nostr protocol endpoint
                </li>
                <li>
                    <strong>NIP-11 Info:</strong> <code>GET /</code> with <code>Accept: application/nostr+json</code>
                    <br>Relay information document
                </li>
                <li>
                    <strong>Health Check:</strong> <code>GET /health</code>
                    <br>Returns 200 OK when healthy
                </li>
                <li>
                    <strong>Metrics:</strong> <code>GET /metrics</code>
                    <br>Prometheus-compatible metrics
                </li>
                <li>
                    <strong>API Stats:</strong> <code>GET /api/stats</code>
                    <br>JSON endpoint for live statistics
                </li>
                <li>
                    <strong>Static Assets:</strong> <code>GET /static/*</code>
                    <br>Serve static files from ./static directory
                </li>
            </ul>
        </div>
    </div>
</body>
</html>
"#;

/// Health check handler
async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    // Check if cancellation has been requested
    if state.cancellation_token.is_cancelled() {
        return (StatusCode::SERVICE_UNAVAILABLE, "Shutting down");
    }

    // Could add more health checks here (database connectivity, etc.)
    (StatusCode::OK, "OK")
}

/// Metrics endpoint (Prometheus-compatible format)
async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let connections = state.connection_counter.load(Ordering::Relaxed);
    let uptime = state.start_time.elapsed().as_secs();

    format!(
        "# HELP nostr_relay_connections Current number of WebSocket connections\n\
         # TYPE nostr_relay_connections gauge\n\
         nostr_relay_connections {}\n\
         \n\
         # HELP nostr_relay_uptime_seconds Relay uptime in seconds\n\
         # TYPE nostr_relay_uptime_seconds counter\n\
         nostr_relay_uptime_seconds {}\n",
        connections, uptime
    )
}

/// JSON stats endpoint for frontend
async fn stats_handler(State(state): State<AppState>) -> impl IntoResponse {
    let connections = state.connection_counter.load(Ordering::Relaxed);
    let uptime = state.start_time.elapsed().as_secs();

    axum::Json(serde_json::json!({
        "connections": connections,
        "uptime_seconds": uptime,
        "status": "operational"
    }))
}

/// Root handler that delegates to relay or serves HTML
async fn root_handler(
    ws: Option<axum::extract::WebSocketUpgrade>,
    connect_info: axum::extract::ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Check if this is a WebSocket or NIP-11 request
    if ws.is_some()
        || headers.get("accept").and_then(|h| h.to_str().ok()) == Some("application/nostr+json")
    {
        // Handle with relay handlers
        state.handlers.axum_root_handler()(ws, connect_info, headers).await
    } else {
        // Serve production dashboard HTML
        Html(PRODUCTION_HTML).into_response()
    }
}

// Type alias to make the code cleaner
type RelayHandlers = nostr_relay_builder::RelayHandlers<()>;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging with environment filter
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,production_relay=debug".into()),
        )
        .init();

    info!("Starting Production Nostr Relay...");

    // Generate relay keys (in production, load from secure storage)
    let keys = Keys::generate();
    info!("Relay public key: {}", keys.public_key());

    // Create relay configuration
    let mut config = RelayConfig::new(
        "ws://localhost:8080",
        "./data/production_relay",
        keys.clone(),
    );

    // Configure WebSocket limits
    config.websocket_config = WebSocketConfig {
        max_connections: Some(10000),
        max_connection_time: Some(86400), // 24 hours
        channel_size: 1000,
    };

    // Create production processor
    let processor = ProductionProcessor {
        rate_limit: 10,               // 10 events per second
        max_payload_size: 512 * 1024, // 512KB
        require_auth_for_deletion: true,
    };

    // Define relay information
    let relay_info = RelayInfo {
        name: "Production Relay".to_string(),
        description: "High-performance Nostr relay with monitoring".to_string(),
        pubkey: keys.public_key().to_hex(),
        contact: "ops@production-relay.com".to_string(),
        supported_nips: vec![1, 9, 11, 40, 42, 45, 50, 70],
        software: "nostr_relay_builder".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: Some("https://production-relay.com/icon.png".to_string()),
    };

    // Create shared resources
    let cancellation_token = CancellationToken::new();
    let connection_counter = Arc::new(AtomicUsize::new(0));

    // Create crypto worker and database for middleware
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(keys.clone()),
        cancellation_token.clone(),
    ));
    let database = config.create_database(crypto_worker)?;

    // Build relay handlers with all production features
    let handlers = Arc::new(
        RelayBuilder::new(config)
            .with_middleware(Nip09Middleware::new(database.clone()))
            .with_middleware(Nip40ExpirationMiddleware::new())
            .with_middleware(Nip70Middleware)
            .with_cancellation_token(cancellation_token.clone())
            .with_connection_counter(connection_counter.clone())
            .build_handlers(processor, relay_info)
            .await?,
    );

    // Create application state
    let app_state = AppState {
        handlers,
        connection_counter,
        cancellation_token: cancellation_token.clone(),
        start_time: std::time::Instant::now(),
    };

    // Build the application with all routes
    let app = Router::new()
        // Main relay endpoint
        .route("/", get(root_handler))
        // Health and monitoring
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_handler))
        .route("/api/stats", get(stats_handler))
        // Static file serving (if ./static directory exists)
        .nest_service("/static", ServeDir::new("./static"))
        // Add shared state
        .with_state(app_state);

    // Server configuration
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));

    info!("🚀 Production relay configuration complete");
    info!("📡 WebSocket: ws://localhost:8080/");
    info!("🌐 Dashboard: http://localhost:8080/");
    info!("📊 Metrics: http://localhost:8080/metrics");
    info!("🏥 Health: http://localhost:8080/health");
    info!("⏹️  Press Ctrl+C for graceful shutdown");

    // Spawn shutdown handler
    let shutdown_token = cancellation_token.clone();
    tokio::spawn(async move {
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                info!("Shutdown signal received, initiating graceful shutdown...");
                shutdown_token.cancel();
            }
            Err(err) => {
                error!("Error listening for shutdown signal: {}", err);
            }
        }
    });

    // Start the server
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Listening on {}", addr);

    // Serve with graceful shutdown
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        cancellation_token.cancelled().await;
        info!("Graceful shutdown initiated");

        // Give connections time to close
        tokio::time::sleep(Duration::from_secs(5)).await;

        info!("Shutdown complete");
    })
    .await?;

    Ok(())
}
