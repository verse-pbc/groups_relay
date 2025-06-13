//! Handler functions for integrating with web frameworks
//!
//! This module provides pre-built handlers that can be used with various web frameworks.
//! Currently supports Axum, with other frameworks planned.

use axum::{
    extract::{ws::WebSocketUpgrade, ConnectInfo},
    http::HeaderMap,
    response::{IntoResponse, Json},
};
use serde::Serialize;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

/// Helper struct for automatic connection counting
struct ConnectionCounter {
    counter: Option<Arc<AtomicUsize>>,
}

impl ConnectionCounter {
    fn new(counter: Option<Arc<AtomicUsize>>) -> Self {
        if let Some(ref c) = counter {
            let prev = c.fetch_add(1, Ordering::Relaxed);
            debug!("New connection. Total active connections: {}", prev + 1);
        }
        Self { counter }
    }
}

impl Drop for ConnectionCounter {
    fn drop(&mut self) {
        if let Some(ref c) = self.counter {
            let prev = c.fetch_sub(1, Ordering::Relaxed);
            debug!("Connection closed. Total active connections: {}", prev - 1);
        }
    }
}

/// Extract the real client IP from headers or socket address
fn get_real_ip(headers: &HeaderMap, socket_addr: SocketAddr) -> String {
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

/// Handler functions for a Nostr relay
pub struct RelayHandlers<T = ()>
where
    T: Clone + Send + Sync + 'static,
{
    /// The WebSocket handler for Nostr protocol
    pub ws_handler: Arc<crate::RelayWebSocketHandler<T>>,
    /// NIP-11 relay information
    pub relay_info: RelayInfo,
    /// Cancellation token for graceful shutdown
    cancellation_token: CancellationToken,
    /// Optional connection counter for metrics
    connection_counter: Option<Arc<AtomicUsize>>,
    /// Subdomain configuration
    pub(crate) scope_config: crate::config::ScopeConfig,
}

/// NIP-11 Relay Information Document
#[derive(Debug, Clone, Serialize)]
pub struct RelayInfo {
    pub name: String,
    pub description: String,
    pub pubkey: String,
    pub contact: String,
    pub supported_nips: Vec<u16>,
    pub software: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Generate default HTML page for relay info
pub fn default_relay_html(relay_info: &RelayInfo) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{name} - Nostr Relay</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            max-width: 800px;
            margin: 0 auto;
            padding: 20px;
            background-color: #f5f5f5;
            color: #333;
        }}
        .container {{
            background-color: white;
            padding: 30px;
            border-radius: 10px;
            box-shadow: 0 2px 4px rgba(0,0,0,0.1);
        }}
        h1 {{
            color: #6200ea;
            margin-bottom: 10px;
        }}
        .description {{
            font-size: 1.1em;
            color: #666;
            margin-bottom: 30px;
        }}
        .info-section {{
            margin: 20px 0;
        }}
        .info-section h2 {{
            color: #444;
            font-size: 1.2em;
            margin-bottom: 10px;
        }}
        .info-item {{
            display: flex;
            padding: 8px 0;
            border-bottom: 1px solid #eee;
        }}
        .info-label {{
            font-weight: 600;
            width: 150px;
            color: #555;
        }}
        .info-value {{
            flex: 1;
            color: #666;
            word-break: break-all;
        }}
        .nip-list {{
            display: flex;
            flex-wrap: wrap;
            gap: 8px;
            margin-top: 5px;
        }}
        .nip-badge {{
            background-color: #6200ea;
            color: white;
            padding: 4px 8px;
            border-radius: 4px;
            font-size: 0.85em;
        }}
        .endpoint {{
            background-color: #f5f5f5;
            padding: 10px;
            border-radius: 4px;
            font-family: monospace;
            margin: 10px 0;
        }}
        .icon {{
            width: 64px;
            height: 64px;
            border-radius: 8px;
            margin-bottom: 20px;
        }}
        a {{
            color: #6200ea;
            text-decoration: none;
        }}
        a:hover {{
            text-decoration: underline;
        }}
    </style>
