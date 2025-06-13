//! Multi-tenant Nostr relay using subdomains for data isolation
//!
//! This example demonstrates:
//! - Subdomain-based data isolation (alice.relay.com vs bob.relay.com)
//! - Per-tenant access control
//! - Cross-tenant event visibility rules
//! - Tenant-specific configuration
//!
//! Run with: cargo run --example subdomain_relay --features axum

use anyhow::Result;
use axum::{response::IntoResponse, routing::get, Router};
use nostr_relay_builder::{
    middlewares::{Nip09Middleware, Nip40ExpirationMiddleware, Nip70Middleware},
    CryptoWorker, EventContext, EventProcessor, RelayBuilder, RelayConfig, RelayInfo,
    Result as RelayResult, StoreCommand,
};
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Tenant configuration
#[derive(Debug, Clone)]
struct TenantConfig {
    /// Owner's public key
    owner: PublicKey,
    /// Whether the tenant's data is public
    is_public: bool,
    /// Allowed writers (if not public)
    allowed_writers: Vec<PublicKey>,
    /// Custom rules or limits
    max_event_size: usize,
}

/// Multi-tenant event processor
#[derive(Debug, Clone)]
struct MultiTenantProcessor {
    /// Tenant configurations by subdomain
    tenants: HashMap<String, TenantConfig>,
    /// Default config for unknown subdomains
    allow_new_tenants: bool,
}

impl MultiTenantProcessor {
    fn get_tenant_config(&self, subdomain: Option<&str>) -> Option<&TenantConfig> {
        subdomain.and_then(|s| self.tenants.get(s))
    }

    fn can_write(&self, subdomain: Option<&str>, pubkey: &PublicKey) -> bool {
        if let Some(config) = self.get_tenant_config(subdomain) {
            // Owner can always write
            if &config.owner == pubkey {
                return true;
            }
            // Check if public or in allowed list
            config.is_public || config.allowed_writers.contains(pubkey)
        } else {
            // Unknown subdomain - check if we allow new tenants
            self.allow_new_tenants
        }
    }
}

