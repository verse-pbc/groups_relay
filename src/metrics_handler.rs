use crate::metrics;
use nostr_relay_builder::middlewares::MetricsHandler;

/// Implementation of MetricsHandler that updates Prometheus metrics
#[derive(Debug, Clone)]
pub struct PrometheusMetricsHandler;

impl MetricsHandler for PrometheusMetricsHandler {
    fn record_event_latency(&self, kind: u32, latency_ms: f64) {
        metrics::event_latency(kind).record(latency_ms);
    }

    fn increment_active_connections(&self) {
        metrics::active_connections().increment(1.0);
    }

    fn decrement_active_connections(&self) {
        metrics::active_connections().decrement(1.0);
    }

    fn increment_inbound_events_processed(&self) {
        metrics::inbound_events_processed().increment(1);
    }

    fn should_track_latency(&self) -> bool {
        true // Always track for full metrics
    }
}

/// Trait for handling subscription metrics
pub trait SubscriptionMetricsHandler: Send + Sync + std::fmt::Debug {
    fn increment_active_subscriptions(&self);
    fn decrement_active_subscriptions(&self, count: usize);
}

/// Implementation of SubscriptionMetricsHandler for Prometheus
#[derive(Debug, Clone)]
pub struct PrometheusSubscriptionMetricsHandler;

impl SubscriptionMetricsHandler for PrometheusSubscriptionMetricsHandler {
    fn increment_active_subscriptions(&self) {
        metrics::active_subscriptions().increment(1.0);
    }

    fn decrement_active_subscriptions(&self, count: usize) {
        metrics::active_subscriptions().decrement(count as f64);
    }
}

// Also implement the nostr_relay_builder trait
impl nostr_relay_builder::metrics::SubscriptionMetricsHandler
    for PrometheusSubscriptionMetricsHandler
{
    fn increment_active_subscriptions(&self) {
        metrics::active_subscriptions().increment(1.0);
    }

    fn decrement_active_subscriptions(&self, count: usize) {
        metrics::active_subscriptions().decrement(count as f64);
    }
}
