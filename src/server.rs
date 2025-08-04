use crate::{
    app_state::HttpServerState, config, groups::Groups,
    groups_event_processor::GroupsRelayProcessor, handler, metrics,
    metrics_handler::PrometheusSubscriptionMetricsHandler,
    sampled_metrics_handler::SampledMetricsHandler, RelayDatabase,
};
use anyhow::Result;
use axum::{response::IntoResponse, routing::get, Router};
use relay_builder::{
    CryptoHelper, Nip40ExpirationMiddleware, Nip70Middleware, RelayBuilder, 
    RelayConfig, RelayInfo, WebSocketConfig,
};
use websocket_builder::{handle_upgrade, HandlerFactory, WebSocketUpgrade};
use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::{error, info};

pub struct ServerState {
    pub http_state: Arc<HttpServerState>,
    pub cancellation_token: CancellationToken,
    pub metrics_handle: metrics::PrometheusHandle,
    pub connection_counter: Arc<AtomicUsize>,
    pub relay_url: String,
}

pub async fn run_server(
    settings: config::Settings,
    relay_keys: config::Keys,
    database: Arc<RelayDatabase>,
    groups: Arc<Groups>,
) -> Result<()> {
    // Setup metrics
    let metrics_handle = metrics::setup_metrics()?;
    let http_state = Arc::new(HttpServerState::new(groups.clone()));

    info!(
        "Listening for websocket connections at: {}",
        settings.local_addr
    );
    info!("Frontend URL: {}", settings.local_addr);
    info!("Relay URL: {}", settings.relay_url);
    info!(
        "Auth requests must match: {} (with matching subdomain if present)",
        settings.relay_url
    );

    // Build the relay configuration
    let websocket_config = WebSocketConfig {
        max_connections: settings.websocket.max_connections(),
        max_connection_time: settings.websocket.max_connection_time.map(|d| d.as_secs()),
    };

    let _crypto_helper = CryptoHelper::new(Arc::new(relay_keys.clone()));
    let mut relay_config = RelayConfig::new(settings.relay_url.clone(), database, relay_keys.clone())
        .with_subdomains_from_url(&settings.relay_url)
        .with_websocket_config(websocket_config)
        .with_subscription_limits(settings.max_subscriptions, settings.max_limit);
    
    // Enable NIP-42 authentication
    relay_config.enable_auth = true;

    let groups_processor = GroupsRelayProcessor::new(groups.clone(), relay_keys.public_key);

    // Create cancellation token and connection counter
    let cancellation_token = CancellationToken::new();
    let connection_counter = Arc::new(AtomicUsize::new(0));

    // Define relay information
    let _relay_info = RelayInfo {
        name: "Nostr Groups Relay".to_string(),
        description: "A specialized relay implementing NIP-29 for Nostr group management. This relay is under development and all data may be deleted in the future".to_string(),
        pubkey: relay_keys.public_key.to_string(),
        contact: "https://daniel.nos.social".to_string(),
        supported_nips: vec![1, 9, 11, 29, 40, 42, 70],
        software: "groups_relay".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: Some("https://pfp.nostr.build/c60f4853a6d4ae046bdbbd935f0ccd7354c9c411c324b411666d325562a5a906.png".to_string()),
    };

    // Build the relay service
    let handler_factory = Arc::new(
        RelayBuilder::<(), GroupsRelayProcessor>::new(relay_config)
            .cancellation_token(cancellation_token.clone())
            .connection_counter(connection_counter.clone())
            .metrics(SampledMetricsHandler::new(10))
            .subscription_metrics(PrometheusSubscriptionMetricsHandler)
            .event_processor(groups_processor)
            .relay_info(_relay_info.clone())
            .build_with(|chain| {
                chain
                    .with(Nip40ExpirationMiddleware::new())
                    .with(Nip70Middleware)
            })
            .await?,
    );

    let app_state = Arc::new(ServerState {
        http_state: http_state.clone(),
        cancellation_token: cancellation_token.clone(),
        metrics_handle: metrics_handle.clone(),
        connection_counter: connection_counter.clone(),
        relay_url: settings.relay_url.clone(),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Metrics handler without state
    let metrics_handler = move || async move { metrics_handle.render() };

    // Create a unified handler that supports both WebSocket and HTTP on the same route
    let root_handler = {
        let handler_factory = handler_factory.clone();
        let relay_info = _relay_info.clone();
        move |ws: Option<WebSocketUpgrade>,
              axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<SocketAddr>,
              headers: axum::http::HeaderMap| {
            let handler_factory = handler_factory.clone();
            let relay_info = relay_info.clone();

            async move {
                match ws {
                    Some(ws) => {
                        // Handle WebSocket upgrade
                        let handler = handler_factory.create();
                        handle_upgrade(ws, addr, handler).await
                    }
                    None => {
                        // Check for NIP-11 JSON request
                        if let Some(accept) = headers.get(axum::http::header::ACCEPT) {
                            if let Ok(value) = accept.to_str() {
                                if value == "application/nostr+json" {
                                    return axum::Json(&relay_info).into_response();
                                }
                            }
                        }

                        // Serve frontend
                        handler::serve_frontend().await.into_response()
                    }
                }
            }
        }
    };

    // Create API routes with state
    let api_routes = Router::new()
        .route("/api/subdomains", get(handler::handle_subdomains))
        .route("/api/config", get(handler::handle_config))
        .with_state(app_state);

    // Build router
    let router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(|| async { "OK" }))
        .route("/metrics", get(metrics_handler))
        .merge(api_routes)
        .nest_service("/assets", ServeDir::new("frontend/dist/assets"))
        .fallback_service(ServeDir::new("frontend/dist"))
        .layer(cors);

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
