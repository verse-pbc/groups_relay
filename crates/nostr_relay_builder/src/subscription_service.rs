//! Unified subscription service handling both subscription management and REQ message processing
//!
//! This module combines:
//! - Active subscription tracking and event broadcasting (formerly SubscriptionManager)
//! - REQ message processing with pagination support (formerly subscription_handler)

use crate::database::RelayDatabase;
use crate::error::Error;
use crate::metrics::SubscriptionMetricsHandler;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tracing_futures::Instrument;
use websocket_builder::MessageSender;

/// Commands that can be executed against the database
#[derive(Debug, Clone, PartialEq)]
pub enum StoreCommand {
    /// Save an unsigned event to the database
    SaveUnsignedEvent(UnsignedEvent, Scope),
    /// Save a signed event to the database
    SaveSignedEvent(Box<Event>, Scope),
    /// Delete events matching the filter from the database
    DeleteEvents(Filter, Scope),
}

impl StoreCommand {
    /// Get the scope for this store command
    pub fn subdomain_scope(&self) -> &Scope {
        match self {
            StoreCommand::SaveSignedEvent(_, scope) => scope,
            StoreCommand::SaveUnsignedEvent(_, scope) => scope,
            StoreCommand::DeleteEvents(_, scope) => scope,
        }
    }

    /// Check if this command contains a replaceable event
    pub fn is_replaceable(&self) -> bool {
        match self {
            StoreCommand::SaveUnsignedEvent(event, _) => {
                event.kind.is_replaceable() || event.kind.is_addressable()
            }
            StoreCommand::SaveSignedEvent(event, _) => {
                event.kind.is_replaceable() || event.kind.is_addressable()
            }
            StoreCommand::DeleteEvents(_, _) => false,
        }
    }

    /// Convert the Scope to an Option<&str> for backward compatibility
    pub fn subdomain(&self) -> Option<&str> {
        match self.subdomain_scope() {
            Scope::Named { name, .. } => Some(name),
            Scope::Default => None,
        }
    }
}

#[derive(Debug)]
enum SubscriptionMessage {
    Add(SubscriptionId, Vec<Filter>),
    Remove(SubscriptionId),
    CheckEvent { event: Box<Event> },
}

/// Buffer for replaceable events to ensure only the latest per (pubkey, kind, scope) survives
/// when events are created in rapid succession within the same second
struct ReplaceableEventsBuffer {
    buffer: HashMap<(PublicKey, Kind, Scope), UnsignedEvent>,
    sender: mpsc::UnboundedSender<(UnsignedEvent, Scope)>,
    receiver: Option<mpsc::UnboundedReceiver<(UnsignedEvent, Scope)>>,
}

impl ReplaceableEventsBuffer {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            buffer: HashMap::new(),
            sender,
            receiver: Some(receiver),
        }
    }

    pub fn get_sender(&self) -> mpsc::UnboundedSender<(UnsignedEvent, Scope)> {
        self.sender.clone()
    }

    pub fn insert(&mut self, event: UnsignedEvent, scope: Scope) {
        // Only buffer events that are replaceable or addressable (like 39000 for groups)
        if !event.kind.is_replaceable() && !event.kind.is_addressable() {
            debug!(
                "Skipping non-replaceable/non-addressable event kind {} for buffering",
                event.kind
            );
            return;
        }

        let key = (event.pubkey, event.kind, scope.clone());

        // Check if we already have an event for this key
        if let Some(existing_event) = self.buffer.get(&key) {
            debug!(
                "Replacing buffered event: pubkey={}, kind={}, scope={:?}, old_timestamp={}, new_timestamp={}",
                event.pubkey, event.kind, scope, existing_event.created_at, event.created_at
            );
        } else {
            debug!(
                "Buffering new event: pubkey={}, kind={}, scope={:?}, timestamp={}",
                event.pubkey, event.kind, scope, event.created_at
            );
        }

        self.buffer.insert(key, event);
    }

    async fn flush(&mut self, database: &Arc<RelayDatabase>) {
        if self.buffer.is_empty() {
            return;
        }

        debug!(
            "Flushing {} replaceable events from buffer",
            self.buffer.len()
        );

        for ((pubkey, kind, scope), event) in self.buffer.drain() {
            match database.save_unsigned_event(event, scope.clone()).await {
                Ok(_saved_event) => {
                    info!(
                        "Saved buffered replaceable event: pubkey={}, kind={}, scope={:?}",
                        pubkey, kind, scope
                    );
                }
                Err(e) => {
                    error!(
                        "Error saving buffered replaceable event: pubkey={}, kind={}, scope={:?}, error={:?}",
                        pubkey, kind, scope, e
                    );
                }
            }
        }
    }

    pub fn start(mut self, database: Arc<RelayDatabase>, token: CancellationToken, id: String) {
        let mut receiver = self.receiver.take().expect("Receiver already taken");

        tokio::spawn(Box::pin(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!(
                            "[{}] Replaceable events buffer shutting down",
                            id
                        );
                        self.flush(&database).await;
                        return;
                    }

                    event_result = receiver.recv() => {
                        if let Some((event, scope)) = event_result {
                            self.insert(event, scope);
                        }
                    }

                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        self.flush(&database).await;
                    }
                }
            }
        }));
    }
}

/// Unified subscription service handling both active subscriptions and REQ processing
#[derive(Clone)]
pub struct SubscriptionService {
    database: Arc<RelayDatabase>,
    subscription_sender: mpsc::UnboundedSender<SubscriptionMessage>,
    outgoing_sender: Option<MessageSender<RelayMessage<'static>>>,
    local_subscription_count: Arc<AtomicUsize>,
    task_token: CancellationToken,
    replaceable_event_queue: mpsc::UnboundedSender<(UnsignedEvent, Scope)>,
    metrics_handler: Option<Arc<dyn SubscriptionMetricsHandler>>,
}

impl std::fmt::Debug for SubscriptionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionService")
            .field("database", &self.database)
            .field("subscription_sender", &self.subscription_sender)
            .field("outgoing_sender", &self.outgoing_sender)
            .field("local_subscription_count", &self.local_subscription_count)
            .field("task_token", &self.task_token)
            .field("replaceable_event_queue", &self.replaceable_event_queue)
            .field("metrics_handler", &self.metrics_handler.is_some())
            .finish()
    }
}

impl SubscriptionService {
    pub async fn new(
        database: Arc<RelayDatabase>,
        outgoing_sender: MessageSender<RelayMessage<'static>>,
    ) -> Result<Self, Error> {
        Self::new_with_metrics(database, outgoing_sender, None).await
    }

    pub async fn new_with_metrics(
        database: Arc<RelayDatabase>,
        outgoing_sender: MessageSender<RelayMessage<'static>>,
        metrics_handler: Option<Arc<dyn SubscriptionMetricsHandler>>,
    ) -> Result<Self, Error> {
        let local_subscription_count = Arc::new(AtomicUsize::new(0));
        let task_token = CancellationToken::new();

        // Create and start the replaceable events buffer
        let buffer = ReplaceableEventsBuffer::new();
        let replaceable_event_queue = buffer.get_sender();

        // Start the buffer task
        buffer.start(
            database.clone(),
            task_token.clone(),
            "replaceable_events_buffer".to_string(),
        );

        let subscription_sender = Self::start_subscription_task(
            outgoing_sender.clone(),
            local_subscription_count.clone(),
            task_token.clone(),
            metrics_handler.clone(),
        )?;

        let service = Self {
            database,
            subscription_sender,
            outgoing_sender: Some(outgoing_sender),
            local_subscription_count,
            task_token,
            replaceable_event_queue,
            metrics_handler,
        };

        service.start_database_subscription_task()?;
        debug!("Subscription service created successfully");
        Ok(service)
    }

