use crate::{
    app_state::HttpServerState, config, groups::Groups,
    groups_event_processor::GroupsRelayProcessor, handler, metrics,
    metrics_handler::PrometheusSubscriptionMetricsHandler,
    sampled_metrics_handler::SampledMetricsHandler, validation_middleware::ValidationMiddleware,
    RelayDatabase,
};
use anyhow::Result;
use axum::{routing::get, Router};
use nostr_relay_builder::{
    crypto_worker::CryptoSender, AuthConfig, Nip09Middleware, Nip40ExpirationMiddleware,
    Nip70Middleware, RelayBuilder, RelayConfig, RelayInfo, WebSocketConfig,
};
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
    pub handlers: Arc<nostr_relay_builder::RelayService<()>>,
    pub cancellation_token: CancellationToken,
    pub metrics_handle: metrics::PrometheusHandle,
    pub connection_counter: Arc<AtomicUsize>,
    pub relay_url: String,
}

pub async fn run_server(
    settings: config::Settings,
    relay_keys: config::Keys,
    database: Arc<RelayDatabase>,
    db_sender: nostr_relay_builder::DatabaseSender,
    crypto_sender: CryptoSender,
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

    let relay_config = RelayConfig::new(
        settings.relay_url.clone(),
        (database.clone(), db_sender, crypto_sender),
        relay_keys.clone(),
    )
    .with_subdomains_from_url(&settings.relay_url)
    .with_auth(AuthConfig {
        relay_url: settings.relay_url.clone(),
        validate_subdomains: true,
    })
    .with_websocket_config(websocket_config)
    .with_subscription_limits(settings.max_subscriptions, settings.max_limit);

    let groups_processor = GroupsRelayProcessor::new(groups.clone(), relay_keys.public_key);

    // Create cancellation token and connection counter
    let cancellation_token = CancellationToken::new();
    let connection_counter = Arc::new(AtomicUsize::new(0));

    // Define relay information
    let relay_info = RelayInfo {
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
    let handlers = RelayBuilder::new(relay_config)
        .with_cancellation_token(cancellation_token.clone())
        .with_connection_counter(connection_counter.clone())
        .with_metrics(SampledMetricsHandler::new(10))
        .with_subscription_metrics(PrometheusSubscriptionMetricsHandler)
        .with_middleware(Nip09Middleware::new(database.clone()))
        .with_middleware(Nip40ExpirationMiddleware::new())
        .with_middleware(Nip70Middleware)
        .with_event_processor(groups_processor)
        .build_relay_service(relay_info)
        .await?;

    let app_state = Arc::new(ServerState {
        http_state: http_state.clone(),
        handlers: handlers.clone(),
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

    let router = Router::new()
        .route("/", get(handler::handle_root))
        .route("/health", get(|| async { "OK" }))
        .route("/metrics", get(metrics_handler))
        .route("/api/subdomains", get(handler::handle_subdomains))
        .route("/api/config", get(handler::handle_config))
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
