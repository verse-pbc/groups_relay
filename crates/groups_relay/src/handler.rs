use crate::groups::Invite;
use crate::server::ServerState;
use crate::subdomain::extract_subdomain;
use axum::{
    body::Body,
    extract::{ConnectInfo, State, WebSocketUpgrade},
    http::{Method, Request, StatusCode},
    response::{IntoResponse, Json},
};
use nostr_lmdb::Scope;
use nostr_sdk::nips::nip11::RelayInformationDocument;
use serde::Serialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeDir;
use tracing::{debug, error, info};

tokio::task_local! {
    pub static CURRENT_REQUEST_HOST: Option<String>;
}

#[derive(Serialize)]
pub struct GroupResponse {
    id: String,
    name: String,
    about: Option<String>,
    picture: Option<String>,
    private: bool,
    closed: bool,
    members: Vec<MemberResponse>,
    invites: HashMap<String, Invite>,
    join_requests: Vec<String>,
    created_at: u64,
    updated_at: u64,
}

#[derive(Serialize)]
struct MemberResponse {
    pubkey: String,
    roles: Vec<String>,
}

#[derive(Serialize)]
pub struct SubdomainResponse {
    subdomains: Vec<String>,
}

#[derive(Serialize)]
pub struct ConfigResponse {
    base_domain_parts: usize,
}

/// A RAII guard for tracking active WebSocket connections
#[derive(Debug)]
struct ConnectionCounter {
    counter: Arc<AtomicUsize>,
}

impl ConnectionCounter {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        let prev = counter.fetch_add(1, Ordering::SeqCst);
        info!("New connection. Total active connections: {}", prev + 1);
        Self { counter }
    }
}

impl Drop for ConnectionCounter {
    fn drop(&mut self) {
        let prev = self.counter.fetch_sub(1, Ordering::SeqCst);
        info!("Connection closed. Total active connections: {}", prev - 1);
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

async fn handle_websocket_connection(
    socket: axum::extract::ws::WebSocket,
    state: Arc<ServerState>,
    real_ip: String,
    host_string: Option<String>,
) {
    // Use a separate function for connection handling with isolated span context
    run_websocket_connection(socket, state, real_ip, host_string).await;
}

async fn run_websocket_connection(
    socket: axum::extract::ws::WebSocket,
    state: Arc<ServerState>,
    real_ip: String,
    host_string: Option<String>,
) {
    // Extract subdomain from host string to use in logs
    let subdomain = host_string
        .as_ref()
        .and_then(|host| extract_subdomain(host, state.base_domain_parts));

    // Create isolated span for this connection
    let span = tracing::info_span!(parent: None, "websocket_connection", ip = %real_ip, subdomain = ?subdomain);
    let _guard = span.enter();

    // Create the connection counter guard - it will be automatically dropped when the connection ends
    let _counter = ConnectionCounter::new(state.connection_counter.clone());

    // Process the connection within the span's lifetime
    CURRENT_REQUEST_HOST
        .scope(host_string, async {
            let result = state
                .ws_handler
                .start(socket, real_ip.clone(), state.cancellation_token.clone())
                .await;
            // Log connection status
            match result {
                Ok(_) => debug!("WebSocket connection closed"),
                Err(e) => error!("WebSocket error: {:?}", e),
            }
        })
        .await;
}

pub async fn handle_root(
    ws: Option<WebSocketUpgrade>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<ServerState>>,
    headers: axum::http::HeaderMap,
    _request: Request<Body>,
) -> impl IntoResponse {
    // 1. WebSocket upgrade: if the upgrade header is present, upgrade the connection.
    if let Some(ws) = ws {
        let real_ip = get_real_ip(&headers, addr);
        let host_string = headers
            .get(axum::http::header::HOST)
            .and_then(|hv| hv.to_str().ok().map(String::from));

        // Extract subdomain for logging
        let subdomain = host_string
            .as_ref()
            .and_then(|host| extract_subdomain(host, state.base_domain_parts));

        // Create a display string with subdomain information
        let display_info = if let Some(sub) = &subdomain {
            if !sub.is_empty() {
                format!(" @{}", sub)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Log upgrade request in isolated span
        let upgrade_span = tracing::info_span!(parent: None, "http_upgrade");
        let _guard = upgrade_span.enter();
        info!(
            "WebSocket upgrade requested from {}{} at root path",
            real_ip, display_info
        );
        drop(_guard);

        // Use detached task for connection handling to prevent span inheritance
        return ws.on_upgrade(move |socket| async move {
            tokio::task::spawn(async move {
                handle_websocket_connection(socket, state, real_ip, host_string).await;
            });
        });
    }

    // 2. Nostr JSON: if the Accept header is "application/nostr+json", serve Nostr JSON.
    if let Some(accept_header) = headers.get(axum::http::header::ACCEPT) {
        if let Ok(value) = accept_header.to_str() {
            if value == "application/nostr+json" {
                debug!("Handling Nostr JSON request");
                return handle_nostr_json(State(state.clone()))
                    .await
                    .into_response();
            }
        }
    }

    // 3. Fallback: serve the static HTML (Vite frontend).
    debug!("Serving frontend HTML for root path");
    let index_req = Request::builder()
        .method(Method::GET)
        .uri("/index.html")
        .body(Body::empty())
        .unwrap();

    match ServeDir::new("frontend/dist").oneshot(index_req).await {
        Ok(response) => {
            debug!("Frontend served successfully");
            response.into_response()
        }
        Err(err) => {
            eprintln!("Error serving frontend: {:?}", err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

pub async fn handle_health() -> impl IntoResponse {
    "OK"
}

pub async fn handle_nostr_json(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    let relay_info = RelayInformationDocument {
        name: Some("Nostr Groups Relay".to_string()),
        description: Some(
            "A specialized relay implementing NIP-29 for Nostr group management. This relay is under development and all data may be deleted in the future".to_string(),
        ),
        supported_nips: Some(vec![1, 9, 11, 29, 40, 42, 70]),
        software: Some("groups_relay".to_string()),
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
        pubkey: Some(state.http_state.groups.relay_pubkey.to_string()),
        contact: Some("https://daniel.nos.social".to_string()),
        limitation: None,
        payments_url: None,
        fees: None,
        icon: Some("https://pfp.nostr.build/c60f4853a6d4ae046bdbbd935f0ccd7354c9c411c324b411666d325562a5a906.png".to_string()),
        relay_countries: vec![],
        language_tags: vec![],
        tags: vec![],
        posting_policy: None,
        retention: vec![],
    };

    Json(relay_info)
}

pub async fn handle_metrics(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    state.metrics_handle.render()
}

pub async fn handle_subdomains(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    debug!("Handling subdomains request");

    // Get all scopes from the groups
    let scopes = state.http_state.groups.get_all_scopes();

    // Convert scopes to subdomain strings
    let mut subdomains = Vec::new();
    for scope in scopes {
        match scope {
            Scope::Default => {
                // The default scope represents the main domain (no subdomain)
                // We could add an empty string or skip it
            }
            Scope::Named { name, .. } => {
                subdomains.push(name);
            }
        }
    }

    // Sort subdomains alphabetically
    subdomains.sort();

    debug!("Found {} subdomains: {:?}", subdomains.len(), subdomains);

    Json(SubdomainResponse { subdomains })
}

pub async fn handle_config(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    debug!("Handling config request");

    Json(ConfigResponse {
        base_domain_parts: state.base_domain_parts,
    })
}
