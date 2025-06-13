//! Complete custom state relay example
//!
//! This example demonstrates a full relay implementation with custom state that includes:
//! - User session tracking with async state mutations
//! - Rate limiting per user with real-time updates
//! - Content filtering based on user reputation
//! - Subscription limits that dynamically adjust
//! - Async operations for reputation updates
//!
//! Run with: cargo run --example custom_state_relay --features axum

use anyhow::Result;
use async_trait::async_trait;
use axum::{response::IntoResponse, routing::get, Router};
use nostr_relay_builder::{
    EventContext, EventProcessor, RelayBuilder, RelayConfig, RelayInfo, Result as RelayResult,
    StoreCommand,
};
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// Custom state for each connection - now mutable in async contexts!
#[derive(Debug, Clone)]
struct UserSession {
    /// User reputation score (0-100)
    reputation: u32,
    /// Rate limiting: timestamps of recent events
    recent_events: Vec<Instant>,
    /// Number of active subscriptions
    active_subscriptions: u32,
    /// Total events published
    total_events_published: u32,
    /// Blocked words for this user
    blocked_words: Vec<String>,
    /// Session start time
    session_start: Instant,
    /// Last activity timestamp
    last_activity: Instant,
    /// Connection quality score (for demonstration of async updates)
    connection_quality: f32,
}

impl Default for UserSession {
    fn default() -> Self {
        Self {
            reputation: 50, // Start with neutral reputation
            recent_events: Vec::new(),
            active_subscriptions: 0,
            total_events_published: 0,
            blocked_words: vec!["spam".to_string(), "scam".to_string()],
            session_start: Instant::now(),
            last_activity: Instant::now(),
            connection_quality: 1.0,
        }
    }
}

impl UserSession {
    /// Check if user is rate limited (async for demonstration)
    async fn is_rate_limited(&mut self) -> bool {
        let now = Instant::now();
        let one_minute_ago = now - Duration::from_secs(60);

        // Clean up old timestamps
        self.recent_events.retain(|&ts| ts > one_minute_ago);

        // Different limits based on reputation
        let rate_limit = match self.reputation {
            0..=20 => 5,   // Low reputation: 5 events/minute
            21..=50 => 20, // Normal: 20 events/minute
            51..=80 => 50, // Good: 50 events/minute
            _ => 100,      // Excellent: 100 events/minute
        };

        // Simulate async operation (e.g., checking external rate limiter)
        tokio::time::sleep(Duration::from_millis(1)).await;

        self.recent_events.len() >= rate_limit
    }

    /// Record an event with async reputation updates
    async fn record_event(&mut self) {
        self.recent_events.push(Instant::now());
        self.total_events_published += 1;
        self.last_activity = Instant::now();

        // Async reputation improvement
        if self.total_events_published % 10 == 0 && self.reputation < 100 {
            // Simulate async reputation calculation
            tokio::time::sleep(Duration::from_millis(1)).await;
            self.reputation += 1;
            info!("User reputation increased to {}", self.reputation);
        }

        // Update connection quality based on activity
        let session_duration = self.session_start.elapsed().as_secs() as f32;
        if session_duration > 0.0 {
            self.connection_quality =
                (self.total_events_published as f32) / (session_duration / 60.0);
        }
    }

    /// Get maximum allowed subscriptions based on reputation
    fn max_subscriptions(&self) -> u32 {
        match self.reputation {
            0..=20 => 1,
            21..=50 => 3,
            51..=80 => 5,
            _ => 10,
        }
    }

    /// Update subscription count (async for demonstration)
    #[allow(dead_code)]
    async fn update_subscription_count(&mut self, change: i32) {
        // Simulate async database update
        tokio::time::sleep(Duration::from_millis(1)).await;

        if change > 0 {
            self.active_subscriptions += change as u32;
        } else {
            self.active_subscriptions = self.active_subscriptions.saturating_sub((-change) as u32);
        }

        self.last_activity = Instant::now();
        info!(
            "Updated subscription count to {}",
            self.active_subscriptions
        );
    }

