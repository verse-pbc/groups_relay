use anyhow::Result;
use metrics::{describe_counter, describe_gauge, describe_histogram, Counter, Gauge, Histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
pub use metrics_exporter_prometheus::PrometheusHandle;

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

/// Event processing latency in milliseconds by event kind
pub fn event_latency(kind: u32) -> Histogram {
    let kind_label = match kind {
        // Group management events
        9000 => "add_user",
        9001 => "remove_user",
        9002 => "edit_metadata",
        9005 => "delete_event",
        9006 => "set_roles",
        9007 => "create",
        9008 => "delete",
        9009 => "create_invite",
        9021 => "join_request",
        9022 => "leave_request",
        // Addressable events
        39000 => "metadata",
        39001 => "admins",
        39002 => "members",
        39003 => "roles",
        // NIP-60 Cashu Wallet events
        17375 => "wallet",
        7375 => "token",
        7376 => "spending_history",
        7374 => "quote",
        // NIP-61 Nutzap events
        10019 => "nutzap_info",
        9321 => "nutzap",
        // Other specific events
        10009 => "simple_list",
        28934 => "claim",
        _ => "other",
    };
    metrics::histogram!("event_latency_ms", "kind" => kind_label.to_string())
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

    // Reset gauges to 0 on startup
    active_connections().set(0.0);
    active_subscriptions().set(0.0);
    active_groups().set(0.0);

    Ok(handle)
}
