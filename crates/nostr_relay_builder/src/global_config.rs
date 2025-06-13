//! Global configuration values that can be accessed throughout the relay
//!
//! This module provides global configuration values using LazyLock to avoid
//! threading configuration through multiple layers of the application.

use std::sync::LazyLock;
use std::sync::RwLock;

/// Global max subscriptions configuration
static MAX_SUBSCRIPTIONS: LazyLock<RwLock<Option<usize>>> = LazyLock::new(|| RwLock::new(None));

/// Global max limit configuration
static MAX_LIMIT: LazyLock<RwLock<Option<usize>>> = LazyLock::new(|| RwLock::new(None));

/// Set the global max subscriptions limit
pub fn set_max_subscriptions(limit: usize) {
    let mut max_subscriptions = MAX_SUBSCRIPTIONS.write().unwrap();
    *max_subscriptions = Some(limit);
}

/// Get the global max subscriptions limit
pub fn get_max_subscriptions() -> Option<usize> {
    *MAX_SUBSCRIPTIONS.read().unwrap()
}

/// Get the max subscriptions limit or a default value
pub fn get_max_subscriptions_or_default(default: usize) -> usize {
    get_max_subscriptions().unwrap_or(default)
}

/// Set the global max limit
pub fn set_max_limit(limit: usize) {
    let mut max_limit = MAX_LIMIT.write().unwrap();
    *max_limit = Some(limit);
}

/// Get the global max limit
pub fn get_max_limit() -> Option<usize> {
    *MAX_LIMIT.read().unwrap()
}

/// Get the max limit or a default value
pub fn get_max_limit_or_default(default: usize) -> usize {
    get_max_limit().unwrap_or(default)
}
