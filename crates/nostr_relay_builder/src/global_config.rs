//! Global configuration values that can be accessed throughout the relay
//!
//! This module provides global configuration values using LazyLock to avoid
//! threading configuration through multiple layers of the application.

use std::sync::LazyLock;
use std::sync::RwLock;

/// Global query limit configuration
static QUERY_LIMIT: LazyLock<RwLock<Option<usize>>> = LazyLock::new(|| RwLock::new(None));

/// Set the global query limit
pub fn set_query_limit(limit: usize) {
    let mut query_limit = QUERY_LIMIT.write().unwrap();
    *query_limit = Some(limit);
}

/// Get the global query limit
pub fn get_query_limit() -> Option<usize> {
    *QUERY_LIMIT.read().unwrap()
}

/// Get the query limit or a default value
pub fn get_query_limit_or_default(default: usize) -> usize {
    get_query_limit().unwrap_or(default)
}
