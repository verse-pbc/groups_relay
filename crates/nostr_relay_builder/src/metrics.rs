//! Metrics traits for relay instrumentation
//!
//! This module provides trait interfaces that allow the relay to report metrics
//! without depending on a specific metrics implementation.

/// Trait for handling subscription metrics
pub trait SubscriptionMetricsHandler: Send + Sync + std::fmt::Debug {
    /// Called when a subscription is added
    fn increment_active_subscriptions(&self);

    /// Called when subscriptions are removed
    fn decrement_active_subscriptions(&self, count: usize);
}

/// Trait for handling event processing metrics
pub trait EventProcessingMetricsHandler: Send + Sync + std::fmt::Debug {
    /// Called when an inbound event is processed
    fn increment_inbound_events_processed(&self);
}

/// A no-op implementation for when metrics are not needed
#[derive(Debug, Clone, Default)]
pub struct NoOpMetricsHandler;

impl SubscriptionMetricsHandler for NoOpMetricsHandler {
    fn increment_active_subscriptions(&self) {}
    fn decrement_active_subscriptions(&self, _count: usize) {}
}

impl EventProcessingMetricsHandler for NoOpMetricsHandler {
    fn increment_inbound_events_processed(&self) {}
}
