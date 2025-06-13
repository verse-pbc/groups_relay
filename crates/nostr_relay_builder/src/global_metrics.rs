//! Global metrics context for the relay
//!
//! This module provides a global context for metrics handlers that need to be
//! accessed from different parts of the relay system.

use crate::metrics::SubscriptionMetricsHandler;
use once_cell::sync::OnceCell;
use std::sync::Arc;

static SUBSCRIPTION_METRICS_HANDLER: OnceCell<Arc<dyn SubscriptionMetricsHandler>> =
    OnceCell::new();

/// Set the global subscription metrics handler
pub fn set_subscription_metrics_handler(handler: Arc<dyn SubscriptionMetricsHandler>) {
    SUBSCRIPTION_METRICS_HANDLER
        .set(handler)
        .unwrap_or_else(|_| panic!("Subscription metrics handler already set"));
}

/// Get the global subscription metrics handler
pub fn get_subscription_metrics_handler() -> Option<Arc<dyn SubscriptionMetricsHandler>> {
    SUBSCRIPTION_METRICS_HANDLER.get().cloned()
}