</head>
<body>
    <div class="container">
        {icon}
        <h1>{name}</h1>
        <p class="description">{description}</p>

        <div class="info-section">
            <h2>Connection Info</h2>
            <div class="endpoint">
                WebSocket Endpoint: <strong id="websocket-url"></strong>
            </div>
        </div>

        <div class="info-section">
            <h2>Relay Details</h2>
            <div class="info-item">
                <span class="info-label">Public Key:</span>
                <span class="info-value">{pubkey}</span>
            </div>
            <div class="info-item">
                <span class="info-label">Contact:</span>
                <span class="info-value">{contact}</span>
            </div>
            <div class="info-item">
                <span class="info-label">Software:</span>
                <span class="info-value">{software} v{version}</span>
            </div>
        </div>

        <div class="info-section">
            <h2>Supported NIPs</h2>
            <div class="nip-list">
                {nips}
            </div>
        </div>

        <div class="info-section">
            <h2>Usage</h2>
            <p>Connect to this relay using any Nostr client that supports the WebSocket protocol.</p>
            <p>For relay information in JSON format, send a request with <code>Accept: application/nostr+json</code></p>
        </div>
    </div>

    <script>
        // Dynamically set the WebSocket URL based on current protocol and hostname
        document.addEventListener('DOMContentLoaded', function() {{
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const host = window.location.host; // includes hostname and port if present
            const websocketUrl = protocol + '//' + host;
            document.getElementById('websocket-url').textContent = websocketUrl;
        }});
    </script>
</body>
</html>"#,
        name = relay_info.name,
        description = relay_info.description,
        icon = relay_info
            .icon
            .as_ref()
            .map(|url| format!(
                r#"<img src="{}" alt="{} icon" class="icon">"#,
                url, relay_info.name
            ))
            .unwrap_or_default(),
        pubkey = relay_info.pubkey,
        contact = relay_info.contact,
        software = relay_info.software,
        version = relay_info.version,
        nips = relay_info
            .supported_nips
            .iter()
            .map(|nip| format!(r#"<span class="nip-badge">NIP-{}</span>"#, nip))
            .collect::<Vec<_>>()
            .join("")
    )
}

