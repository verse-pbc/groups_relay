pub mod app_state;
pub mod config;
pub mod create_client;
pub mod error;
pub mod group;
pub mod groups;
pub mod groups_event_processor;
pub mod handler;
pub mod metrics;
pub mod metrics_handler;
#[cfg(test)]
pub mod relay_middleware_integration_tests;
#[cfg(test)]
pub mod relay_middleware_tests;
pub mod sampled_metrics_handler;
pub mod server;
pub mod utils;
pub mod validation_middleware;

#[cfg(test)]
pub mod test_utils;

// Re-export commonly used items
pub use app_state::HttpServerState;
pub use groups::{Group, Groups, KIND_GROUP_USER_JOIN_REQUEST_9021};
pub use nostr_relay_builder::Error;
pub use nostr_relay_builder::RelayDatabase;
pub use nostr_relay_builder::StoreCommand;
pub use server::ServerState;