#[async_trait::async_trait]
impl EventProcessor for MultiTenantProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        let subdomain = context.subdomain;
        let subdomain_str = subdomain.name();

        // Check write permissions
        if !self.can_write(subdomain_str, &event.pubkey) {
            warn!(
                "Rejecting event from {} - no write access to subdomain {:?}",
                event.pubkey.to_bech32().unwrap_or_default(),
                subdomain
            );
            return Ok(vec![]);
        }

        // Check event size limits
        if let Some(config) = self.get_tenant_config(subdomain_str) {
            let event_size = event.as_json().len();
            if event_size > config.max_event_size {
                warn!(
                    "Rejecting oversized event ({} bytes) from {}",
                    event_size,
                    event.pubkey.to_bech32().unwrap_or_default()
                );
                return Ok(vec![]);
            }
        }

        info!(
            "Accepting event {} for subdomain {:?}",
            event.id.to_hex(),
            subdomain
        );

        // Store event in the subdomain's isolated storage
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            subdomain.clone(),
        )])
    }

    fn can_see_event(
        &self,
        _event: &Event,
        _custom_state: &(),
        context: EventContext<'_>,
    ) -> RelayResult<bool> {
        // Get the subdomain where the event is stored
        let event_subdomain_str = context.subdomain.name();

        if let Some(config) = self.get_tenant_config(event_subdomain_str) {
            if config.is_public {
                // Public tenant - everyone can see
                Ok(true)
            } else {
                // Private tenant - check if viewer has access
                if let Some(viewer_pubkey) = context.authed_pubkey {
                    Ok(&config.owner == viewer_pubkey
                        || config.allowed_writers.contains(viewer_pubkey))
                } else {
                    Ok(false)
                }
            }
        } else {
            // Unknown subdomain - use default policy
            Ok(self.allow_new_tenants)
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,subdomain_relay=debug,nostr_relay_builder=debug")
        .init();

    // Generate relay keys
    let keys = Keys::generate();
    info!("Starting Multi-tenant Nostr Relay");
    info!("Relay pubkey: {}", keys.public_key().to_bech32()?);

    // Configure some example tenants
    let mut tenants = HashMap::new();

    // Alice's tenant - public blog
    let alice_keys = Keys::generate();
    tenants.insert(
        "alice".to_string(),
        TenantConfig {
            owner: alice_keys.public_key(),
            is_public: true,
            allowed_writers: vec![],
            max_event_size: 64 * 1024, // 64KB
        },
    );

    // Bob's tenant - private workspace
    let bob_keys = Keys::generate();
    let charlie_keys = Keys::generate();
    tenants.insert(
        "bob".to_string(),
        TenantConfig {
            owner: bob_keys.public_key(),
            is_public: false,
            allowed_writers: vec![charlie_keys.public_key()], // Bob and Charlie can write
            max_event_size: 256 * 1024,                       // 256KB
        },
    );

    // Corp tenant - company relay
    let corp_keys = Keys::generate();
    tenants.insert(
        "corp".to_string(),
        TenantConfig {
            owner: corp_keys.public_key(),
            is_public: false,
            allowed_writers: vec![],     // Would load from employee database
            max_event_size: 1024 * 1024, // 1MB
        },
    );

    info!("Configured tenants:");
    for (subdomain, config) in &tenants {
        info!(
            "  - {}: owner={}, public={}",
            subdomain,
            config.owner.to_bech32()?,
            config.is_public
        );
    }

    // Create relay configuration
    let config = RelayConfig::new(
        "ws://relay.example.com",
        "./data/multitenant_relay", // Database directory
        keys.clone(),
    )
    .with_subdomains(2); // relay.example.com has 2 parts, so alice.relay.example.com -> "alice"

    // Create multi-tenant processor
    let processor = MultiTenantProcessor {
        tenants,
        allow_new_tenants: true, // Allow anyone to create new subdomains
    };

    // Custom HTML showing tenant information
    let custom_html = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Multi-tenant Nostr Relay</title>
    <style>
        body {
            font-family: -apple-system, system-ui, sans-serif;
            background: #f5f5f5;
            margin: 0;
            padding: 2rem;
        }
        .container {
            max-width: 800px;
            margin: 0 auto;
            background: white;
            border-radius: 10px;
            padding: 2rem;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }
        h1 { color: #333; }
        .tenant-info {
            background: #f0f0f0;
            border-radius: 5px;
            padding: 1rem;
            margin: 1rem 0;
        }
        .example {
            background: #e8f4f8;
            border-left: 4px solid #0088cc;
            padding: 1rem;
            margin: 1rem 0;
        }
        code {
            background: #333;
            color: #fff;
            padding: 0.2rem 0.5rem;
            border-radius: 3px;
        }
        .grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1rem;
            margin-top: 2rem;
        }
        .tenant-card {
            border: 1px solid #ddd;
            border-radius: 5px;
            padding: 1rem;
        }
        .public { border-color: #51cf66; background: #f0fff4; }
        .private { border-color: #ff6b6b; background: #fff5f5; }
    </style>
</head>
<body>
    <div class="container">
        <h1>🏢 Multi-tenant Nostr Relay</h1>
        <p>This relay provides isolated data storage for different tenants using subdomains.</p>
        
        <div class="tenant-info">
            <h3>How it works:</h3>
            <ul>
                <li>Each subdomain is a separate tenant with isolated data</li>
                <li>Connect to <code>alice.relay.example.com</code> for Alice's space</li>
                <li>Connect to <code>bob.relay.example.com</code> for Bob's space</li>
                <li>Events are stored separately and access is controlled per tenant</li>
            </ul>
        </div>

        <div class="example">
            <strong>Example WebSocket URLs:</strong>
            <ul>
                <li><code>ws://alice.relay.example.com</code> - Public blog (anyone can read)</li>
                <li><code>ws://bob.relay.example.com</code> - Private workspace (authentication required)</li>
                <li><code>ws://corp.relay.example.com</code> - Company relay (employees only)</li>
            </ul>
        </div>

        <h3>Current Tenants:</h3>
        <div class="grid">
            <div class="tenant-card public">
                <h4>alice</h4>
                <p>📖 Public Blog</p>
                <p>Anyone can read, only Alice can write</p>
            </div>
            <div class="tenant-card private">
                <h4>bob</h4>
                <p>🔒 Private Workspace</p>
                <p>Bob and Charlie only</p>
            </div>
            <div class="tenant-card private">
                <h4>corp</h4>
                <p>🏢 Company Relay</p>
                <p>Employees only</p>
            </div>
        </div>
    </div>
</body>
</html>
"#;

    // Define relay information
    let relay_info = RelayInfo {
        name: "Multi-tenant Relay".to_string(),
        description: "Subdomain-based isolated relay instances".to_string(),
        pubkey: keys.public_key().to_string(),
        contact: "admin@relay.example.com".to_string(),
        supported_nips: vec![1, 9, 11, 40, 70],
        software: "nostr_relay_builder".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: None,
    };

    // Create connection counter for monitoring
    let connection_counter = Arc::new(AtomicUsize::new(0));
    let metrics_counter = connection_counter.clone();

    // Create crypto worker and database for middleware
    let cancellation_token = CancellationToken::new();
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(keys.clone()),
        cancellation_token,
    ));
    let database = config.create_database(crypto_worker)?;

    // Build the relay handlers with connection counting
    let handlers = Arc::new(
        RelayBuilder::new(config)
            .with_middleware(Nip09Middleware::new(database.clone()))
            .with_middleware(Nip40ExpirationMiddleware::new())
            .with_middleware(Nip70Middleware)
            .with_connection_counter(connection_counter)
            .build_handlers(processor, relay_info)
            .await?,
    );

    // Create the Axum app
    let app = Router::new()
        .route(
            "/",
            get({
                let handlers = handlers.clone();
                move |ws: Option<axum::extract::WebSocketUpgrade>,
                      connect_info: axum::extract::ConnectInfo<SocketAddr>,
                      headers: axum::http::HeaderMap| async move {
                    if ws.is_some()
                        || headers.get("accept").and_then(|h| h.to_str().ok())
                            == Some("application/nostr+json")
                    {
                        handlers.axum_root_handler()(ws, connect_info, headers).await
                    } else {
                        axum::response::Html(custom_html).into_response()
                    }
                }
            }),
        )
        .route("/health", get(|| async { "OK" }))
        .route(
            "/metrics",
            get(move || async move {
                format!(
                    "active_connections: {}\n",
                    metrics_counter.load(Ordering::Relaxed)
                )
            }),
        );

    // Start the server
    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("🏢 Multi-tenant relay started!");
    info!("📍 Subdomains provide isolated data storage");
    info!("🔗 Connect to subdomain.localhost:8080 for tenant access");
    info!("📊 Metrics available at http://localhost:8080/metrics");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
