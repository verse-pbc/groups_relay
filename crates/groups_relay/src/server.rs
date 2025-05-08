use crate::{
    app_state::HttpServerState,
    config,
    groups::Groups,
    handler, metrics,
    nostr_database::RelayDatabase,
    nostr_session_state::{NostrConnectionFactory, NostrConnectionState},
    websocket_server::{self, NostrMessageConverter},
};
use anyhow::Result;
use axum::{routing::get, Router};
use nostr_sdk::prelude::*;
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
    pub ws_handler: Arc<
        websocket_server::WebSocketHandler<
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
            NostrMessageConverter,
            NostrConnectionFactory,
        >,
    >,
    pub cancellation_token: CancellationToken,
    pub metrics_handle: metrics::PrometheusHandle,
    pub connection_counter: Arc<AtomicUsize>,
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
    info!("Auth requests must match this URL: {}", settings.auth_url);

    let relay_url_parsed = RelayUrl::parse(&settings.relay_url)?;

    let ws_handler = Arc::new(websocket_server::build_websocket_handler(
        relay_url_parsed,
        settings.auth_url.clone(),
        groups.clone(),
        &relay_keys,
        database,
        &settings.websocket,
    )?);

    let cancellation_token = CancellationToken::new();
    let app_state = Arc::new(ServerState {
        http_state: http_state.clone(),
        ws_handler: ws_handler.clone(),
        cancellation_token: cancellation_token.clone(),
        metrics_handle: metrics_handle.clone(),
        connection_counter: Arc::new(AtomicUsize::new(0)),
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = Router::new()
        .route("/", get(handler::handle_root))
        .route("/health", get(handler::handle_health))
        .route("/metrics", get(handler::handle_metrics))
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
