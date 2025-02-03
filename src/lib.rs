pub mod app_state;
pub mod config;
pub mod create_client;
pub mod error;
pub mod event_store_connection;
pub mod groups;
pub mod handler;
pub mod metrics;
pub mod middlewares;
pub mod nostr_database;
pub mod nostr_session_state;

#[cfg(test)]
pub mod test_utils;

// Re-export commonly used items
pub use app_state::HttpServerState;
pub use error::Error;
pub use event_store_connection::{EventStoreConnection, StoreCommand};
pub use groups::{Group, Groups, KIND_GROUP_USER_JOIN_REQUEST_9021};
pub use nostr_database::NostrDatabase;