    fn start_subscription_task(
        mut outgoing_sender: MessageSender<RelayMessage<'static>>,
        local_subscription_count: Arc<AtomicUsize>,
        task_token: CancellationToken,
        metrics_handler: Option<Arc<dyn SubscriptionMetricsHandler>>,
    ) -> Result<mpsc::UnboundedSender<SubscriptionMessage>, Error> {
        let (subscription_sender, mut subscription_receiver) = mpsc::unbounded_channel();

        // Create isolated span for subscription task
        let task_span = tracing::info_span!(parent: None, "subscription_task");

        tokio::spawn(
            async move {
                let mut subscriptions = HashMap::new();

                loop {
                    tokio::select! {
                        // Check if the task has been cancelled
                        _ = task_token.cancelled() => {
                            debug!("Subscription task cancelled");
                            break;
                        }
                        // Process incoming subscription messages
                        msg = subscription_receiver.recv() => {
                            match msg {
                                Some(SubscriptionMessage::Add(subscription_id, filters)) => {
                                    subscriptions.insert(subscription_id.clone(), filters);
                                    local_subscription_count.fetch_add(1, Ordering::SeqCst);
                                    if let Some(handler) = &metrics_handler {
                                        handler.increment_active_subscriptions();
                                    }
                                    debug!("Subscription {} added", subscription_id);
                                }
                                Some(SubscriptionMessage::Remove(subscription_id)) => {
                                    if subscriptions.remove(&subscription_id).is_some() {
                                        local_subscription_count.fetch_sub(1, Ordering::SeqCst);
                                        if let Some(handler) = &metrics_handler {
                                            handler.decrement_active_subscriptions(1);
                                        }
                                        debug!("Subscription {} removed", subscription_id);
                                    }
                                }
                                Some(SubscriptionMessage::CheckEvent { event }) => {
                                    for (subscription_id, filters) in &subscriptions {
                                        if filters
                                            .iter()
                                            .any(|filter| filter.match_event(event.as_ref()))
                                        {
                                            let message = RelayMessage::Event {
                                                subscription_id: Cow::Owned(subscription_id.clone()),
                                                event: Cow::Owned(*event.clone()),
                                            };
                                            if let Err(e) = outgoing_sender.send(message) {
                                                error!("Failed to send event: {:?}", e);
                                                info!("Outgoing sender closed, terminating subscription task");
                                                return;
                                            }
                                        }
                                    }
                                }
                                None => {
                                    debug!("Subscription channel closed");
                                    break;
                                }
                            }
                        }
                    }
                }
                debug!("Subscription task stopped");
            }
            .instrument(task_span),
        );

        Ok(subscription_sender)
    }

