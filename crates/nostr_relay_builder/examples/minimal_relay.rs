//! Minimal Nostr relay - the simplest possible relay that accepts all events
//!
//! This example shows the absolute minimum code needed to run a functional Nostr relay.
//! It accepts all events without any filtering or authentication.
//!
//! Run with: cargo run --example minimal_relay --features axum

use anyhow::Result;
use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo},
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use nostr_relay_builder::{
    EventContext, EventProcessor, RelayBuilder, RelayConfig, RelayInfo, Result as RelayResult,
    StoreCommand,
};
use nostr_sdk::prelude::*;
use std::net::SocketAddr;
use std::sync::Arc;

/// Simple event processor that accepts all events
#[derive(Debug, Clone)]
struct AcceptAllProcessor;

#[async_trait::async_trait]
impl EventProcessor for AcceptAllProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        // Accept all events
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Create relay configuration
    let relay_url = "ws://localhost:8080";
    let db_path = "./minimal_relay_db";
    let relay_keys = Keys::generate();
    let config = RelayConfig::new(relay_url, db_path, relay_keys);

    // Create relay info for NIP-11
    let relay_info = RelayInfo {
        name: "Minimal Relay".to_string(),
        description: "A minimal Nostr relay that accepts all events".to_string(),
        pubkey: config.keys.public_key().to_hex(),
        contact: "admin@minimal.relay".to_string(),
        supported_nips: vec![1],
        software: "nostr_relay_builder".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: None,
    };

    // Build the relay handlers using the new API
    let processor = AcceptAllProcessor;
    let handlers = Arc::new(
        RelayBuilder::new(config.clone())
            .build_handlers(processor, relay_info)
            .await?,
    );

    // Create root handler that serves either WebSocket/NIP-11 or HTML
    let root_handler = {
        let handlers = handlers.clone();
        move |ws: Option<WebSocketUpgrade>,
              connect_info: ConnectInfo<SocketAddr>,
              headers: HeaderMap| async move {
            if ws.is_some()
                || headers
                    .get("accept")
                    .and_then(|h| h.to_str().ok())
                    == Some("application/nostr+json")
            {
                // Handle WebSocket or NIP-11 request
                handlers.axum_root_handler()(ws, connect_info, headers).await
            } else {
                // Serve custom HTML for regular browser requests
                Html(
                    r#"
                    <!DOCTYPE html>
                    <html>
                    <head>
                        <title>Minimal Nostr Relay</title>
                    </head>
                    <body>
                        <h1>🚀 Minimal Nostr Relay</h1>
                        <p>Connect with any Nostr client using WebSocket.</p>
                        <p>WebSocket endpoint: <code>ws://localhost:8080</code></p>
                    </body>
                    </html>
                "#,
                )
                .into_response()
            }
        }
    };

    // Create HTTP server
    let app = Router::new().route("/", get(root_handler));

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    println!("🚀 Minimal relay listening on: {}", addr);
    println!("📡 WebSocket endpoint: ws://localhost:8080");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
