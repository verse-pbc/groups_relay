pub mod app_state;
pub mod config;
pub mod create_client;
pub mod error;
pub mod groups;
pub mod handler;
pub mod metrics;
pub mod middlewares;
pub mod nostr_database;
pub mod nostr_session_state;
pub mod server;
pub mod subscription_manager;
pub mod utils;
pub mod websocket_server;

#[cfg(test)]
pub mod test_utils;

// Re-export commonly used items
pub use app_state::HttpServerState;
pub use error::Error;
pub use groups::{Group, Groups, KIND_GROUP_USER_JOIN_REQUEST_9021};
pub use nostr_database::RelayDatabase;
pub use server::ServerState;
pub use subscription_manager::{StoreCommand, SubscriptionManager};
