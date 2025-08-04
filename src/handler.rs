use crate::groups::Invite;
use crate::server::ServerState;
use axum::{
    body::Body,
    extract::State,
    http::{Method, Request, StatusCode},
    response::{IntoResponse, Json},
};
use nostr_lmdb::Scope;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;
use tower_http::services::ServeDir;
use tracing::debug;

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

pub async fn handle_root() -> impl IntoResponse {
    // Serve the frontend HTML
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
            eprintln!("Error serving frontend: {err:?}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}

pub async fn handle_health() -> impl IntoResponse {
    "OK"
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

    // Extract host from relay URL and count parts
    let base_domain_parts = nostr_sdk::Url::parse(&state.relay_url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .map(|host| {
            if host == "localhost" || host.parse::<std::net::IpAddr>().is_ok() {
                2 // Default for localhost/IP
            } else {
                host.split('.').count()
            }
        })
        .unwrap_or(2);

    Json(ConfigResponse { base_domain_parts })
}

/// Serve the frontend without needing state
pub async fn serve_frontend() -> impl IntoResponse {
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
            eprintln!("Error serving frontend: {err:?}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
        }
    }
}
