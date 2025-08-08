use anyhow::Result;
use metrics::{describe_counter, describe_gauge, describe_histogram, Counter, Gauge, Histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
pub use metrics_exporter_prometheus::PrometheusHandle;
use nostr::Kind;
use once_cell::sync::{Lazy, OnceCell};
use std::collections::HashMap;
use std::sync::RwLock;

/// Global metrics handle to ensure single initialization
static METRICS_HANDLE: OnceCell<PrometheusHandle> = OnceCell::new();

/// Active WebSocket connections gauge
pub fn active_connections() -> Gauge {
    metrics::gauge!("active_connections")
}

/// Total inbound events processed counter
pub fn inbound_events_processed() -> Counter {
    metrics::counter!("inbound_events_processed")
}

/// Active subscriptions gauge
pub fn active_subscriptions() -> Gauge {
    metrics::gauge!("active_subscriptions")
}

/// Total groups created counter
pub fn groups_created() -> Counter {
    metrics::counter!("groups_created")
}

/// Cached histogram instances for event latency
static EVENT_LATENCY_HISTOGRAMS: Lazy<RwLock<HashMap<u32, Histogram>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Get the label for an event kind
fn get_kind_label(kind: u32) -> String {
    let nostr_kind = Kind::from(kind as u16);

    match nostr_kind {
        // For Custom kinds, check if they're ones we handle
        Kind::Custom(k) => match k {
            // NIP-29 Group management events (not in nostr library yet)
            9000 | 9001 | 9002 | 9005 | 9006 | 9007 | 9008 | 9009 | 9021 | 9022 |
            // NIP-29 Group addressable events (not in nostr library yet)  
            39000 | 39001 | 39002 | 39003 |
            // Nutzap (not in nostr library yet)
            10019 | 9321 |
            // Push notifications (not in nostr library yet)
            3079 | 3080 |
            // Other custom kinds we want to track
            28934 => kind.to_string(),
            // Unknown custom kinds
            _ => "other".to_string(),
        },
        // All standard kinds in the nostr library enum
        // This includes CashuWallet (17375), CashuWalletUnspentProof (7375), 
        // CashuWalletSpendingHistory (7376), and many others
        _ => kind.to_string(),
    }
}

/// Event processing latency in milliseconds by event kind
pub fn event_latency(kind: u32) -> Histogram {
    // Try to get from cache first
    if let Ok(cache) = EVENT_LATENCY_HISTOGRAMS.read() {
        if let Some(histogram) = cache.get(&kind) {
            return histogram.clone();
        }
    }

    // Not in cache, need to create it
    let kind_label = get_kind_label(kind);
    let histogram = metrics::histogram!("event_latency_ms", "kind" => kind_label);

    // Store in cache
    if let Ok(mut cache) = EVENT_LATENCY_HISTOGRAMS.write() {
        cache.insert(kind, histogram.clone());
    }

    histogram
}

/// Groups gauge by privacy settings
pub fn groups_by_privacy(private: bool, closed: bool) -> Gauge {
    metrics::gauge!("groups_by_privacy", "private" => private.to_string(), "closed" => closed.to_string())
}

/// Active groups gauge by privacy settings (groups with 2+ members and at least one event)
pub fn active_groups_by_privacy(private: bool, closed: bool) -> Gauge {
    metrics::gauge!("active_groups_by_privacy", "private" => private.to_string(), "closed" => closed.to_string())
}

/// Active groups gauge (groups with 2+ members and 1+ event)
pub fn active_groups() -> Gauge {
    metrics::gauge!("active_groups")
}

/// Sets up the Prometheus recorder and returns a handle that can be used
/// to expose the /metrics endpoint.
pub fn setup_metrics() -> Result<PrometheusHandle, anyhow::Error> {
    // Return existing handle if already initialized
    if let Some(handle) = METRICS_HANDLE.get() {
        return Ok(handle.clone());
    }

    // Initialize only once
    METRICS_HANDLE
        .get_or_try_init(|| {
            // Describe metrics
            describe_counter!("groups_created", "Total number of groups created");
            describe_gauge!(
                "groups_by_privacy",
                "Number of groups by privacy settings (private/public and closed/open)"
            );
            describe_histogram!(
                "event_latency_ms",
                "Event processing latency in milliseconds by event kind"
            );
            describe_gauge!(
                "active_groups_by_privacy",
                "Number of active groups (2+ members and 1+ event) by privacy settings"
            );
            describe_gauge!(
                "active_groups",
                "Number of groups with at least 2 members and 1 event"
            );
            describe_gauge!(
                "active_connections",
                "Number of active WebSocket connections"
            );
            describe_counter!(
                "inbound_events_processed",
                "Total number of inbound events processed"
            );
            describe_gauge!(
                "active_subscriptions",
                "Number of active REQ subscriptions across all connections"
            );

            let builder = PrometheusBuilder::new();
            let handle = builder.install_recorder()?;
            Ok(handle)
        })
        .cloned()
}
