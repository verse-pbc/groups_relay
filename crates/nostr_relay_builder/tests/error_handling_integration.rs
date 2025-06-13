//! Integration tests for error handling middleware

use nostr_relay_builder::{ErrorHandlingMiddleware, RelayBuilder, RelayConfig};
use nostr_sdk::prelude::*;

#[tokio::test]
async fn test_error_handling_middleware_in_relay() {
    // This test verifies that ErrorHandlingMiddleware is properly integrated
    // Create a simple relay with error handling middleware
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir
        .path()
        .join("test_db")
        .to_string_lossy()
        .to_string();
    let keys = Keys::generate();
    let config = RelayConfig::new("ws://test", db_path, keys);

    let _relay_builder =
        RelayBuilder::<()>::new(config).with_middleware(ErrorHandlingMiddleware::new());

    // The actual middleware execution is tested in the error_handling.rs unit tests
    // and through the websocket_builder's integration test pattern
}

#[test]
fn test_error_handling_middleware_creation() {
    // Verify the middleware can be created
    let _middleware = ErrorHandlingMiddleware::<()>::new();
}

#[test]
fn test_error_handling_middleware_debug_trait() {
    let middleware = ErrorHandlingMiddleware::<()>::new();
    let debug_str = format!("{:?}", middleware);
    assert!(debug_str.contains("ErrorHandlingMiddleware"));
}
