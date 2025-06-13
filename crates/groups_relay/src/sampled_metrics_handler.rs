use crate::metrics;
use nostr_relay_builder::middlewares::MetricsHandler;
use std::sync::atomic::{AtomicU64, Ordering};

/// A metrics handler that samples events to reduce overhead
///
/// This handler only records metrics for a configurable percentage of events
/// to avoid the performance overhead of recording every single event.
#[derive(Debug)]
pub struct SampledMetricsHandler {
    /// Counter for sampling - we record metrics for every Nth event
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

    /// Check if we should sample this event
    fn should_sample(&self) -> bool {
        let count = self.event_counter.fetch_add(1, Ordering::Relaxed);
        count % self.sample_rate == 0
    }
}

impl MetricsHandler for SampledMetricsHandler {
    fn record_event_latency(&self, kind: u32, latency_ms: f64) {
        // We already sampled in should_track_latency, so just record
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
        // Use sampling for event counts too
        if self.should_sample() {
            // Multiply by sample rate to extrapolate
            metrics::inbound_events_processed().increment(self.sample_rate);
        }
    }

    fn should_track_latency(&self) -> bool {
        // This is the key optimization - only call Instant::now() for sampled events
        self.should_sample()
    }
}