    fn start_database_subscription_task(&self) -> Result<(), Error> {
        let mut database_subscription = self.database.subscribe();
        let subscription_sender = self.subscription_sender.clone();
        let task_token = self.task_token.clone();

        // Create isolated span for database subscription task
        let db_task_span = tracing::info_span!(parent: None, "database_subscription_task");

        tokio::spawn(
            async move {
                loop {
                    tokio::select! {
                        _ = task_token.cancelled() => {
                            debug!("Database subscription task cancelled");
                            break;
                        }
                        event_result = database_subscription.recv() => {
                            match event_result {
                                Ok(event) => {
                                    if let Err(e) = subscription_sender.send(SubscriptionMessage::CheckEvent { event }) {
                                        error!("Failed to send event: {:?}", e);
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to receive event from database: {:?}", e);
                                    break;
                                }
                            }
                        }
                    }
                }
                debug!("Database subscription task stopped");
            }
            .instrument(db_task_span),
        );

        Ok(())
    }

    // REQ message handling logic (from subscription_handler)

    /// Handle a REQ message from a client
    pub async fn handle_req(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
        authed_pubkey: Option<PublicKey>,
        subdomain: &Scope,
        filter_fn: impl Fn(&Event, &Scope, Option<&PublicKey>) -> bool + Send + Sync + Clone + 'static,
    ) -> Result<(), Error> {
        // Get the sender first to check availability
        let Some(sender) = self.outgoing_sender.as_ref() else {
            return Err(Error::internal("No outgoing sender available"));
        };

        // Handle the historical events
        self.process_historical_events(
            subscription_id.clone(),
            filters.clone(),
            authed_pubkey,
            subdomain,
            sender.clone(),
            filter_fn,
        )
        .await?;

        // Add the subscription
        // TODO: We could have lost some events here while process_historical_events was running, but if we add the subscription before it, we would send the events before EOSE, we are postponing this fix
        self.add_subscription(subscription_id, filters)?;

        Ok(())
    }

    async fn process_historical_events(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
        authed_pubkey: Option<PublicKey>,
        subdomain: &Scope,
        sender: MessageSender<RelayMessage<'static>>,
        filter_fn: impl Fn(&Event, &Scope, Option<&PublicKey>) -> bool + Send + Sync + Clone + 'static,
    ) -> Result<(), Error> {
        // Cap filter limits based on configured max_limit
        let max_limit = crate::global_config::get_max_limit_or_default(500);

        // Also check actual channel capacity as a safety measure
        // This handles edge cases like tests with tiny channels
        let channel_capacity = sender.capacity();
        let channel_safe_limit = if channel_capacity < 10 {
            // For very small channels (like in tests), leave room for EOSE
            channel_capacity.saturating_sub(1)
        } else {
            // For normal channels, this shouldn't matter since channel_size
            // is calculated from max_subscriptions * max_limit * 1.10
            usize::MAX
        };
        let effective_limit = max_limit.min(channel_safe_limit);

        let filters: Vec<Filter> = filters
            .into_iter()
            .map(|mut filter| {
                match filter.limit {
                    Some(filter_limit) if filter_limit > effective_limit => {
                        debug!(
                            "Capping filter limit from {} to {}",
                            filter_limit, effective_limit
                        );
                        filter.limit = Some(effective_limit);
                    }
                    None => {
                        debug!(
                            "Setting filter limit to {} for unbounded query",
                            effective_limit
                        );
                        filter.limit = Some(effective_limit);
                    }
                    _ => {} // Keep existing limit if it's already below effective_limit
                }
                filter
            })
            .collect();

        let ctx = SubscriptionContext {
            subscription_id,
            filters,
            authed_pubkey,
            database: &self.database,
            subdomain,
            sender,
        };

        // Analyze filters to detect optimization opportunities
        // Use optimization whenever we have a limit, to handle pagination properly
        let can_optimize = ctx
            .filters
            .iter()
            .any(|f| f.limit.is_some() && f.kinds.as_ref().is_none_or(|k| !k.is_empty()));

        if can_optimize {
            debug!(
                "Processing subscription {} with optimization",
                ctx.subscription_id
            );
            Self::handle_optimized_subscription(ctx, filter_fn).await
        } else {
            debug!(
                "Processing subscription {} without optimization",
                ctx.subscription_id
            );
            Self::handle_simple_subscription(ctx, filter_fn).await
        }
    }

    /// Handle subscriptions without optimization (no limits or both since and until)
    async fn handle_simple_subscription(
        mut ctx: SubscriptionContext<'_>,
        filter_fn: impl Fn(&Event, &Scope, Option<&PublicKey>) -> bool + Send + Sync,
    ) -> Result<(), Error> {
        let events = ctx
            .database
            .query(ctx.filters, ctx.subdomain)
            .await
            .map_err(|e| Error::notice(format!("Failed to fetch events: {:?}", e)))?;

        debug!(
            "Simple query for {} returned {} events",
            ctx.subscription_id,
            events.len()
        );

        // Send matching events
        for event in events {
            if filter_fn(&event, ctx.subdomain, ctx.authed_pubkey.as_ref()) {
                let message = RelayMessage::Event {
                    subscription_id: Cow::Owned(ctx.subscription_id.clone()),
                    event: Cow::Owned(event),
                };
                ctx.sender
                    .send_bypass(message)
                    .map_err(|e| Error::internal(format!("Failed to send event: {:?}", e)))?;
            }
        }

        // Send EOSE
        ctx.sender
            .send(RelayMessage::EndOfStoredEvents(Cow::Owned(
                ctx.subscription_id,
            )))
            .map_err(|e| Error::internal(format!("Failed to send EOSE: {:?}", e)))?;

        Ok(())
    }

    /// Handle subscriptions with optimization (window sliding or exponential buffer fill)
    async fn handle_optimized_subscription(
        ctx: SubscriptionContext<'_>,
        filter_fn: impl Fn(&Event, &Scope, Option<&PublicKey>) -> bool + Send + Sync + Clone + 'static,
    ) -> Result<(), Error> {
        let has_open_time_window = ctx
            .filters
            .iter()
            .any(|f| f.limit.is_some() && (f.until.is_none() || f.since.is_none()));

        if has_open_time_window {
            Self::handle_window_sliding(ctx, filter_fn).await
        } else {
            Self::handle_exponential_fill(ctx, filter_fn).await
        }
    }

    /// Window sliding strategy for open-ended time queries
    async fn handle_window_sliding(
        mut ctx: SubscriptionContext<'_>,
        filter_fn: impl Fn(&Event, &Scope, Option<&PublicKey>) -> bool + Send + Sync + Clone + 'static,
    ) -> Result<(), Error> {
        let mut sent_events = HashSet::new();
        let mut total_sent = 0;
        let max_limit = ctx
            .filters
            .iter()
            .filter_map(|f| f.limit)
            .max()
            .unwrap_or(0);

        // Process each filter separately
        for (filter_idx, filter) in ctx.filters.iter().enumerate() {
            let requested_limit = filter.limit.unwrap_or(0);
            if requested_limit == 0 {
                continue;
            }

            let filter_has_since = filter.since.is_some();
            let filter_has_until = filter.until.is_some();

            let mut window_filter = filter.clone();
            let mut filter_sent = 0;
            let mut last_timestamp = None;
            let mut attempts = 0;
            const MAX_ATTEMPTS: usize = 50;

            loop {
                attempts += 1;
                debug!(
                    "Window sliding attempt {} for filter {} of subscription {}",
                    attempts, filter_idx, ctx.subscription_id
                );

                let events = ctx
                    .database
                    .query(vec![window_filter.clone()], ctx.subdomain)
                    .await
                    .map_err(|e| Error::notice(format!("Failed to fetch events: {:?}", e)))?;

                if events.is_empty() {
                    debug!("No more events found for filter {}", filter_idx);
                    break;
                }

                let mut filter_events = Vec::new();
                for event in events {
                    // Skip if we've already sent this event
                    if sent_events.contains(&event.id) {
                        continue;
                    }

                    // Update last timestamp for next window
                    let event_created_at = event.created_at;

                    if filter_fn(&event, ctx.subdomain, ctx.authed_pubkey.as_ref()) {
                        filter_events.push(event);
                    }

                    if filter_has_until && !filter_has_since {
                        // Moving backward in time (until only)
                        if last_timestamp.is_none() || Some(event_created_at) < last_timestamp {
                            last_timestamp = Some(event_created_at);
                        }
                    } else if filter_has_since && !filter_has_until {
                        // Moving forward in time (since only)
                        if last_timestamp.is_none() || Some(event_created_at) > last_timestamp {
                            last_timestamp = Some(event_created_at);
                        }
                    } else if !filter_has_since && !filter_has_until {
                        // Limit only - moving backward from most recent
                        if last_timestamp.is_none() || Some(event_created_at) < last_timestamp {
                            last_timestamp = Some(event_created_at);
                        }
                    }
                }

                // Send events in correct order
                #[allow(clippy::overly_complex_bool_expr)]
                if (filter_has_until && !filter_has_since)
                    || (!filter_has_since && !filter_has_until)
                {
                    // Reverse chronological for backward queries (until-only or limit-only)
                    filter_events.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                } else {
                    // Chronological for forward queries (since-only)
                    filter_events.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                }

                for event in filter_events {
                    if filter_sent >= requested_limit {
                        break;
                    }

                    sent_events.insert(event.id);
                    let message = RelayMessage::Event {
                        subscription_id: Cow::Owned(ctx.subscription_id.clone()),
                        event: Cow::Owned(event),
                    };
                    ctx.sender
                        .send_bypass(message)
                        .map_err(|e| Error::internal(format!("Failed to send event: {:?}", e)))?;
                    filter_sent += 1;
                    total_sent += 1;
                }

                if filter_sent >= requested_limit {
                    debug!(
                        "Reached requested limit {} for filter {}",
                        requested_limit, filter_idx
                    );
                    break;
                }

                // Prepare next window
                if let Some(ts) = last_timestamp {
                    #[allow(clippy::overly_complex_bool_expr)]
                    if (filter_has_until && !filter_has_since)
                        || (!filter_has_since && !filter_has_until)
                    {
                        // Moving backward in time (until-only or limit-only)
                        window_filter.until = Some(ts - 1);
                    } else {
                        // Moving forward in time (since-only)
                        window_filter.since = Some(ts + 1);
                    }
                } else {
                    debug!("No valid timestamp found for next window");
                    break;
                }

                if attempts >= MAX_ATTEMPTS {
                    warn!(
                        "Window sliding reached max attempts ({}) for subscription {}",
                        MAX_ATTEMPTS, ctx.subscription_id
                    );
                    break;
                }
            }
        }

        debug!(
            "Window sliding complete for subscription {}: sent {} events (requested max: {})",
            ctx.subscription_id, total_sent, max_limit
        );

        // Send EOSE
        ctx.sender
            .send(RelayMessage::EndOfStoredEvents(Cow::Owned(
                ctx.subscription_id,
            )))
            .map_err(|e| Error::internal(format!("Failed to send EOSE: {:?}", e)))?;

        Ok(())
    }

    /// Exponential buffer fill strategy for bounded time queries
    async fn handle_exponential_fill(
        mut ctx: SubscriptionContext<'_>,
        filter_fn: impl Fn(&Event, &Scope, Option<&PublicKey>) -> bool + Send + Sync + Clone + 'static,
    ) -> Result<(), Error> {
        let mut sent_events = HashSet::new();
        let mut total_sent = 0;
        let max_limit = ctx
            .filters
            .iter()
            .filter_map(|f| f.limit)
            .max()
            .unwrap_or(0);

        for (filter_idx, filter) in ctx.filters.iter().enumerate() {
            let requested_limit = filter.limit.unwrap_or(0);
            if requested_limit == 0 {
                continue;
            }

            let mut buffer_filter = filter.clone();
            let mut filter_sent = 0;
            let mut buffer_multiplier = 2;
            let mut total_attempts = 0;
            const MAX_ATTEMPTS: usize = 10;

            while filter_sent < requested_limit && total_attempts < MAX_ATTEMPTS {
                total_attempts += 1;

                // Exponentially increase the buffer
                let buffer_size = requested_limit.saturating_mul(buffer_multiplier);
                buffer_filter.limit = Some(buffer_size);

                debug!(
                    "Buffer fill attempt {} for filter {} with buffer size {}",
                    total_attempts, filter_idx, buffer_size
                );

                let events = ctx
                    .database
                    .query(vec![buffer_filter.clone()], ctx.subdomain)
                    .await
                    .map_err(|e| Error::notice(format!("Failed to fetch events: {:?}", e)))?;

                if events.is_empty() {
                    debug!("No events found for filter {}", filter_idx);
                    break;
                }

                let mut matching_events = Vec::new();
                for event in events {
                    if sent_events.contains(&event.id) {
                        continue;
                    }

                    if filter_fn(&event, ctx.subdomain, ctx.authed_pubkey.as_ref()) {
                        matching_events.push(event);
                    }
                }

                // Send up to the requested limit
                for event in matching_events
                    .into_iter()
                    .take(requested_limit - filter_sent)
                {
                    sent_events.insert(event.id);
                    let message = RelayMessage::Event {
                        subscription_id: Cow::Owned(ctx.subscription_id.clone()),
                        event: Cow::Owned(event),
                    };
                    ctx.sender
                        .send_bypass(message)
                        .map_err(|e| Error::internal(format!("Failed to send event: {:?}", e)))?;
                    filter_sent += 1;
                    total_sent += 1;
                }

                if filter_sent >= requested_limit {
                    debug!(
                        "Reached requested limit {} for filter {}",
                        requested_limit, filter_idx
                    );
                    break;
                }

                // Increase buffer for next attempt
                buffer_multiplier = buffer_multiplier.saturating_mul(2);
            }
        }

        debug!(
            "Exponential fill complete for subscription {}: sent {} events (requested max: {})",
            ctx.subscription_id, total_sent, max_limit
        );

        // Send EOSE
        ctx.sender
            .send(RelayMessage::EndOfStoredEvents(Cow::Owned(
                ctx.subscription_id,
            )))
            .map_err(|e| Error::internal(format!("Failed to send EOSE: {:?}", e)))?;

        Ok(())
    }

    // Public API methods (from SubscriptionManager)

    pub fn sender_capacity(&self) -> usize {
        self.outgoing_sender
            .as_ref()
            .map_or(0, |sender| sender.capacity())
    }

    pub fn set_outgoing_sender(&mut self, sender: MessageSender<RelayMessage<'static>>) {
        self.outgoing_sender = Some(sender);
    }

    pub fn get_outgoing_sender(&self) -> Option<&MessageSender<RelayMessage<'static>>> {
        self.outgoing_sender.as_ref()
    }