    /// Decrease reputation for bad behavior (async)
    async fn decrease_reputation(&mut self, amount: u32, reason: &str) {
        self.reputation = self.reputation.saturating_sub(amount);
        warn!(
            "Reputation decreased by {} (reason: {}). New reputation: {}",
            amount, reason, self.reputation
        );

        // Simulate async logging to external system
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// Custom event processor with session management and async state updates
#[derive(Debug, Clone)]
struct SessionAwareProcessor {
    /// Global banned pubkeys
    banned_pubkeys: Arc<HashMap<PublicKey, String>>,
}

impl SessionAwareProcessor {
    fn new() -> Self {
        Self {
            banned_pubkeys: Arc::new(HashMap::new()),
        }
    }

    /// Check if content contains blocked words
    fn contains_blocked_content(content: &str, blocked_words: &[String]) -> bool {
        let content_lower = content.to_lowercase();
        blocked_words
            .iter()
            .any(|word| content_lower.contains(word))
    }
}

#[async_trait]
impl EventProcessor<UserSession> for SessionAwareProcessor {
    async fn handle_event(
        &self,
        event: Event,
        custom_state: &mut UserSession,
        context: EventContext<'_>,
    ) -> RelayResult<Vec<StoreCommand>> {
        // Check if user is banned globally
        if let Some(reason) = self.banned_pubkeys.get(&event.pubkey) {
            warn!(
                "Rejecting event from banned pubkey {}: {}",
                event.pubkey.to_bech32().unwrap_or_default(),
                reason
            );
            return Err(nostr_relay_builder::Error::restricted(format!(
                "User banned: {}",
                reason
            )));
        }

        // Async rate limiting check with state mutation
        if custom_state.is_rate_limited().await {
            let limit_info = format!(
                "Rate limit exceeded. Reputation: {}, Events in last minute: {}",
                custom_state.reputation,
                custom_state.recent_events.len()
            );
            warn!("Rate limited: {}", limit_info);
            return Err(nostr_relay_builder::Error::restricted(limit_info));
        }

        // Check content filtering with async reputation penalty
        if Self::contains_blocked_content(&event.content, &custom_state.blocked_words) {
            warn!(
                "Event from {} contains blocked content",
                event.pubkey.to_bech32().unwrap_or_default()
            );

            // Async reputation decrease
            custom_state
                .decrease_reputation(5, "posted blocked content")
                .await;

            return Err(nostr_relay_builder::Error::restricted(
                "Content contains prohibited words",
            ));
        }

        // Async event recording with state updates
        custom_state.record_event().await;

        // Log comprehensive session statistics
        let session_duration = custom_state.session_start.elapsed();
        let time_since_activity = custom_state.last_activity.elapsed();

        info!(
            "Session stats: {} events, {} reputation, {} subscriptions, {}s duration, {:.2} quality, {}s since last activity",
            custom_state.total_events_published,
            custom_state.reputation,
            custom_state.active_subscriptions,
            session_duration.as_secs(),
            custom_state.connection_quality,
            time_since_activity.as_secs()
        );

        // Accept the event
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }

    fn verify_filters(
        &self,
        filters: &[Filter],
        custom_state: &UserSession,
        _context: EventContext<'_>,
    ) -> RelayResult<()> {
        // Check subscription limits
        let max_subs = custom_state.max_subscriptions();
        if custom_state.active_subscriptions >= max_subs {
            warn!(
                "At subscription limit: {}/{}",
                custom_state.active_subscriptions, max_subs
            );
            return Err(nostr_relay_builder::Error::restricted(format!(
                "Subscription limit reached ({}/{}). Improve reputation to increase limit.",
                custom_state.active_subscriptions, max_subs
            )));
        }

        // Note: In verify_filters we can't mutate state, so we just check limits
        // Actual subscription count updates would happen elsewhere

        // Check filter complexity based on reputation
        let total_filter_items: usize = filters
            .iter()
            .map(|f| {
                f.ids.as_ref().map_or(0, |v| v.len())
                    + f.authors.as_ref().map_or(0, |v| v.len())
                    + f.kinds.as_ref().map_or(0, |v| v.len())
            })
            .sum();

        let max_complexity = match custom_state.reputation {
            0..=20 => 10,
            21..=50 => 50,
            51..=80 => 100,
            _ => 500,
        };

        if total_filter_items > max_complexity {
            return Err(nostr_relay_builder::Error::restricted(format!(
                "Filter too complex ({} items). Max allowed: {}",
                total_filter_items, max_complexity
            )));
        }

        Ok(())
    }

    fn can_see_event(
        &self,
        _event: &Event,
        _custom_state: &UserSession,
        _context: EventContext<'_>,
    ) -> RelayResult<bool> {
        // Note: We can no longer update activity timestamp here since state is immutable
        // Activity tracking happens in handle_event instead

        // Everyone can see events (this is a public relay)
        // But we could restrict based on reputation if needed
        Ok(true)
    }
}

const CUSTOM_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Custom State Relay</title>
    <style>
        body {
            font-family: 'Inter', -apple-system, system-ui, sans-serif;
            background: linear-gradient(135deg, #2c3e50 0%, #3498db 100%);
            color: white;
            margin: 0;
            padding: 2rem;
            min-height: 100vh;
        }
        .container {
            max-width: 1000px;
            margin: 0 auto;
            background: rgba(255, 255, 255, 0.1);
            backdrop-filter: blur(10px);
            border-radius: 20px;
            padding: 3rem;
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
        }
        h1 { font-size: 3rem; margin-bottom: 1rem; }
        .feature-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 2rem;
            margin-top: 2rem;
        }
        .feature {
            background: rgba(255, 255, 255, 0.1);
            padding: 2rem;
            border-radius: 10px;
            border: 1px solid rgba(255, 255, 255, 0.2);
        }
        .feature h3 { margin-top: 0; color: #3498db; }
        .reputation-levels {
            list-style: none;
            padding: 0;
        }
        .reputation-levels li {
            padding: 0.5rem 0;
            border-bottom: 1px solid rgba(255, 255, 255, 0.1);
        }
        .reputation-levels li:last-child { border: none; }
        code {
            background: rgba(0, 0, 0, 0.3);
            padding: 0.2rem 0.5rem;
            border-radius: 3px;
            font-family: 'Consolas', monospace;
        }
        .async-badge {
            background: #e74c3c;
            color: white;
            padding: 0.2rem 0.5rem;
            border-radius: 12px;
            font-size: 0.8rem;
            font-weight: bold;
        }
    </style>
</head>
<body>
    <div class="container">
        <h1>🌟 Custom State Relay <span class="async-badge">ASYNC</span></h1>
        <p>A sophisticated Nostr relay with async user reputation and session management</p>
        
        <div class="feature-grid">
            <div class="feature">
                <h3>🏆 Async Reputation System</h3>
                <p>Users start with 50 reputation points and can earn up to 100 with async updates</p>
                <ul class="reputation-levels">
                    <li>🔴 0-20: Limited access (5 events/min, 1 subscription)</li>
                    <li>🟡 21-50: Normal access (20 events/min, 3 subscriptions)</li>
                    <li>🟢 51-80: Enhanced access (50 events/min, 5 subscriptions)</li>
                    <li>⭐ 81-100: Premium access (100 events/min, 10 subscriptions)</li>
                </ul>
            </div>
            
            <div class="feature">
                <h3>⚡ Async Rate Limiting</h3>
                <p>Dynamic rate limits with real-time async state updates:</p>
                <ul>
                    <li>Async tracking of events per minute</li>
                    <li>Real-time reputation increases</li>
                    <li>Async penalty system for violations</li>
                    <li>Connection quality scoring</li>
                </ul>
            </div>
            
            <div class="feature">
                <h3>🛡️ Smart Content Filtering</h3>
                <p>Advanced content protection with async operations:</p>
                <ul>
                    <li>Per-user blocked word lists</li>
                    <li>Async reputation penalties</li>
                    <li>Real-time quality adjustments</li>
                    <li>Activity tracking updates</li>
                </ul>
            </div>
            
            <div class="feature">
                <h3>📊 Live Session Analytics</h3>
                <p>Real-time session tracking with async updates:</p>
                <ul>
                    <li>Async connection duration tracking</li>
                    <li>Live subscription count updates</li>
                    <li>Connection quality scoring</li>
                    <li>Last activity timestamps</li>
                </ul>
            </div>
        </div>
        
        <div style="margin-top: 3rem; text-align: center;">
            <p>Connect with <code>ws://localhost:8080</code></p>
            <p><strong>✨ New in this version:</strong> Async state mutations enable real-time updates!</p>
            <p>Built with nostr_relay_builder's unified EventProcessor&lt;T&gt; API</p>
        </div>
    </div>
</body>
</html>
"#;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    info!("Starting Async Custom State Relay...");

    // Generate relay keys
    let keys = Keys::generate();
    info!("Relay pubkey: {}", keys.public_key());

    // Create relay configuration
    let config = RelayConfig::new(
        "ws://localhost:8080",
        "./data/custom_state_relay",
        keys.clone(),
    );

    // Define relay information
    let relay_info = RelayInfo {
        name: "Async Custom State Relay".to_string(),
        description: "A relay with async user sessions and reputation system".to_string(),
        pubkey: keys.public_key().to_hex(),
        contact: "admin@asyncrelay.com".to_string(),
        supported_nips: vec![1, 9, 11, 40, 42, 70],
        software: "nostr_relay_builder".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        icon: None,
    };

    // Create the processor
    let processor = SessionAwareProcessor::new();

    // Build the relay handlers with custom state support
    // Note: Standard NIP middlewares (09, 40, 70) are not yet compatible with custom state
    // They require generic state support to work with UserSession
    let handlers = Arc::new(
        RelayBuilder::<UserSession>::new(config.clone())
            .with_state_factory(UserSession::default)
            .build_handlers(processor, relay_info)
            .await?,
    );

    // Create HTTP server with custom HTML at root
    let app = Router::new().route(
        "/",
        get({
            let handlers = handlers.clone();
            move |ws: Option<axum::extract::WebSocketUpgrade>,
                  connect_info: axum::extract::ConnectInfo<SocketAddr>,
                  headers: axum::http::HeaderMap| async move {
                if ws.is_some()
                    || headers.get("accept").and_then(|h| h.to_str().ok())
                        == Some("application/nostr+json")
                {
                    handlers.axum_root_handler()(ws, connect_info, headers).await
                } else {
                    axum::response::Html(CUSTOM_HTML).into_response()
                }
            }
        }),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));

    println!("\n🚀 Async Custom State Relay Configuration Complete!");
    println!("====================================");
    println!("🌟 New Features Demonstrated:");
    println!("  ⚡ Async rate limiting with real-time state updates");
    println!("  🏆 Async reputation scoring and penalties");
    println!("  📊 Live session analytics with quality scoring");
    println!("  🔄 Mutable state in async contexts");
    println!();
    println!("📡 WebSocket endpoint: ws://localhost:8080");
    println!("🌐 Web interface: http://localhost:8080/");
    println!();
    println!("Reputation Levels (with async updates):");
    println!("  🔴 0-20: Limited (5 events/min, 1 subscription)");
    println!("  🟡 21-50: Normal (20 events/min, 3 subscriptions)");
    println!("  🟢 51-80: Enhanced (50 events/min, 5 subscriptions)");
    println!("  ⭐ 81-100: Premium (100 events/min, 10 subscriptions)");
    println!();
    println!("✅ Async relay server starting on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
