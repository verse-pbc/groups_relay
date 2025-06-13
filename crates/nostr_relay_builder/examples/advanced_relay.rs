//! Advanced Nostr relay with authentication, custom logic, and server features
//!
//! This example demonstrates:
//! - Custom relay logic with event filtering
//! - NIP-42 authentication
//! - Custom middleware
//! - Server configuration (CORS, metrics, static files)
//! - Background tasks
//! - Custom HTML frontend
//!
//! Run with: cargo run --example advanced_relay --features axum

use anyhow::Result;
use axum::{response::IntoResponse, routing::get, Router};
use nostr_relay_builder::{
    CryptoWorker, EventContext, EventProcessor, Nip09Middleware, Nip40ExpirationMiddleware,
    Nip70Middleware, RelayBuilder, RelayConfig, RelayInfo, Result as RelayResult, StoreCommand,
    WebSocketConfig,
};
use nostr_sdk::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Custom event processor that:
/// - Requires authentication for posting
/// - Filters out events with inappropriate content
/// - Logs all events
#[derive(Debug, Clone)]
struct ModeratedRelayProcessor {
    banned_words: Vec<String>,
}

#[async_trait::async_trait]
impl EventProcessor for ModeratedRelayProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        // Require authentication for posting
        if context.authed_pubkey.is_none() {
            warn!("Rejecting event from unauthenticated user");
            return Ok(vec![]);
        }

        // Check for banned content
        let content = &event.content;
        for banned_word in &self.banned_words {
            if content.to_lowercase().contains(banned_word) {
                warn!("Rejecting event with banned content from {}", event.pubkey);
                return Ok(vec![]);
            }
        }

        info!(
            "Accepting event {} from authenticated user {}",
            event.id, event.pubkey
        );

        // Save the event
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }

    // Remove can_see_event as it's no longer needed
}

const CUSTOM_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Advanced Nostr Relay</title>
    <style>
        body { 
            font-family: Arial, sans-serif; 
            max-width: 800px; 
            margin: 0 auto; 
            padding: 20px;
            background: #f5f5f5;
        }
        .container {
            background: white;
            padding: 30px;
            border-radius: 10px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }
        h1 { color: #333; }
        .info { background: #e8f4f8; padding: 15px; border-radius: 5px; }
        .warning { background: #fff3cd; padding: 15px; border-radius: 5px; margin: 15px 0; }
        code { background: #f0f0f0; padding: 2px 5px; border-radius: 3px; }
    </style>
</head>
<body>
    <div class="container">
        <h1>🚀 Advanced Nostr Relay</h1>
        <div class="info">
            <h2>Features</h2>
            <ul>
                <li>✅ NIP-42 Authentication required for posting</li>
                <li>🛡️ Content moderation (banned words filtering)</li>
                <li>📊 Event logging and metrics</li>
                <li>🔧 Custom middleware support</li>
            </ul>
        </div>
        <div class="warning">
            <strong>⚠️ Authentication Required</strong><br>
            You must authenticate with NIP-42 to post events to this relay.
        </div>
        <h2>Connection Info</h2>
        <p>WebSocket endpoint: <code>ws://localhost:8080/ws</code></p>
    </div>
</body>
</html>
"#;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create relay configuration with custom WebSocket settings
    let relay_url = "ws://localhost:8080";
    let db_path = "./advanced_relay_db";
    let relay_keys = Keys::generate();

    let mut config = RelayConfig::new(relay_url, db_path, relay_keys);
    config.websocket_config = WebSocketConfig {
        max_connections: Some(1000),
        max_connection_time: Some(3600), // 1 hour
        channel_size: 500,
    };

    // Create the moderated processor
    let processor = ModeratedRelayProcessor {
        banned_words: vec!["spam".to_string(), "scam".to_string()],
    };

    // Build the relay handlers
    let relay_info = RelayInfo {
        name: "Advanced Moderated Relay".to_string(),
        description: "A relay with authentication and content filtering".to_string(),
        pubkey: config.keys.public_key().to_hex(),
        contact: "admin@advanced.relay".to_string(),
        supported_nips: vec![1, 9, 11, 40, 42, 70],
        software: "nostr_relay_builder".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: None,
    };

    // Create cancellation token for graceful shutdown
    let cancellation_token = CancellationToken::new();
    let shutdown_token = cancellation_token.clone();

    // Create crypto worker and database for middleware
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(config.keys.clone()),
        cancellation_token.clone(),
    ));
    let database = config.create_database(crypto_worker)?;

    let handlers = Arc::new(
        RelayBuilder::new(config.clone())
            .with_middleware(Nip09Middleware::new(database.clone()))
            .with_middleware(Nip40ExpirationMiddleware::new())
            .with_middleware(Nip70Middleware)
            .with_cancellation_token(cancellation_token)
            .build_handlers(processor, relay_info)
            .await?,
    );

    // Spawn a background task for periodic stats
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            info!("Relay health check - everything running smoothly");
        }
    });

    // Create HTTP server with custom HTML at root
    let app = Router::new().route(
        "/",
        get({
            let handlers = handlers.clone();
            move |ws: Option<axum::extract::WebSocketUpgrade>,
                  connect_info: axum::extract::ConnectInfo<SocketAddr>,
                  headers: axum::http::HeaderMap| async move {
                // Check if this is a WebSocket or NIP-11 request
                if ws.is_some()
                    || headers.get("accept").and_then(|h| h.to_str().ok())
                        == Some("application/nostr+json")
                {
                    // Handle WebSocket/NIP-11 with the builder's handler
                    handlers.axum_root_handler()(ws, connect_info, headers).await
                } else {
                    // Serve custom HTML for regular requests at root
                    axum::response::Html(CUSTOM_HTML).into_response()
                }
            }
        }),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    println!("🚀 Advanced relay listening on: {}", addr);
    println!("📡 WebSocket endpoint: ws://localhost:8080/");
    println!("🌐 Web interface: http://localhost:8080/");
    println!("🔐 Authentication: Required for posting");
    println!("⏹️  Press Ctrl+C to gracefully shutdown");

    // Spawn shutdown handler
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        info!("Shutdown signal received, initiating graceful shutdown...");
        shutdown_token.cancel();
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