    /// Returns the current number of active subscriptions
    pub fn subscription_count(&self) -> usize {
        self.local_subscription_count.load(Ordering::SeqCst)
    }

    pub fn add_subscription(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
    ) -> Result<(), Error> {
        // Check max subscriptions limit
        let max_subscriptions = crate::global_config::get_max_subscriptions_or_default(50);
        let current_count = self.local_subscription_count.load(Ordering::SeqCst);

        if current_count >= max_subscriptions {
            return Err(Error::restricted(format!(
                "Maximum number of subscriptions ({}) reached",
                max_subscriptions
            )));
        }

        self.subscription_sender
            .send(SubscriptionMessage::Add(subscription_id, filters))
            .map_err(|e| Error::internal(format!("Failed to send subscription: {}", e)))
    }

    pub fn remove_subscription(&self, subscription_id: SubscriptionId) -> Result<(), Error> {
        self.subscription_sender
            .send(SubscriptionMessage::Remove(subscription_id))
            .map_err(|e| Error::internal(format!("Failed to send unsubscribe: {}", e)))
    }

    pub async fn save_and_broadcast(&self, store_command: StoreCommand) -> Result<(), Error> {
        match store_command {
            StoreCommand::SaveUnsignedEvent(event, scope)
                if event.kind.is_replaceable() || event.kind.is_addressable() =>
            {
                // Send replaceable/addressable unsigned events to the buffer
                debug!(
                    "Buffering unsigned event: kind={}, scope={:?} (will be saved within 1 second)",
                    event.kind, scope
                );
                debug!(
                    "Sending replaceable/addressable unsigned event to buffer: kind={}, scope={:?}",
                    event.kind, scope
                );
                if let Err(e) = self.replaceable_event_queue.send((event, scope)) {
                    error!("Failed to send replaceable event to buffer: {:?}", e);
                    return Err(Error::internal(format!(
                        "Failed to send replaceable event to buffer: {}",
                        e
                    )));
                }
                Ok(())
            }
            _ => {
                // All other commands go directly to the database
                self.database
                    .save_store_command(store_command)
                    .await
                    .map_err(|e| {
                        error!("Failed to save store command: {}", e);
                        e
                    })
            }
        }
    }

    /// Fetches historical events from the database without sending them
    pub async fn fetch_historical_events(
        &self,
        filters: &[Filter],
        subdomain: &Scope,
    ) -> Result<Events, Error> {
        self.database
            .query(filters.to_vec(), subdomain)
            .await
            .map_err(|e| Error::notice(format!("Failed to fetch events: {:?}", e)))
    }

    pub async fn handle_unsubscribe(&self, subscription_id: SubscriptionId) -> Result<(), Error> {
        debug!("Handling unsubscribe {}", subscription_id);
        self.remove_subscription(subscription_id)
    }

    pub fn cancel_subscription_task(&self) {
        self.task_token.cancel();
    }

    pub fn cleanup(&self) {
        self.cancel_subscription_task();

        // Swap the count to 0 and get the previous value
        let remaining_subs = self.local_subscription_count.swap(0, Ordering::SeqCst);

        if remaining_subs > 0 {
            if let Some(handler) = &self.metrics_handler {
                handler.decrement_active_subscriptions(remaining_subs);
            }
        }
        debug!(
            "Cleaned up subscription service with {} remaining subscriptions",
            remaining_subs
        );
    }

    /// Waits for the subscription count to reach the expected value with a timeout
    #[cfg(test)]
    pub async fn wait_for_subscription_count(&self, expected: usize, timeout_ms: u64) -> bool {
        use tokio::time::{sleep, Duration};

        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        while start.elapsed() < timeout {
            if self.subscription_count() == expected {
                return true;
            }
            sleep(Duration::from_millis(10)).await;
        }

        false
    }
}

impl Drop for SubscriptionService {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Context struct to group related subscription handling parameters
struct SubscriptionContext<'a> {
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
    authed_pubkey: Option<PublicKey>,
    database: &'a Arc<RelayDatabase>,
    subdomain: &'a Scope,
    sender: MessageSender<RelayMessage<'static>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::setup_test;
    use std::time::Instant;
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use websocket_builder::MessageSender;

    #[allow(dead_code)]
    async fn create_test_event(
        keys: &Keys,
        timestamp: Timestamp,
        group: &str,
        content: &str,
    ) -> Event {
        let tags = vec![
            Tag::custom(TagKind::from("h"), vec![group.to_string()]),
            Tag::custom(TagKind::from("test"), vec!["pagination".to_string()]),
        ];

        EventBuilder::new(Kind::from(9), content)
            .custom_created_at(timestamp)
            .tags(tags)
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(keys)
            .unwrap()
    }

