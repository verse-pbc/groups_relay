//! Private Nostr relay with multiple access control strategies
//!
//! This example demonstrates different approaches to creating a private relay:
//! - Whitelist mode: Only specific pubkeys can post
//! - Paid relay: Requires payment verification
//! - Friends-of-friends: Access based on social graph
//! - Read-only public access with authenticated posting
//!
//! Run with: cargo run --example private_relay --features axum

use anyhow::Result;
use axum::{response::IntoResponse, routing::get, Router};
use nostr_relay_builder::{
    middlewares::{Nip09Middleware, Nip40ExpirationMiddleware, Nip70Middleware},
    AuthConfig, CryptoWorker, EventContext, EventProcessor, RelayBuilder, RelayConfig, RelayInfo,
    Result as RelayResult, StoreCommand,
};
use nostr_sdk::prelude::*;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Different access control strategies
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum AccessMode {
    /// Only whitelisted pubkeys can read and write
    Whitelist(HashSet<PublicKey>),
    /// Anyone can read, only whitelisted can write
    PublicRead(HashSet<PublicKey>),
    /// Paid relay - check payment status (simplified example)
    Paid,
    /// Friends-of-friends based on follow lists
    FriendsOfFriends { root_pubkeys: HashSet<PublicKey> },
}

#[derive(Debug, Clone)]
struct PrivateRelayProcessor {
    mode: AccessMode,
    #[allow(dead_code)]
    relay_pubkey: PublicKey,
}

impl PrivateRelayProcessor {
    fn new(mode: AccessMode, relay_pubkey: PublicKey) -> Self {
        Self { mode, relay_pubkey }
    }

    fn is_authorized(&self, pubkey: &PublicKey, authed_pubkey: Option<&PublicKey>) -> bool {
        match &self.mode {
            AccessMode::Whitelist(allowed) => allowed.contains(pubkey),
            AccessMode::PublicRead(allowed) => allowed.contains(pubkey),
            AccessMode::Paid => {
                // In a real implementation, check payment database
                // For demo, we'll check if they're authenticated
                authed_pubkey.is_some()
            }
            AccessMode::FriendsOfFriends { root_pubkeys } => {
                // Simplified: check if user is in root set or authenticated
                root_pubkeys.contains(pubkey) || authed_pubkey.is_some()
            }
        }
    }
}

