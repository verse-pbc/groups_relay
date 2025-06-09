//! A framework for building custom Nostr relays with middleware support
//!
//! This crate provides the building blocks for creating Nostr relays with:
//! - Middleware-based message processing
//! - Pluggable business logic via the EventProcessor trait
//! - Built-in protocol support (NIPs 09, 40, 42, 70)
//! - WebSocket connection management
//! - Database abstraction

pub mod config;
pub mod crypto_worker;
pub mod database;
pub mod error;
pub mod relay_builder;
pub mod global_config;
pub mod global_metrics;
#[cfg(feature = "axum")]
pub mod handlers;
pub mod event_processor;
pub mod message_converter;
pub mod metrics;
pub mod middleware;
pub mod middlewares;
pub mod state;
pub mod subdomain;
pub mod subscription_service;
#[cfg(test)]
pub mod test_utils;
pub mod utils;

pub use config::{RelayConfig, ScopeConfig, WebSocketConfig};
pub use database::{NostrDatabase, RelayDatabase};
pub use error::{Error, Result};
pub use relay_builder::{DefaultRelayWebSocketHandler, RelayBuilder, RelayWebSocketHandler};
#[cfg(feature = "axum")]
pub use handlers::{RelayHandlers, RelayInfo};
pub use event_processor::{EventContext, EventProcessor, PublicRelayProcessor};
pub use message_converter::NostrMessageConverter;
pub use middleware::RelayMiddleware;
pub use state::{
    DefaultNostrConnectionState, GenericNostrConnectionFactory, NostrConnectionFactory,
    NostrConnectionState,
};
pub use subscription_service::{StoreCommand, SubscriptionService};
pub use crypto_worker::{CryptoWorker, CryptoWorkerMetricsSnapshot};

// Re-export commonly used middlewares
pub use middlewares::{
    AuthConfig, ClientMessageId, ErrorHandlingMiddleware, EventVerifierMiddleware,
    LoggerMiddleware, Nip09Middleware, Nip40ExpirationMiddleware,
    Nip42Middleware, Nip70Middleware,
};
