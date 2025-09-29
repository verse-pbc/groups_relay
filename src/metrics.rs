use anyhow::Result;
use heavykeeper::TopK;
use metrics::{describe_counter, describe_gauge, describe_histogram, Counter, Gauge, Histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
pub use metrics_exporter_prometheus::PrometheusHandle;
use nostr::Kind;
use once_cell::sync::{Lazy, OnceCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tracing::info;

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

/// Tracks the most frequent unknown event kinds using HeavyKeeper algorithm
pub struct UnknownKindTracker {
    top_k: Arc<Mutex<TopK<u16>>>,
    last_report: Arc<Mutex<Instant>>,
    report_interval: Duration,
}

impl UnknownKindTracker {
    /// Create a new tracker for unknown event kinds
    pub fn new() -> Self {
        Self {
            // Track top 10 unknown kinds with good accuracy
            top_k: Arc::new(Mutex::new(TopK::new(10, 1000, 4, 0.9))),
            last_report: Arc::new(Mutex::new(Instant::now())),
            report_interval: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Track an unknown event kind
    pub fn track(&self, kind: u16) {
        if let Ok(mut top_k) = self.top_k.lock() {
            top_k.add(&kind, 1);
        }

        // Check if it's time to report
        self.maybe_report();
    }

    /// Report top unknown kinds if enough time has passed
    fn maybe_report(&self) {
        let now = Instant::now();

        // Check if enough time has passed (without holding the lock)
        let should_report = {
            if let Ok(last_report) = self.last_report.lock() {
                now.duration_since(*last_report) >= self.report_interval
            } else {
                false
            }
        };

        if should_report {
            // Try to update last_report time
            if let Ok(mut last_report) = self.last_report.lock() {
                // Double-check the time with the lock
                if now.duration_since(*last_report) >= self.report_interval {
                    *last_report = now;

                    // Report the top unknown kinds
                    if let Ok(top_k) = self.top_k.lock() {
                        let top_kinds = top_k.list();
                        if !top_kinds.is_empty() {
                            let kinds_info: Vec<String> = top_kinds
                                .iter()
                                .map(|node| format!("kind {} ({} times)", node.item, node.count))
                                .collect();

                            info!(
                                "Top unknown event kinds in the last {} minutes: {}",
                                self.report_interval.as_secs() / 60,
                                kinds_info.join(", ")
                            );
                        }
                    }
                }
            }
        }
    }

    /// Check if a kind should be tracked as unknown
    pub fn is_unknown_kind(kind: u32) -> bool {
        let nostr_kind = Kind::from(kind as u16);
        matches!(nostr_kind, Kind::Custom(k) if !Self::is_known_custom_kind(k))
    }

    /// Check if a custom kind is one we explicitly handle
    fn is_known_custom_kind(kind: u16) -> bool {
        matches!(
            kind,
            // NIP-29 Group management events
            9000 | 9001 | 9002 | 9005 | 9006 | 9007 | 9008 | 9009 | 9021 | 9022 |
            // NIP-29 Group addressable events  
            39000 | 39001 | 39002 | 39003 |
            // Nutzap
            10019 | 9321 |
            // Push notifications
            3079 | 3080 |
            // Other custom kinds we track
            28934
        )
    }
}

impl std::fmt::Debug for UnknownKindTracker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnknownKindTracker")
            .field("report_interval", &self.report_interval)
            .finish()
    }
}
