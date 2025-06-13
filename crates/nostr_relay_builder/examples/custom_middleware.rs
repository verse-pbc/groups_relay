//! Example showing how to create and use custom middleware

use anyhow::Result;
use async_trait::async_trait;
use nostr_relay_builder::{
    CryptoWorker, EventContext, EventProcessor, Nip09Middleware, Nip40ExpirationMiddleware,
    Nip70Middleware, NostrConnectionState, RelayBuilder, RelayConfig, RelayDatabase,
    Result as RelayResult, StoreCommand, WebSocketConfig,
};
use nostr_sdk::prelude::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_subscriber::{fmt, EnvFilter};
use websocket_builder::{InboundContext, Middleware, SendMessage};

/// Custom middleware that counts messages and implements rate limiting
#[derive(Debug, Clone)]
struct RateLimitMiddleware {
    message_counts: Arc<Mutex<HashMap<String, (u64, Instant)>>>,
    max_messages_per_minute: u64,
}

impl RateLimitMiddleware {
    fn new(max_messages_per_minute: u64) -> Self {
        Self {
            message_counts: Arc::new(Mutex::new(HashMap::new())),
            max_messages_per_minute,
        }
    }
}

#[async_trait]
impl Middleware for RateLimitMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> anyhow::Result<()> {
        let conn_id = ctx.connection_id.clone();
        let now = Instant::now();

        // Check if rate limit exceeded
        let exceeded = {
            let mut counts = self.message_counts.lock();
            let (count, last_reset) = counts.entry(conn_id).or_insert((0, now));

            // Reset counter if a minute has passed
            if now.duration_since(*last_reset).as_secs() >= 60 {
                *count = 0;
                *last_reset = now;
            }

            *count += 1;

            let current_count = *count;
            let exceeded = current_count > self.max_messages_per_minute;

            if !exceeded {
                info!(
                    "Message {} from connection {} this minute",
                    current_count, ctx.connection_id
                );
            }

            exceeded
        }; // Drop the lock here

        if exceeded {
            warn!("Connection {} exceeded rate limit", ctx.connection_id);

            // Send a notice and drop the message
            ctx.send_message(RelayMessage::notice("rate limit exceeded"))?;

            // Drop the message by not calling next
            return Ok(());
        }

        // Process the message normally
        ctx.next().await
    }
}

/// Custom middleware that logs all events with specific kinds
#[derive(Debug, Clone)]
struct EventLoggerMiddleware {
    logged_kinds: Vec<Kind>,
}

impl EventLoggerMiddleware {
    fn new(kinds: Vec<Kind>) -> Self {
        Self {
            logged_kinds: kinds,
        }
    }
}

#[async_trait]
impl Middleware for EventLoggerMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> anyhow::Result<()> {
        // Check if this is an event message
        if let Some(ClientMessage::Event(event)) = &ctx.message {
            if self.logged_kinds.contains(&event.kind) {
                info!(
                    "Event logged - ID: {}, Kind: {:?}, Content: {}",
                    event.id, event.kind, event.content
                );
            }
        }

        // Always continue processing
        ctx.next().await
    }
}

/// A simple event processor that accepts all events
#[derive(Debug, Clone)]
struct SimpleEventProcessor;

#[async_trait]
impl EventProcessor for SimpleEventProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup logging
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,nostr_relay_builder=debug"));
    fmt().with_env_filter(env_filter).with_target(true).init();

    // Generate relay keys
    let keys = Keys::generate();
    println!("Relay public key: {}", keys.public_key());

    // Configure the relay with custom WebSocket settings
    let websocket_config = WebSocketConfig {
        channel_size: 500,               // Smaller channel for this example
        max_connections: Some(10),       // Limit to 10 concurrent connections
        max_connection_time: Some(3600), // 1 hour max connection time
    };

    // Create the crypto worker and database
    let cancellation_token = CancellationToken::new();
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(keys.clone()),
        cancellation_token,
    ));
    let database = Arc::new(RelayDatabase::new("./data/custom_relay.db", crypto_worker)?);

    let config = RelayConfig::new("wss://localhost:8080", database.clone(), keys)
        .with_websocket_config(websocket_config);

    // Create event processor
    let processor = SimpleEventProcessor;

    // Build the WebSocket server with custom middlewares
    let _handler = RelayBuilder::new(config)
        // Add our custom rate limit middleware first
        .with_middleware(RateLimitMiddleware::new(100))
        // Add event logger for text notes and metadata
        .with_middleware(EventLoggerMiddleware::new(vec![
            Kind::TextNote,
            Kind::Metadata,
        ]))
        // Add standard protocol middlewares (NIP-09, NIP-40, NIP-70)
        .with_middleware(Nip09Middleware::new(database.clone()))
        .with_middleware(Nip40ExpirationMiddleware::new())
        .with_middleware(Nip70Middleware)
        .build_server(processor)
        .await?;

    println!("\nCustom relay running with:");
    println!("- Rate limiting: max 100 messages per minute per connection");
    println!("- Event logging for text notes and metadata");
    println!("- NIP-09 deletion support");
    println!("- NIP-40 expiration support");
    println!("- Max 10 concurrent connections");
    println!("- 1 hour max connection time");

    Ok(())
}
