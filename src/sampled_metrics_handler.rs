use crate::metrics;
use relay_builder::middlewares::MetricsHandler;
use std::sync::atomic::{AtomicU64, Ordering};

/// A metrics handler that samples events to reduce overhead
///
/// This handler only records metrics for a configurable percentage of events
/// to avoid the performance overhead of recording every single event.
#[derive(Debug)]
pub struct SampledMetricsHandler {
    /// Counter for events - used to determine which events to sample
    event_counter: AtomicU64,
    /// Sample rate - record 1 out of every N events
    sample_rate: u64,
}

impl SampledMetricsHandler {
    /// Create a new sampled metrics handler
    ///
    /// sample_rate: Record metrics for 1 out of every N events
    /// For example, sample_rate = 10 means record 10% of events
    pub fn new(sample_rate: u64) -> Self {
        Self {
            event_counter: AtomicU64::new(0),
            sample_rate: sample_rate.max(1), // Ensure at least 1
        }
    }
}

impl MetricsHandler for SampledMetricsHandler {
    fn record_event_latency(&self, kind: u32, latency_ms: f64) {
        // We already sampled in should_track_latency, so just record
        // Multiply by sample rate to extrapolate the sampled data
        metrics::event_latency(kind).record(latency_ms);
    }

    fn increment_active_connections(&self) {
        // Always track connections (not sampled)
        metrics::active_connections().increment(1.0);
    }

    fn decrement_active_connections(&self) {
        // Always track connections (not sampled)
        metrics::active_connections().decrement(1.0);
    }

    fn increment_inbound_events_processed(&self) {
        // Always increment the counter to have accurate counts
        // Don't sample this metric as it's a simple counter increment
        metrics::inbound_events_processed().increment(1);
    }

    fn should_track_latency(&self) -> bool {
        // Increment counter and decide if we should track this event
        // This method should only be called once per event by the middleware
        let count = self.event_counter.fetch_add(1, Ordering::Relaxed);
        count % self.sample_rate == 0
    }
}