impl<T> RelayHandlers<T>
where
    T: Clone + Send + Sync + 'static,
{
    /// Create new relay handlers
    pub fn new(
        ws_handler: crate::RelayWebSocketHandler<T>,
        relay_info: RelayInfo,
        cancellation_token: Option<CancellationToken>,
        connection_counter: Option<Arc<AtomicUsize>>,
        scope_config: crate::config::ScopeConfig,
    ) -> Self {
        Self {
            ws_handler: Arc::new(ws_handler),
            relay_info,
            cancellation_token: cancellation_token.unwrap_or_default(),
            connection_counter,
            scope_config,
        }
    }

    /// Check if request wants NIP-11 JSON based on Accept header
    pub fn wants_nostr_json(accept_header: &str) -> bool {
        accept_header == "application/nostr+json"
    }

    /// Get the relay information
    pub fn relay_info(&self) -> &RelayInfo {
        &self.relay_info
    }

    /// Get the cancellation token
    pub fn cancellation_token(&self) -> &CancellationToken {
        &self.cancellation_token
    }

    /// Get the current connection count if tracking is enabled
    pub fn connection_count(&self) -> Option<usize> {
        self.connection_counter
            .as_ref()
            .map(|counter| counter.load(Ordering::Relaxed))
    }

    /// Creates an Axum-compatible WebSocket-only handler function
    pub fn axum_ws_handler(
        self: Arc<Self>,
    ) -> impl Fn(
        WebSocketUpgrade,
        ConnectInfo<SocketAddr>,
        HeaderMap,
    ) -> Pin<Box<dyn Future<Output = axum::response::Response> + Send>>
           + Clone
           + Send
           + 'static {
        move |ws: WebSocketUpgrade,
              ConnectInfo(addr): ConnectInfo<SocketAddr>,
              headers: HeaderMap| {
            let handlers = self.clone();

            Box::pin(async move {
                let real_ip = get_real_ip(&headers, addr);
                let host = headers
                    .get("host")
                    .and_then(|h| h.to_str().ok())
                    .map(String::from);

                // Extract subdomain for logging
                let subdomain = host.as_ref().and_then(|h| match &handlers.scope_config {
                    crate::config::ScopeConfig::Subdomain { base_domain_parts } => {
                        crate::subdomain::extract_subdomain(h, *base_domain_parts)
                    }
                    _ => None,
                });

                let ws_handler = handlers.ws_handler.clone();
                let cancellation_token = handlers.cancellation_token.clone();
                let connection_counter = handlers.connection_counter.clone();

                ws.on_upgrade(move |socket| async move {
                    // Create isolated span for this connection
                    let span = tracing::info_span!(
                        parent: None,
                        "websocket_connection",
                        ip = %real_ip,
                        subdomain = ?subdomain
                    );
                    let _guard = span.enter();

                    let _counter = ConnectionCounter::new(connection_counter);

                    info!("New WebSocket connection from {}", real_ip);

                    // Set the host in task-local storage for subdomain extraction
                    crate::state::CURRENT_REQUEST_HOST
                        .scope(host, async move {
                            if let Err(e) = ws_handler
                                .start(socket, real_ip.clone(), cancellation_token)
                                .await
                            {
                                error!("WebSocket error for connection {}: {}", real_ip, e);
                            }
                        })
                        .await;
                })
                .into_response()
            })
        }
    }

    /// Creates an Axum-compatible root handler function
    pub fn axum_root_handler(
        self: Arc<Self>,
    ) -> impl Fn(
        Option<WebSocketUpgrade>,
        ConnectInfo<SocketAddr>,
        HeaderMap,
    ) -> Pin<Box<dyn Future<Output = axum::response::Response> + Send>>
           + Clone
           + Send
           + 'static {
        move |ws: Option<WebSocketUpgrade>,
              ConnectInfo(addr): ConnectInfo<SocketAddr>,
              headers: HeaderMap| {
            let handlers = self.clone();

            Box::pin(async move {
                // 1. WebSocket upgrade
                if let Some(ws) = ws {
                    let real_ip = get_real_ip(&headers, addr);
                    let host = headers
                        .get("host")
                        .and_then(|h| h.to_str().ok())
                        .map(String::from);

                    // Extract subdomain for logging
                    let subdomain = host.as_ref().and_then(|h| match &handlers.scope_config {
                        crate::config::ScopeConfig::Subdomain { base_domain_parts } => {
                            crate::subdomain::extract_subdomain(h, *base_domain_parts)
                        }
                        _ => None,
                    });

                    let display_info = if let Some(sub) = &subdomain {
                        if !sub.is_empty() {
                            format!(" @{}", sub)
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };

                    // Log the upgrade request with real IP and subdomain
                    debug!(
                        "WebSocket upgrade requested from {}{} at root path",
                        real_ip, display_info
                    );

                    let ws_handler = handlers.ws_handler.clone();
                    let cancellation_token = handlers.cancellation_token.clone();
                    let connection_counter = handlers.connection_counter.clone();

                    return ws
                        .on_upgrade(move |socket| async move {
                            // Create isolated span for this connection
                            let span = tracing::info_span!(
                                parent: None,
                                "websocket_connection",
                                ip = %real_ip,
                                subdomain = ?subdomain
                            );
                            let _guard = span.enter();

                            let _counter = ConnectionCounter::new(connection_counter);

                            info!("New WebSocket connection from {}", real_ip);

                            // Set the host in task-local storage for subdomain extraction
                            crate::state::CURRENT_REQUEST_HOST
                                .scope(host, async move {
                                    if let Err(e) = ws_handler
                                        .start(socket, real_ip.clone(), cancellation_token)
                                        .await
                                    {
                                        error!("WebSocket error for connection {}: {}", real_ip, e);
                                    }
                                })
                                .await;
                        })
                        .into_response();
                }

                // 2. NIP-11 JSON
                if let Some(accept) = headers.get(axum::http::header::ACCEPT) {
                    if let Ok(value) = accept.to_str() {
                        if Self::wants_nostr_json(value) {
                            return Json(&handlers.relay_info).into_response();
                        }
                    }
                }

                // 3. Return 404 for other requests
                axum::http::StatusCode::NOT_FOUND.into_response()
            })
        }
    }
}
