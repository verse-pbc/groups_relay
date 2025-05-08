use crate::{
    config::{self, WebSocketSettings},
    groups::Groups,
    middlewares::{
        ErrorHandlingMiddleware, EventVerifierMiddleware, LoggerMiddleware, Nip09Middleware,
        Nip29Middleware, Nip40Middleware, Nip42Middleware, Nip70Middleware, ValidationMiddleware,
    },
    nostr_database::RelayDatabase,
    nostr_session_state::{NostrConnectionFactory, NostrConnectionState},
};
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use websocket_builder::WebSocketBuilder;
pub use websocket_builder::WebSocketHandler;

#[derive(Clone, Debug)]
pub struct NostrMessageConverter;

impl<'a> websocket_builder::MessageConverter<ClientMessage<'a>, RelayMessage<'a>>
    for NostrMessageConverter
{
    fn outbound_to_string(&self, message: RelayMessage<'a>) -> Result<String, anyhow::Error> {
        Ok(message.as_json())
    }

    fn inbound_from_string(
        &self,
        message: String,
    ) -> Result<Option<ClientMessage<'a>>, anyhow::Error> {
        match ClientMessage::from_json(&message) {
            Ok(sdk_msg) => Ok(Some(sdk_msg)),
            Err(e) => {
                if message.trim().is_empty() {
                    Ok(None)
                } else {
                    tracing::warn!("Failed to parse client message: {}, error: {}", message, e);
                    Err(anyhow::anyhow!("Failed to parse client message: {}", e))
                }
            }
        }
    }
}

#[allow(clippy::type_complexity)]
pub fn build_websocket_handler(
    default_relay_url: RelayUrl,
    auth_url: String,
    groups: Arc<Groups>,
    relay_keys: &config::Keys,
    database: Arc<RelayDatabase>,
    ws_settings: &WebSocketSettings,
) -> Result<
    WebSocketHandler<
        NostrConnectionState,
        ClientMessage<'static>,
        RelayMessage<'static>,
        NostrMessageConverter,
        NostrConnectionFactory,
    >,
> {
    let factory = NostrConnectionFactory::new(
        default_relay_url.to_string(),
        database.clone(),
        groups.clone(),
    )?;
    let converter = NostrMessageConverter;

    let mut builder = WebSocketBuilder::new(factory, converter)
        .with_middleware(LoggerMiddleware::new())
        .with_middleware(ErrorHandlingMiddleware {})
        .with_middleware(Nip42Middleware::new(auth_url))
        .with_middleware(EventVerifierMiddleware::new())
        .with_middleware(ValidationMiddleware::new(relay_keys.public_key))
        .with_middleware(Nip09Middleware::new(database.clone()))
        .with_middleware(Nip40Middleware::new(database.clone()))
        .with_middleware(Nip70Middleware {})
        .with_middleware(Nip29Middleware::new(
            groups,
            relay_keys.public_key,
            database,
        ));

    if let Some(max_conn_time) = ws_settings.max_connection_time {
        builder = builder.with_max_connection_time(max_conn_time);
    }
    if let Some(max_conns) = ws_settings.max_connections {
        builder = builder.with_max_connections(max_conns);
    }
    builder = builder.with_channel_size(ws_settings.channel_size);

    Ok(builder.build())
}
