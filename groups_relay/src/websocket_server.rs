use crate::{
    config,
    groups::Groups,
    middlewares::{
        EventStoreMiddleware, EventVerifierMiddleware, LoggerMiddleware, Nip29Middleware,
        Nip42Middleware, Nip70Middleware, NostrMessageConverter, ValidationMiddleware,
    },
    nostr_database::NostrDatabase,
    nostr_session_state::{NostrConnectionFactory, NostrConnectionState},
};
use anyhow::Result;
use nostr_sdk::{ClientMessage, RelayMessage};
use std::sync::Arc;
use websocket_builder::WebSocketBuilder;
pub use websocket_builder::WebSocketHandler;

pub fn build_websocket_handler(
    relay_url: String,
    auth_url: String,
    groups: Arc<Groups>,
    relay_keys: &config::Keys,
    database: Arc<NostrDatabase>,
    websocket_settings: &config::WebSocketSettings,
) -> Result<
    WebSocketHandler<
        NostrConnectionState,
        ClientMessage,
        RelayMessage,
        NostrMessageConverter,
        NostrConnectionFactory,
    >,
> {
    let logger = LoggerMiddleware;
    let event_verifier = EventVerifierMiddleware;
    let nip_42 = Nip42Middleware::new(auth_url);
    let nip_70 = Nip70Middleware;
    let nip_29 = Nip29Middleware::new(groups, relay_keys.public_key());
    let event_store = EventStoreMiddleware::new(database);
    let validation_middleware = ValidationMiddleware::new(relay_keys.public_key());
    let connection_state_factory = NostrConnectionFactory::new(relay_url)?;

    let mut builder = WebSocketBuilder::new(connection_state_factory, NostrMessageConverter);

    // Apply WebSocket settings from configuration
    builder = builder.with_channel_size(websocket_settings.channel_size());
    if let Some(max_time) = websocket_settings.max_connection_time() {
        builder = builder.with_max_connection_time(max_time);
    }

    if let Some(max_conns) = websocket_settings.max_connections() {
        builder = builder.with_max_connections(max_conns);
    }

    Ok(builder
        .with_middleware(logger)
        .with_middleware(nip_42)
        .with_middleware(validation_middleware)
        .with_middleware(event_verifier)
        .with_middleware(nip_70)
        .with_middleware(nip_29)
        .with_middleware(event_store)
        .build())
}
