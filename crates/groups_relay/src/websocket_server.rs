use crate::{
    config, groups::Groups, middlewares::ValidationMiddleware,
    relay_logic::groups_logic::GroupsRelayProcessor,
};
use anyhow::Result;
use nostr_relay_builder::{
    AuthConfig, Nip09Middleware, Nip40ExpirationMiddleware, Nip70Middleware, RelayBuilder,
    RelayConfig, RelayWebSocketHandler, WebSocketConfig,
};
use nostr_sdk::prelude::*;
use std::sync::Arc;

#[allow(clippy::type_complexity)]
pub async fn build_websocket_handler(
    default_relay_url: RelayUrl,
    auth_url: String,
    groups: Arc<Groups>,
    relay_keys: &config::Keys,
    database: Arc<nostr_relay_builder::RelayDatabase>,
    settings: &config::Settings,
) -> Result<RelayWebSocketHandler> {
    let websocket_config = WebSocketConfig {
        channel_size: settings.websocket.channel_size,
        max_connections: settings.websocket.max_connections,
        max_connection_time: settings.websocket.max_connection_time.map(|d| d.as_secs()),
    };

    let relay_config = RelayConfig::new(
        default_relay_url.to_string(),
        database.clone(),
        relay_keys.clone(),
    )
    .with_subdomains(settings.base_domain_parts)
    .with_auth(AuthConfig {
        auth_url: auth_url.clone(),
        base_domain_parts: settings.base_domain_parts,
        validate_subdomains: true,
    })
    .with_websocket_config(websocket_config)
    .with_query_limit(settings.query_limit);

    let groups_processor = GroupsRelayProcessor::new(groups.clone(), relay_keys.public_key);

    // NIP-42 auth middleware is automatically added when with_auth() is used
    let handler = RelayBuilder::new(relay_config)
        .with_middleware(ValidationMiddleware::new(relay_keys.public_key))
        .with_middleware(Nip09Middleware::new(database.clone()))
        .with_middleware(Nip40ExpirationMiddleware::new())
        .with_middleware(Nip70Middleware)
        .build_server(groups_processor)
        .await?;

    Ok(handler)
}