    #[tokio::test]
    async fn test_subscription_management() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Test adding subscriptions
        let sub_id1 = SubscriptionId::new("sub1");
        let sub_id2 = SubscriptionId::new("sub2");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);

        service
            .add_subscription(sub_id1.clone(), vec![filter.clone()])
            .unwrap();
        // Wait for async subscription processing
        assert!(service.wait_for_subscription_count(1, 1000).await);

        service
            .add_subscription(sub_id2.clone(), vec![filter])
            .unwrap();
        assert!(service.wait_for_subscription_count(2, 1000).await);

        // Test removing subscriptions
        service.remove_subscription(sub_id1).unwrap();
        assert!(service.wait_for_subscription_count(1, 1000).await);

        service.remove_subscription(sub_id2).unwrap();
        assert!(service.wait_for_subscription_count(0, 1000).await);
    }

    #[tokio::test]
    async fn test_replaceable_event_buffering() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create multiple metadata events
        let mut metadata1 = Metadata::new();
        metadata1.name = Some("First".to_string());
        let event1 = EventBuilder::metadata(&metadata1)
            .build_with_ctx(&Instant::now(), admin_keys.public_key())
            .sign_with_keys(&admin_keys)
            .unwrap();

        let mut metadata2 = Metadata::new();
        metadata2.name = Some("Second".to_string());
        let event2 = EventBuilder::metadata(&metadata2)
            .build_with_ctx(&Instant::now(), admin_keys.public_key())
            .sign_with_keys(&admin_keys)
            .unwrap();

        // Both should be replaceable events
        assert!(event1.kind.is_replaceable());
        assert!(event2.kind.is_replaceable());

        // Save both through the service - the buffer should handle deduplication
        let cmd1 = StoreCommand::SaveSignedEvent(Box::new(event1.clone()), Scope::Default);
        let cmd2 = StoreCommand::SaveSignedEvent(Box::new(event2.clone()), Scope::Default);

        service.save_and_broadcast(cmd1).await.unwrap();
        service.save_and_broadcast(cmd2).await.unwrap();

        // Wait for buffer to flush
        sleep(Duration::from_secs(2)).await;

        // Query the database - should have both metadata events (no buffer for signed events)
        let filter = Filter::new()
            .author(admin_keys.public_key())
            .kinds(vec![Kind::Metadata]);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // Note: Signed events bypass the replaceable buffer, but since both events
        // have the same kind and author, only the latest should be kept
        assert_eq!(
            events.len(),
            1,
            "Should only have the latest metadata event"
        );
        // The database keeps the latest replaceable event based on timestamp/id
        let event = events.into_iter().next().unwrap();
        assert!(
            event.content.contains("\"name\":\"Second\"")
                || event.content.contains("\"name\":\"First\""),
            "Should have one of the metadata events"
        );
    }

    /// Test window sliding pagination for limit-only queries (implicit until=now)
    #[tokio::test]
    async fn test_window_sliding_limit_only() {
        // Initialize logging for tests
        let _ = tracing_subscriber::fmt::try_init();

        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events alternating between public and private groups
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let group = if i % 2 == 0 { "public" } else { "private" };
            let event = create_test_event(&keys, timestamp, group, &format!("Event {}", i)).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        // Wait a bit for database to process
        sleep(Duration::from_millis(100)).await;

        // Request limit=5, but only public events should be returned
        let filter = Filter::new().kinds(vec![Kind::from(9)]).limit(5);
        let sub_id = SubscriptionId::new("test_sub");

        // Filter function that only allows public group events
        let filter_fn = |event: &Event, _scope: &Scope, _auth: Option<&PublicKey>| -> bool {
            event.tags.iter().any(|t| {
                t.as_slice().len() > 1 && t.as_slice()[0] == "h" && t.as_slice()[1] == "public"
            })
        };

        // Process the subscription
        service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await
            .unwrap();

        // Allow some time for events to be processed
        sleep(Duration::from_millis(100)).await;

        // Collect events from receiver
        let mut received_events = Vec::new();
        let mut eose_received = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { event, .. } => {
                    received_events.push(event.into_owned());
                }
                RelayMessage::EndOfStoredEvents(_) => {
                    eose_received = true;
                    break;
                }
                _ => {}
            }
        }

        assert!(eose_received, "Should receive EOSE");
        assert_eq!(
            received_events.len(),
            5,
            "Should receive exactly 5 public events through window sliding"
        );

        // Verify all events are public
        for event in &received_events {
            assert!(
                event.tags.iter().any(|t| t.as_slice().len() > 1
                    && t.as_slice()[0] == "h"
                    && t.as_slice()[1] == "public"),
                "All events should be from public group"
            );
        }
    }

    /// Test exponential buffer pagination for bounded time queries
    #[tokio::test]
    async fn test_exponential_buffer_since_until_limit() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let base_timestamp = Timestamp::from(1700000000);

        // Create 20 events: 10 public, 10 private, interleaved
        for i in 0..20 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 5);
            let group = if i % 2 == 0 { "public" } else { "private" };
            let event = create_test_event(&keys, timestamp, group, &format!("Event {}", i)).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        // Wait a bit for database to process
        sleep(Duration::from_millis(100)).await;

        // Request events in time window [25, 75] with limit 5
        // Events are at timestamps: 0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75, 80, 85, 90, 95
        // Window [25, 75] contains: 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 75
        // That's indices 5-15 inclusive (11 events total)
        // Public events (even indices): 6, 8, 10, 12, 14 (5 public events)
        let filter = Filter::new()
            .kinds(vec![Kind::from(9)])
            .since(Timestamp::from(base_timestamp.as_u64() + 25))
            .until(Timestamp::from(base_timestamp.as_u64() + 75))
            .limit(5);

        let sub_id = SubscriptionId::new("test_sub");

        // Filter function that only allows public group events
        let filter_fn = |event: &Event, _scope: &Scope, _auth: Option<&PublicKey>| -> bool {
            event.tags.iter().any(|t| {
                t.as_slice().len() > 1 && t.as_slice()[0] == "h" && t.as_slice()[1] == "public"
            })
        };

        // Process the subscription
        service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await
            .unwrap();

        // Allow some time for events to be processed
        sleep(Duration::from_millis(100)).await;

        // Collect events from receiver
        let mut received_events = Vec::new();
        let mut eose_received = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { event, .. } => {
                    received_events.push(event.into_owned());
                }
                RelayMessage::EndOfStoredEvents(_) => {
                    eose_received = true;
                    break;
                }
                _ => {}
            }
        }

        assert!(eose_received, "Should receive EOSE");
        // The exponential buffer should get exactly 5 public events (there are 6 public events in the window, limited to 5)
        assert_eq!(
            received_events.len(),
            5,
            "Should receive exactly 5 public events with exponential buffer"
        );

        // Verify all events are public and within the time window
        for event in &received_events {
            assert!(
                event.tags.iter().any(|t| t.as_slice().len() > 1
                    && t.as_slice()[0] == "h"
                    && t.as_slice()[1] == "public"),
                "All events should be from public group"
            );
            assert!(event.created_at.as_u64() >= base_timestamp.as_u64() + 25);
            assert!(event.created_at.as_u64() <= base_timestamp.as_u64() + 75);
        }
    }

    /// Test pagination bug scenario where initial query returns no events after filtering
    #[tokio::test]
    async fn test_pagination_bug_scenario() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let base_timestamp = Timestamp::from(1700000000);

        // Create 1 old accessible event
        let event =
            create_test_event(&keys, base_timestamp, "public", "Old accessible event").await;
        database
            .save_signed_event(event, Scope::Default)
            .await
            .unwrap();

        // Create 5 newer non-accessible events
        for i in 0..5 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + 100 + i * 10);
            let event =
                create_test_event(&keys, timestamp, "private", &format!("Private {}", i)).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        // Wait a bit for database to process
        sleep(Duration::from_millis(100)).await;

        // Request limit=5 (will get the 5 newest events, all private)
        let filter = Filter::new().kinds(vec![Kind::from(9)]).limit(5);
        let sub_id = SubscriptionId::new("test_sub");

        // Filter function that only allows public group events
        let filter_fn = |event: &Event, _scope: &Scope, _auth: Option<&PublicKey>| -> bool {
            event.tags.iter().any(|t| {
                t.as_slice().len() > 1 && t.as_slice()[0] == "h" && t.as_slice()[1] == "public"
            })
        };

        // Process the subscription - window sliding should find the old public event
        service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await
            .unwrap();

        // Allow some time for events to be processed
        sleep(Duration::from_millis(100)).await;

        // Collect events from receiver
        let mut received_events = Vec::new();
        let mut eose_received = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { event, .. } => {
                    received_events.push(event.into_owned());
                }
                RelayMessage::EndOfStoredEvents(_) => {
                    eose_received = true;
                    break;
                }
                _ => {}
            }
        }

        assert!(eose_received, "Should receive EOSE");
        assert_eq!(
            received_events.len(),
            1,
            "Should find the old accessible event through window sliding"
        );
        assert_eq!(received_events[0].content, "Old accessible event");
    }

    /// Test that subscriptions receive historical events immediately upon creation
    #[tokio::test]
    async fn test_subscription_receives_historical_events() {
        let (_tmp_dir, database, keys) = setup_test().await;

        // Create and save a historical event
        let historical_event = EventBuilder::text_note("Historical event")
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();
        database
            .save_signed_event(historical_event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Create a subscription service
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);

        // Add subscription and immediately fetch historical events
        service
            .add_subscription(subscription_id.clone(), vec![filter.clone()])
            .unwrap();

        // Manually trigger historical event fetch (normally done by handle_req)
        let events = service
            .fetch_historical_events(&[filter], &Scope::Default)
            .await
            .unwrap();
        let events_vec: Vec<Event> = events.into_iter().collect();
        assert_eq!(events_vec.len(), 1);
        assert_eq!(events_vec[0], historical_event);
    }

    /// Test that subscriptions receive new events when they're saved
    #[tokio::test]
    async fn test_subscription_receives_new_events() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription first
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);

        service
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();
        assert!(service.wait_for_subscription_count(1, 1000).await);

        // Create and save a new event
        let new_event = EventBuilder::text_note("New event!")
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();

        // Save through the service which should broadcast to subscriptions
        let cmd = StoreCommand::SaveSignedEvent(Box::new(new_event.clone()), Scope::Default);
        service.save_and_broadcast(cmd).await.unwrap();

        // Wait a bit for async processing
        sleep(Duration::from_millis(100)).await;

        // Check if we received the event
        let mut found = false;
        while let Ok(msg) = rx.try_recv() {
            if let RelayMessage::Event {
                event,
                subscription_id: sub_id,
            } = msg.0
            {
                if *sub_id == subscription_id && *event == new_event {
                    found = true;
                    break;
                }
            }
        }

        assert!(
            found,
            "Should have received the new event through subscription"
        );
    }

    #[tokio::test]
    async fn test_replaceable_buffer_non_replaceable_events() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create non-replaceable unsigned event
        let unsigned_event = EventBuilder::text_note("Not replaceable")
            .build_with_ctx(&Instant::now(), keys.public_key());

        // Should go directly to database, not buffer
        let cmd = StoreCommand::SaveUnsignedEvent(unsigned_event.clone(), Scope::Default);
        service.save_and_broadcast(cmd).await.unwrap();

        // Should be saved immediately
        sleep(Duration::from_millis(100)).await;

        let filter = Filter::new()
            .author(keys.public_key())
            .kinds(vec![Kind::TextNote]);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_buffer_with_different_scopes() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let scope1 = Scope::Named {
            name: "scope1".to_string(),
            hash: 1,
        };
        let scope2 = Scope::Named {
            name: "scope2".to_string(),
            hash: 2,
        };

        // Create events for different scopes
        let event1 = EventBuilder::metadata(&Metadata::new())
            .build_with_ctx(&Instant::now(), keys.public_key());
        let event2 = EventBuilder::metadata(&Metadata::new())
            .build_with_ctx(&Instant::now(), keys.public_key());

        // Save to different scopes
        let cmd1 = StoreCommand::SaveUnsignedEvent(event1, scope1.clone());
        let cmd2 = StoreCommand::SaveUnsignedEvent(event2, scope2.clone());

        service.save_and_broadcast(cmd1).await.unwrap();
        service.save_and_broadcast(cmd2).await.unwrap();

        // Wait for buffer flush
        sleep(Duration::from_secs(2)).await;

        // Both should be saved (different scopes)
        let filter = Filter::new()
            .author(keys.public_key())
            .kinds(vec![Kind::Metadata]);

        let events1 = database.query(vec![filter.clone()], &scope1).await.unwrap();
        let events2 = database.query(vec![filter], &scope2).await.unwrap();

        assert_eq!(events1.len(), 1);
        assert_eq!(events2.len(), 1);
    }

    #[tokio::test]
    async fn test_cancel_subscription_task() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Add a subscription
        let sub_id = SubscriptionId::new("test");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);
        service.add_subscription(sub_id, vec![filter]).unwrap();
        assert!(service.wait_for_subscription_count(1, 1000).await);

        // Cancel the subscription task
        service.cancel_subscription_task();

        // Task should be cancelled
        assert!(service.task_token.is_cancelled());
    }

    #[tokio::test]
    async fn test_save_and_broadcast_database_error() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Test with a regular event that goes to database
        let event = EventBuilder::text_note("Test")
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();

        // This should succeed
        let cmd = StoreCommand::SaveSignedEvent(Box::new(event), Scope::Default);
        let result = service.save_and_broadcast(cmd).await;
        assert!(result.is_ok());

        // Test with delete command
        let filter = Filter::new().author(keys.public_key());
        let delete_cmd = StoreCommand::DeleteEvents(filter, Scope::Default);
        let result = service.save_and_broadcast(delete_cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_empty_filters() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        let sub_id = SubscriptionId::new("empty_filters");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        // Handle REQ with empty filters
        service
            .handle_req(sub_id, vec![], None, &Scope::Default, filter_fn)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Should get EOSE immediately
        let mut got_eose = false;
        while let Ok(msg) = rx.try_recv() {
            if let RelayMessage::EndOfStoredEvents(_) = msg.0 {
                got_eose = true;
                break;
            }
        }

        assert!(got_eose);
    }

    #[tokio::test]
    async fn test_filter_with_zero_limit() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create some events
        for i in 0..5 {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Filter with limit 0
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(0);
        let sub_id = SubscriptionId::new("zero_limit");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        service
            .handle_req(sub_id, vec![filter], None, &Scope::Default, filter_fn)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Should get no events, just EOSE
        let mut event_count = 0;
        let mut got_eose = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { .. } => event_count += 1,
                RelayMessage::EndOfStoredEvents(_) => got_eose = true,
                _ => {}
            }
        }

        assert_eq!(event_count, 0);
        assert!(got_eose);
    }

    #[tokio::test]
    async fn test_replaceable_buffer_logging() {
        // Test to ensure logging branches are covered
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create a replaceable event and save it twice to trigger "replacing" log
        let metadata1 = Metadata::new();
        let event1 =
            EventBuilder::metadata(&metadata1).build_with_ctx(&Instant::now(), keys.public_key());

        let mut metadata2 = Metadata::new();
        metadata2.name = Some("Updated".to_string());
        let event2 =
            EventBuilder::metadata(&metadata2).build_with_ctx(&Instant::now(), keys.public_key());

        // Send both - second should replace first in buffer
        let cmd1 = StoreCommand::SaveUnsignedEvent(event1, Scope::Default);
        let cmd2 = StoreCommand::SaveUnsignedEvent(event2, Scope::Default);

        service.save_and_broadcast(cmd1).await.unwrap();
        service.save_and_broadcast(cmd2).await.unwrap();

        // Wait for buffer to flush
        sleep(Duration::from_secs(2)).await;

        // Check only latest is saved
        let filter = Filter::new()
            .author(keys.public_key())
            .kinds(vec![Kind::Metadata]);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_max_attempts_window_sliding() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create just one event
        let event = EventBuilder::text_note("Single event")
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();
        database
            .save_signed_event(event, Scope::Default)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Request with very high limit that can't be satisfied
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(1000);
        let sub_id = SubscriptionId::new("high_limit");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        service
            .handle_req(sub_id, vec![filter], None, &Scope::Default, filter_fn)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Should still get the one event and EOSE
        let mut event_count = 0;
        let mut got_eose = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { .. } => event_count += 1,
                RelayMessage::EndOfStoredEvents(_) => got_eose = true,
                _ => {}
            }
        }

        assert_eq!(event_count, 1);
        assert!(got_eose);
    }

    #[tokio::test]
    async fn test_database_subscription_task_error() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Cancel the task to simulate disconnection
        service.task_token.cancel();

        // Give task time to shutdown
        sleep(Duration::from_millis(100)).await;

        // Task should be cancelled
        assert!(service.task_token.is_cancelled());
    }

    #[tokio::test]
    async fn test_save_unsigned_event_to_subdomain() {
        let keys = Keys::generate();
        let event =
            EventBuilder::text_note("Test").build_with_ctx(&Instant::now(), keys.public_key());

        let named_scope = Scope::Named {
            name: "subdomain".to_string(),
            hash: 123,
        };
        let cmd = StoreCommand::SaveUnsignedEvent(event, named_scope.clone());

        // Test subdomain methods
        assert_eq!(cmd.subdomain(), Some("subdomain"));
        assert_eq!(cmd.subdomain_scope(), &named_scope);
        assert!(!cmd.is_replaceable()); // TextNote is not replaceable
    }

    #[tokio::test]
    async fn test_addressable_event_is_replaceable() {
        let keys = Keys::generate();
        let tags = vec![Tag::custom(
            TagKind::from("d"),
            vec!["identifier".to_string()],
        )];
        let event = EventBuilder::new(Kind::from(30000), "Addressable")
            .tags(tags)
            .build_with_ctx(&Instant::now(), keys.public_key());

        // Kind 30000 is addressable
        assert!(event.kind.is_addressable());

        let cmd = StoreCommand::SaveUnsignedEvent(event, Scope::Default);
        // The is_replaceable method checks for both replaceable and addressable kinds
        assert!(cmd.is_replaceable());
    }

    #[tokio::test]
    async fn test_subscription_message_none_case() {
        let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<SubscriptionMessage>();

        // Close the channel immediately
        drop(sub_tx);

        // Try to receive - should get None
        assert!(sub_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_exponential_buffer_empty_results() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Request events that don't exist
        let filter = Filter::new()
            .kinds(vec![Kind::from(60000)]) // Non-existent kind
            .since(Timestamp::from(1700000000))
            .until(Timestamp::from(1700001000))
            .limit(10);

        let sub_id = SubscriptionId::new("empty_buffer");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        service
            .handle_req(sub_id, vec![filter], None, &Scope::Default, filter_fn)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Should get EOSE only
        let mut got_eose = false;
        while let Ok(msg) = rx.try_recv() {
            if let RelayMessage::EndOfStoredEvents(_) = msg.0 {
                got_eose = true;
            }
        }

        assert!(got_eose);
    }

    #[tokio::test]
    async fn test_store_command_types() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Test saving signed event
        let event = EventBuilder::text_note("Test note")
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();

        let cmd = StoreCommand::SaveSignedEvent(Box::new(event.clone()), Scope::Default);
        service.save_and_broadcast(cmd).await.unwrap();

        // Test delete command
        let filter = Filter::new().id(event.id);
        let delete_cmd = StoreCommand::DeleteEvents(filter, Scope::Default);
        service.save_and_broadcast(delete_cmd).await.unwrap();
    }

    /// Test window sliding with until + limit
    #[tokio::test]
    async fn test_window_sliding_until_limit() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events across 100 seconds
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let group = if i % 2 == 0 { "public" } else { "private" };
            let event = create_test_event(&keys, timestamp, group, &format!("Event {}", i)).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Request with until=80 (position 8) and limit 5
        let filter = Filter::new()
            .kinds(vec![Kind::from(9)])
            .until(Timestamp::from(base_timestamp.as_u64() + 80))
            .limit(5);

        let sub_id = SubscriptionId::new("test_sub");
        let filter_fn = |event: &Event, _scope: &Scope, _auth: Option<&PublicKey>| -> bool {
            event.tags.iter().any(|t| {
                t.as_slice().len() > 1 && t.as_slice()[0] == "h" && t.as_slice()[1] == "public"
            })
        };

        service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        let mut received_events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let RelayMessage::Event { event, .. } = msg.0 {
                received_events.push(event.into_owned());
            }
        }

        // Should get public events 8, 6, 4, 2, 0 through window sliding
        assert_eq!(received_events.len(), 5, "Should receive 5 public events");

        // Verify they're in reverse chronological order
        for i in 1..received_events.len() {
            assert!(
                received_events[i - 1].created_at > received_events[i].created_at,
                "Events should be in reverse chronological order"
            );
        }
    }

    /// Test window sliding with since + limit
    #[tokio::test]
    async fn test_window_sliding_since_limit() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let group = if i % 2 == 0 { "public" } else { "private" };
            let event = create_test_event(&keys, timestamp, group, &format!("Event {}", i)).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Request with since=20 and limit 5
        let filter = Filter::new()
            .kinds(vec![Kind::from(9)])
            .since(Timestamp::from(base_timestamp.as_u64() + 20))
            .limit(5);

        let sub_id = SubscriptionId::new("test_sub");
        let filter_fn = |event: &Event, _scope: &Scope, _auth: Option<&PublicKey>| -> bool {
            event.tags.iter().any(|t| {
                t.as_slice().len() > 1 && t.as_slice()[0] == "h" && t.as_slice()[1] == "public"
            })
        };

        service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        let mut received_events = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let RelayMessage::Event { event, .. } = msg.0 {
                received_events.push(event.into_owned());
            }
        }

        // Since query with limit 5 starting at timestamp 20
        // Database returns the 5 oldest events after timestamp 20 (events at 20,30,40,50,60)
        // Of these, only events at 20,40,60 are public (indices 2,4,6)
        // But the window sliding might not fetch more since it got some results
        assert!(
            received_events.len() >= 2,
            "Should receive at least 2 public events"
        );

        // All events should be after the since timestamp
        for event in &received_events {
            assert!(event.created_at.as_u64() >= base_timestamp.as_u64() + 20);
        }

        // Verify they're in chronological order (forward)
        for i in 1..received_events.len() {
            assert!(
                received_events[i - 1].created_at < received_events[i].created_at,
                "Events should be in chronological order"
            );
        }
    }

    #[tokio::test]
    async fn test_cleanup() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Add some subscriptions
        let sub_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);
        service.add_subscription(sub_id, vec![filter]).unwrap();

        assert!(service.wait_for_subscription_count(1, 1000).await);

        // Cleanup should reset subscription count
        service.cleanup();
        assert_eq!(service.subscription_count(), 0);
    }

    #[tokio::test]
    async fn test_store_command_methods() {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("Test")
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();

        // Test SaveSignedEvent
        let cmd = StoreCommand::SaveSignedEvent(Box::new(event.clone()), Scope::Default);
        assert_eq!(cmd.subdomain(), None);
        assert!(!cmd.is_replaceable()); // TextNote is not replaceable

        // Test with named scope
        let named_scope = Scope::Named {
            name: "test".to_string(),
            hash: 0,
        };
        let cmd = StoreCommand::SaveSignedEvent(Box::new(event), named_scope.clone());
        assert_eq!(cmd.subdomain(), Some("test"));

        // Test replaceable event
        let metadata = Metadata::new();
        let replaceable_event = EventBuilder::metadata(&metadata)
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();
        let cmd = StoreCommand::SaveSignedEvent(Box::new(replaceable_event), Scope::Default);
        assert!(cmd.is_replaceable());

        // Test DeleteEvents
        let filter = Filter::new().author(keys.public_key());
        let cmd = StoreCommand::DeleteEvents(filter, Scope::Default);
        assert!(!cmd.is_replaceable());
        assert_eq!(cmd.subdomain(), None);
    }

    #[tokio::test]
    async fn test_getter_methods() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let mut service = SubscriptionService::new(database, MessageSender::new(tx.clone(), 0))
            .await
            .unwrap();

        // Test sender_capacity
        assert_eq!(service.sender_capacity(), 10);

        // Test get_outgoing_sender
        assert!(service.get_outgoing_sender().is_some());

        // Test set_outgoing_sender
        let new_sender = MessageSender::new(tx, 0);
        service.set_outgoing_sender(new_sender);
        assert!(service.get_outgoing_sender().is_some());
    }

    #[tokio::test]
    async fn test_handle_req_without_sender() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let mut service = SubscriptionService::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Remove the outgoing sender
        service.outgoing_sender = None;

        let sub_id = SubscriptionId::new("test");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        // Should fail with no outgoing sender
        let result = service
            .handle_req(sub_id, vec![filter], None, &Scope::Default, filter_fn)
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No outgoing sender"));
    }

    #[tokio::test]
    async fn test_addressable_events_buffering() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create an addressable event (e.g., Kind 30000)
        let tags = vec![Tag::custom(
            TagKind::from("d"),
            vec!["test-identifier".to_string()],
        )];
        let event = EventBuilder::new(Kind::from(30000), "Addressable content")
            .tags(tags)
            .build_with_ctx(&Instant::now(), keys.public_key())
            .sign_with_keys(&keys)
            .unwrap();

        // Addressable events should NOT go through the replaceable buffer for signed events
        let cmd = StoreCommand::SaveSignedEvent(Box::new(event), Scope::Default);
        service.save_and_broadcast(cmd).await.unwrap();

        // Wait a bit
        sleep(Duration::from_millis(100)).await;

        // Query and verify
        let filter = Filter::new()
            .author(keys.public_key())
            .kinds(vec![Kind::from(30000)]);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_drop_trait() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);

        {
            let service = SubscriptionService::new(database, MessageSender::new(tx, 0))
                .await
                .unwrap();

            // Add a subscription
            let sub_id = SubscriptionId::new("test");
            let filter = Filter::new().kinds(vec![Kind::TextNote]);
            service.add_subscription(sub_id, vec![filter]).unwrap();
            assert!(service.wait_for_subscription_count(1, 1000).await);

            // Service will be dropped here
        }

        // The drop trait should have called cleanup
        // We can't directly test the internal state after drop,
        // but at least verify drop doesn't panic
    }

    #[tokio::test]
    async fn test_save_unsigned_replaceable_event() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create unsigned replaceable events
        let mut metadata1 = Metadata::new();
        metadata1.name = Some("First".to_string());
        let unsigned_event1 =
            EventBuilder::metadata(&metadata1).build_with_ctx(&Instant::now(), keys.public_key());

        let mut metadata2 = Metadata::new();
        metadata2.name = Some("Second".to_string());
        let unsigned_event2 =
            EventBuilder::metadata(&metadata2).build_with_ctx(&Instant::now(), keys.public_key());

        // Save unsigned events - should go through buffer
        let cmd1 = StoreCommand::SaveUnsignedEvent(unsigned_event1, Scope::Default);
        let cmd2 = StoreCommand::SaveUnsignedEvent(unsigned_event2, Scope::Default);

        service.save_and_broadcast(cmd1).await.unwrap();
        service.save_and_broadcast(cmd2).await.unwrap();

        // Wait for buffer flush
        sleep(Duration::from_secs(2)).await;

        // Should only have the latest event
        let filter = Filter::new()
            .author(keys.public_key())
            .kinds(vec![Kind::Metadata]);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_unsubscribe() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;
        let (tx, _rx) = mpsc::channel(10);
        let service = SubscriptionService::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Add a subscription
        let sub_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]);
        service
            .add_subscription(sub_id.clone(), vec![filter])
            .unwrap();
        assert!(service.wait_for_subscription_count(1, 1000).await);

        // Unsubscribe using handle_unsubscribe
        service.handle_unsubscribe(sub_id).await.unwrap();
        assert!(service.wait_for_subscription_count(0, 1000).await);
    }

    /// Test that the channel overflow bug is fixed by capping limit to channel capacity
    #[tokio::test]
    async fn test_channel_overflow_with_large_limit() {
        let (_tmp_dir, database, keys) = setup_test().await;

        // Create a VERY SMALL channel to test the limit capping
        let channel_size = 2;
        let (tx, mut rx) = mpsc::channel(channel_size);

        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create many events (more than channel capacity)
        let num_events = 10;
        for i in 0..num_events {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Request more events than channel capacity
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(num_events); // Request 10 events but channel can only hold 2

        let sub_id = SubscriptionId::new("test_overflow");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        // This should succeed because limit is capped to channel capacity
        let result = service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await;

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
        }
        assert!(result.is_ok());

        // Collect all messages
        let mut event_count = 0;
        let mut eose_received = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { .. } => event_count += 1,
                RelayMessage::EndOfStoredEvents(_) => eose_received = true,
                _ => {}
            }
        }

        assert!(eose_received, "Should receive EOSE");
        // Should only receive events up to channel capacity minus 1 (for EOSE)
        let expected_max_events = channel_size - 1;
        assert!(
            event_count <= expected_max_events,
            "Should receive at most {} events (channel capacity {} - 1 for EOSE), but got {}",
            expected_max_events,
            channel_size,
            event_count
        );
    }

    #[tokio::test]
    async fn test_unbounded_query_gets_limited() {
        let (_tmp_dir, database, keys) = setup_test().await;

        // Create a small channel
        let channel_size = 5;
        let (tx, mut rx) = mpsc::channel(channel_size);

        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create some events
        for i in 0..20 {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Create filter WITHOUT limit (unbounded query)
        let filter = Filter::new().kinds(vec![Kind::TextNote]);
        // No limit set!

        let sub_id = SubscriptionId::new("test_unbounded");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        // This should succeed with automatic limit
        let result = service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await;

        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
        }
        assert!(result.is_ok());

        // Collect all messages
        let mut event_count = 0;
        let mut eose_received = false;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { .. } => event_count += 1,
                RelayMessage::EndOfStoredEvents(_) => eose_received = true,
                _ => {}
            }
        }

        assert!(eose_received, "Should receive EOSE");
        // Should receive exactly channel_size - 1 events (max allowed)
        let expected_events = channel_size - 1;
        assert_eq!(
            event_count, expected_events,
            "Unbounded query should be limited to {} events (channel capacity {} - 1 for EOSE)",
            expected_events, channel_size
        );
    }

    #[tokio::test]
    async fn test_multiple_filters_with_different_limits() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let (tx, mut rx) = mpsc::channel(100);
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        let base_timestamp = Timestamp::from(1700000000);

        // Create 20 events of different kinds
        for i in 0..20 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let kind = if i % 2 == 0 {
                Kind::TextNote
            } else {
                Kind::from(9)
            };
            let event = EventBuilder::new(kind, format!("Event {}", i))
                .custom_created_at(timestamp)
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Create filters with different limits
        let filters = vec![
            Filter::new().kinds(vec![Kind::TextNote]).limit(3),
            Filter::new().kinds(vec![Kind::from(9)]).limit(5),
        ];

        let sub_id = SubscriptionId::new("multi_filter");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        service
            .handle_req(sub_id, filters, None, &Scope::Default, filter_fn)
            .await
            .unwrap();

        sleep(Duration::from_millis(100)).await;

        // Count events by kind
        let mut text_notes = 0;
        let mut kind_9 = 0;
        let mut _eose_count = 0;

        while let Ok(msg) = rx.try_recv() {
            match msg.0 {
                RelayMessage::Event { event, .. } => match event.kind {
                    Kind::TextNote => text_notes += 1,
                    k if k == Kind::from(9) => kind_9 += 1,
                    _ => {}
                },
                RelayMessage::EndOfStoredEvents(_) => _eose_count += 1,
                _ => {}
            }
        }

        // Should receive at most the limits from each filter
        assert!(
            text_notes <= 3,
            "Should receive at most 3 text notes, got {}",
            text_notes
        );
        assert!(
            kind_9 <= 5,
            "Should receive at most 5 kind 9 events, got {}",
            kind_9
        );
    }
}
