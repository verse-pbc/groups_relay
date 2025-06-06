//! Utility functions for the relay builder

use std::sync::OnceLock;
use tokio::runtime::Runtime;

/// Shared runtime for CPU-intensive operations that need to be executed in a blocking context.
///
/// This runtime is used by spawn_blocking operations that need to execute async code,
/// such as event signing and verification. Using a shared runtime prevents resource
/// exhaustion that would occur if we created a new runtime for each operation.
pub static BLOCKING_RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get or initialize the blocking runtime.
pub fn get_blocking_runtime() -> &'static Runtime {
    BLOCKING_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all() // Enable all features including time and IO
            .build()
            .unwrap()
    })
}