#[async_trait::async_trait]
impl EventProcessor for PrivateRelayProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        // Check authorization
        if !self.is_authorized(&event.pubkey, context.authed_pubkey) {
            warn!(
                "Rejecting event from unauthorized pubkey: {}",
                event.pubkey.to_bech32().unwrap_or_default()
            );
            return Ok(vec![]);
        }

        info!(
            "Accepting event {} from authorized user {}",
            event.id.to_hex(),
            event.pubkey.to_bech32().unwrap_or_default()
        );

        // Special handling for NIP-28 channel creation (kind 40)
        if event.kind == Kind::ChannelCreation {
            info!("Channel creation event from authorized user");
        }

        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }

    fn can_see_event(
        &self,
        _event: &Event,
        _custom_state: &(),
        context: EventContext<'_>,
    ) -> RelayResult<bool> {
        match &self.mode {
            AccessMode::PublicRead(_) => Ok(true), // Anyone can read
            _ => {
                // For other modes, must be authorized to read
                if let Some(authed_pubkey) = context.authed_pubkey {
                    Ok(self.is_authorized(authed_pubkey, context.authed_pubkey))
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn verify_filters(
        &self,
        _filters: &[Filter],
        _custom_state: &(),
        context: EventContext<'_>,
    ) -> RelayResult<()> {
        // For public read mode, allow all filters
        if matches!(self.mode, AccessMode::PublicRead(_)) {
            return Ok(());
        }

        // For other modes, require authentication
        if let Some(pubkey) = context.authed_pubkey {
            if self.is_authorized(pubkey, context.authed_pubkey) {
                Ok(())
            } else {
                Err(nostr_relay_builder::Error::internal("Unauthorized access"))
            }
        } else {
            Err(nostr_relay_builder::Error::internal(
                "Authentication required",
            ))
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,private_relay=debug,nostr_relay_builder=debug")
        .init();

    // Generate relay keys
    let keys = Keys::generate();
    let relay_pubkey = keys.public_key();

    info!("Starting Private Nostr Relay");
    info!("Relay pubkey: {}", relay_pubkey.to_bech32()?);

    // Create whitelist of allowed pubkeys
    // In production, load from config file or database
    let mut whitelist = HashSet::new();

    // Add some example pubkeys (in real usage, use actual pubkeys)
    let alice = Keys::generate().public_key();
    let bob = Keys::generate().public_key();
    whitelist.insert(alice);
    whitelist.insert(bob);

    info!("Whitelisted pubkeys:");
    for pubkey in &whitelist {
        info!("  - {}", pubkey.to_bech32()?);
    }

    // Configure relay with authentication
    let config = RelayConfig::new(
        "ws://localhost:8080",
        "./data/private_relay", // Database directory
        keys.clone(),
    )
    .with_auth(AuthConfig {
        relay_url: "ws://localhost:8080".to_string(),
        validate_subdomains: false,
    });

    // Choose access mode (change this to test different modes)
    let access_mode = AccessMode::PublicRead(whitelist);
    let processor = PrivateRelayProcessor::new(access_mode, relay_pubkey);

    // Custom HTML for the private relay
    let custom_html = format!(
        r#"
<!DOCTYPE html>
<html>
<head>
    <title>Private Nostr Relay</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a1a;
            color: #e0e0e0;
            margin: 0;
            padding: 2rem;
            min-height: 100vh;
        }}
        .container {{
            max-width: 600px;
            margin: 0 auto;
            background: #2a2a2a;
            border-radius: 10px;
            padding: 2rem;
            box-shadow: 0 4px 6px rgba(0, 0, 0, 0.3);
        }}
        h1 {{
            color: #f0f0f0;
            border-bottom: 2px solid #444;
            padding-bottom: 1rem;
        }}
        .warning {{
            background: #3a2a2a;
            border-left: 4px solid #ff6b6b;
            padding: 1rem;
            margin: 1rem 0;
        }}
        .info {{
            background: #2a3a2a;
            border-left: 4px solid #51cf66;
            padding: 1rem;
            margin: 1rem 0;
        }}
        code {{
            background: #3a3a3a;
            padding: 0.2rem 0.4rem;
            border-radius: 3px;
            font-family: 'Consolas', 'Monaco', monospace;
        }}
        .pubkey {{
            word-break: break-all;
            font-size: 0.9em;
            opacity: 0.8;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>🔐 Private Nostr Relay</h1>
        <div class="warning">
            <strong>Access Restricted</strong>
            <p>This is a private relay with limited access. Authentication may be required.</p>
        </div>
        <div class="info">
            <strong>Relay Information</strong>
            <p>Mode: Public Read / Authenticated Write</p>
            <p>WebSocket: <code>ws://localhost:8080</code></p>
            <p>Supports: NIP-01, NIP-09, NIP-40, NIP-42 (Auth), NIP-70</p>
        </div>
        <p class="pubkey">Relay pubkey: {}</p>
    </div>
</body>
</html>
"#,
        relay_pubkey.to_bech32().unwrap_or_default()
    );

    // Define relay information
    let relay_info = RelayInfo {
        name: "Private Relay".to_string(),
        description: "Access-controlled relay with authentication".to_string(),
        pubkey: relay_pubkey.to_string(),
        contact: "admin@private-relay.com".to_string(),
        supported_nips: vec![1, 9, 11, 40, 42, 70],
        software: "nostr_relay_builder".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: None,
    };

    // Create crypto worker and database for middleware
    let cancellation_token = CancellationToken::new();
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(keys.clone()),
        cancellation_token,
    ));
    let database = config.create_database(crypto_worker)?;

    // Build the relay handlers
    let handlers = Arc::new(
        RelayBuilder::new(config)
            .with_middleware(Nip09Middleware::new(database.clone()))
            .with_middleware(Nip40ExpirationMiddleware::new())
            .with_middleware(Nip70Middleware)
            .build_handlers(processor, relay_info)
            .await?,
    );

    // Create the Axum app
    let app = Router::new()
        .route(
            "/",
            get({
                let handlers = handlers.clone();
                let html = custom_html.clone();
                move |ws: Option<axum::extract::WebSocketUpgrade>,
                      connect_info: axum::extract::ConnectInfo<SocketAddr>,
                      headers: axum::http::HeaderMap| async move {
                    if ws.is_some()
                        || headers.get("accept").and_then(|h| h.to_str().ok())
                            == Some("application/nostr+json")
                    {
                        handlers.axum_root_handler()(ws, connect_info, headers).await
                    } else {
                        axum::response::Html(html).into_response()
                    }
                }
            }),
        )
        .route("/health", get(|| async { "OK" }));

    // Start the server
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("🔐 Private relay started successfully!");
    info!("📡 Connect with authenticated Nostr client to post");
    info!("👁️  Public users can read but not write");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
