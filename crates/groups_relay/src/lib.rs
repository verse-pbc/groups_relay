pub mod app_state;
pub mod config;
pub mod create_client;
pub mod groups;
pub mod handler;
pub mod metrics;
pub mod middlewares;
// pub mod nostr_database; // Now using RelayDatabase from nostr_relay_builder
// pub mod nostr_session_state; // Now using NostrConnectionState from nostr_relay_builder
// pub mod relay_builder; // Moved to nostr_relay_builder crate
pub mod relay_logic;
// pub mod relay_middleware; // Now using generic RelayMiddleware from nostr_relay_builder
#[cfg(test)]
pub mod relay_middleware_integration_tests;
#[cfg(test)]
pub mod relay_middleware_tests;
pub mod server;
// pub mod subdomain; // Moved to nostr_relay_builder
// pub mod subscription_manager; // Moved to nostr_relay_builder
pub mod utils;
// pub mod websocket_server; // No longer needed - using RelayBuilder directly

#[cfg(test)]
pub mod test_utils;

// Re-export commonly used items
pub use app_state::HttpServerState;
pub use groups::{Group, Groups, KIND_GROUP_USER_JOIN_REQUEST_9021};
pub use nostr_relay_builder::Error;
pub use nostr_relay_builder::RelayDatabase;
pub use nostr_relay_builder::StoreCommand;
pub use nostr_relay_builder::SubscriptionService;
pub use server::ServerState;
