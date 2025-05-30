use crate::error::Error;
use crate::nostr_database::RelayDatabase;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use snafu::Backtrace;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use tracing_futures::Instrument;
use websocket_builder::MessageSender;

#[derive(Debug)]
enum SubscriptionMessage {
    Add(SubscriptionId, Vec<Filter>),
    Remove(SubscriptionId),
    CheckEvent { event: Box<Event> },
}

// Buffer for replaceable events to ensure only the latest per (pubkey, kind, scope) survives
// when events are created in rapid succession within the same second
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
                    // Optionally broadcast the event here if needed
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

#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    database: Arc<RelayDatabase>,
    subscription_sender: mpsc::UnboundedSender<SubscriptionMessage>,
    outgoing_sender: Option<MessageSender<RelayMessage<'static>>>,
    local_subscription_count: Arc<AtomicUsize>,
    task_token: CancellationToken,
    replaceable_event_queue: mpsc::UnboundedSender<(UnsignedEvent, Scope)>,
}

impl SubscriptionManager {
    pub async fn new(
        database: Arc<RelayDatabase>,
        outgoing_sender: MessageSender<RelayMessage<'static>>,
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
        )?;

        let manager = Self {
            database,
            subscription_sender,
            outgoing_sender: Some(outgoing_sender),
            local_subscription_count,
            task_token,
            replaceable_event_queue,
        };

        manager.start_database_subscription_task()?;
        info!("Connection created successfully");
        Ok(manager)
    }

    fn start_subscription_task(
        mut outgoing_sender: MessageSender<RelayMessage<'static>>,
        local_subscription_count: Arc<AtomicUsize>,
        task_token: CancellationToken,
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
                                    crate::metrics::active_subscriptions().increment(1.0);
                                    debug!("Subscription {} added", subscription_id);
                                }
                                Some(SubscriptionMessage::Remove(subscription_id)) => {
                                    if subscriptions.remove(&subscription_id).is_some() {
                                        local_subscription_count.fetch_sub(1, Ordering::SeqCst);
                                        crate::metrics::active_subscriptions().decrement(1.0);
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
                info!("Subscription manager stopped");
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

    /// Returns the current number of active subscriptions.
    /// Uses SeqCst ordering for maximum reliability.
    pub fn subscription_count(&self) -> usize {
        self.local_subscription_count.load(Ordering::SeqCst)
    }

    pub fn add_subscription(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
    ) -> Result<(), Error> {
        self.subscription_sender
            .send(SubscriptionMessage::Add(subscription_id, filters))
            .map_err(|e| Error::Internal {
                message: format!("Failed to send subscription: {}", e),
                backtrace: Backtrace::capture(),
            })
    }

    pub fn remove_subscription(&self, subscription_id: SubscriptionId) -> Result<(), Error> {
        self.subscription_sender
            .send(SubscriptionMessage::Remove(subscription_id))
            .map_err(|e| Error::Internal {
                message: format!("Failed to send unsubscribe: {}", e),
                backtrace: Backtrace::capture(),
            })
    }

    pub async fn save_and_broadcast(&self, store_command: StoreCommand) -> Result<(), Error> {
        match store_command {
            StoreCommand::SaveUnsignedEvent(event, scope)
                if event.kind.is_replaceable() || event.kind.is_addressable() =>
            {
                // Send replaceable/addressable unsigned events to the buffer instead of saving directly
                debug!(
                    "Sending replaceable/addressable unsigned event to buffer: kind={}, scope={:?}",
                    event.kind, scope
                );
                if let Err(e) = self.replaceable_event_queue.send((event, scope)) {
                    error!("Failed to send replaceable event to buffer: {:?}", e);
                    return Err(Error::Internal {
                        message: format!("Failed to send replaceable event to buffer: {}", e),
                        backtrace: Backtrace::capture(),
                    });
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

    /// Fetches historical events from the database without sending them.
    /// The middleware will handle filtering and sending to clients.
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

    // Should be idempotent
    pub fn cleanup(&self) {
        self.cancel_subscription_task();

        // Swap the count to 0 and get the previous value
        let remaining_subs = self.local_subscription_count.swap(0, Ordering::SeqCst);

        if remaining_subs > 0 {
            crate::metrics::active_subscriptions().decrement(remaining_subs as f64);
        }
        info!(
            "Cleaned up connection with {} remaining subscriptions",
            remaining_subs
        );
    }

    /// Waits for the subscription count to reach the expected value with a timeout.
    /// This is useful for tests to ensure that subscription messages have been processed.
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

impl Drop for SubscriptionManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum StoreCommand {
    SaveUnsignedEvent(UnsignedEvent, Scope),
    SaveSignedEvent(Box<Event>, Scope),
    DeleteEvents(Filter, Scope),
}

impl StoreCommand {
    pub fn is_replaceable(&self) -> bool {
        match self {
            StoreCommand::SaveUnsignedEvent(event, _) => event.kind.is_replaceable(),
            StoreCommand::SaveSignedEvent(event, _) => event.kind.is_replaceable(),
            StoreCommand::DeleteEvents(_, _) => false,
        }
    }

    pub fn subdomain_scope(&self) -> &Scope {
        match self {
            StoreCommand::SaveUnsignedEvent(_, scope) => scope,
            StoreCommand::SaveSignedEvent(_, scope) => scope,
            StoreCommand::DeleteEvents(_, scope) => scope,
        }
    }

    /// Convert the Scope to an Option<&str> for backward compatibility with code that
    /// expects Option<&str> representing a subdomain.
    /// This is NOT used for database operations, only for logging and compatibility.
    pub fn subdomain(&self) -> Option<&str> {
        match self.subdomain_scope() {
            Scope::Named { name, .. } => Some(name),
            Scope::Default => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::setup_test;
    use std::time::Instant;
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use websocket_builder::MessageSender;

    // These tests are disabled because they test the old behavior where
    // the subscription manager sends events directly. With the new architecture,
    // the middleware handles filtering and sending events.

    #[ignore]
    #[tokio::test]
    async fn test_subscription_receives_historical_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a historical event
        let keys = Keys::generate();
        let historical_event = EventBuilder::text_note("Historical event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let historical_event = keys.sign_event(historical_event).await.unwrap();
        database
            .save_signed_event(historical_event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database, MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(5);

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // Verify we receive the historical event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(*event, historical_event, "Event content mismatch");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Wait for subscription count to be updated and verify
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_subscription_receives_new_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(5);

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // Verify we receive EOSE since there are no historical events
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Create and save a new event
        let keys = Keys::generate();
        let new_event = EventBuilder::text_note("Hello, world!")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let new_event = Box::new(keys.sign_event(new_event).await.unwrap());
        let new_event_clone = new_event.clone();

        // Save and broadcast the event
        connection
            .save_and_broadcast(StoreCommand::SaveSignedEvent(new_event, Scope::Default))
            .await
            .unwrap();

        // Verify we receive the new event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(*event, *new_event_clone, "Event content mismatch");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify subscription count
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_subscription_receives_both_historical_and_new_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a historical event
        let keys = Keys::generate();
        let historical_event = EventBuilder::text_note("Historical event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let historical_event = keys.sign_event(historical_event).await.unwrap();
        database
            .save_signed_event(historical_event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(5);

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // Verify we receive the historical event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(
                    *event, historical_event,
                    "Historical event content mismatch"
                );
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Create and save a new event
        let keys = Keys::generate();
        let new_event =
            EventBuilder::text_note("New event").build_with_ctx(&Instant::now(), keys.public_key());
        let new_event = Box::new(keys.sign_event(new_event).await.unwrap());
        let new_event_clone = new_event.clone();

        // Save and broadcast the event
        connection
            .save_and_broadcast(StoreCommand::SaveSignedEvent(new_event, Scope::Default))
            .await
            .unwrap();

        // Verify we receive the new event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(*event, *new_event_clone, "New event content mismatch");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify subscription count
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_limit_filter_returns_events_in_reverse_chronological_order() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create events with different timestamps
        let keys = Keys::generate();
        let mut events = vec![];

        // Create events with increasing timestamps
        let base_time = Timestamp::now();
        for i in 0..5 {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .custom_created_at(base_time + i as u64) // Each event 1 second apart
                .build(keys.public_key());
            let event = keys.sign_event(event).await.unwrap();
            database
                .save_signed_event(event.clone(), Scope::Default)
                .await
                .unwrap();
            events.push(event);
        }

        // Wait for events to be saved - increase wait time
        sleep(std::time::Duration::from_millis(100)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription with limit filter
        let subscription_id = SubscriptionId::new("limited_events");
        let filter = Filter::new().limit(3); // Only get last 3 events

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // Collect all received messages and sort them later
        let mut received_messages = vec![];
        for _ in 0..4 {
            // Receive all 4 expected messages (3 events + 1 EOSE)
            if let Some(message) = rx.recv().await {
                received_messages.push(message);
            }
        }

        // Sort and separate messages
        let mut received_events = vec![];
        let mut received_eose = None;

        for message in received_messages {
            match message {
                (
                    RelayMessage::Event {
                        event,
                        subscription_id: sub_id,
                    },
                    _idx,
                ) => {
                    assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                    received_events.push(event.clone());
                }
                (RelayMessage::EndOfStoredEvents(sub_id), _idx) => {
                    assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
                    received_eose = Some(sub_id);
                }
                other => panic!("Unexpected message: {:?}", other),
            }
        }

        // Verify we received both events and EOSE
        assert_eq!(received_events.len(), 3, "Should receive exactly 3 events");
        assert!(received_eose.is_some(), "Should receive EOSE message");

        // Verify events are in reverse chronological order and have different timestamps
        for i in 0..received_events.len() - 1 {
            assert!(
                received_events[i].created_at > received_events[i + 1].created_at,
                "Events not in reverse chronological order or have same timestamp: {} <= {}",
                received_events[i].created_at,
                received_events[i + 1].created_at
            );
        }

        // Verify we got the most recent events (last 3 from our 5 events)
        assert_eq!(received_events.len(), 3);
        // Verify we got events 4, 3, and 2 in that order
        for (i, event) in received_events.iter().enumerate() {
            assert_eq!(
                event.content,
                format!("Event {}", 4 - i),
                "Wrong event content at position {}",
                i
            );
        }

        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        connection.cleanup();

        assert_eq!(connection.subscription_count(), 0);
    }

    #[ignore]
    #[tokio::test]
    async fn test_empty_filter_returns_text_note_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a text note event
        let keys = Keys::generate();
        let text_note = EventBuilder::text_note("Text note event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let text_note = keys.sign_event(text_note).await.unwrap();
        database
            .save_signed_event(text_note.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("text_note_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // We should receive the text note event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(event.kind, Kind::TextNote, "Event was not a text note");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Wait for subscription count to be updated and verify
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_empty_filter_returns_metadata_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a metadata event
        let keys = Keys::generate();
        let mut metadata = Metadata::new();
        metadata.name = Some("Test User".to_string());
        let metadata_event =
            EventBuilder::metadata(&metadata).build_with_ctx(&Instant::now(), keys.public_key());
        let metadata_event = keys.sign_event(metadata_event).await.unwrap();
        database
            .save_signed_event(metadata_event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("metadata_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // We should receive the metadata event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(event.kind, Kind::Metadata, "Event was not a metadata event");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Wait for subscription count to be updated and verify
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_empty_filter_returns_contact_list_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a contact list event
        let keys = Keys::generate();
        let contacts_event = EventBuilder::new(Kind::ContactList, "[]")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let contacts_event = keys.sign_event(contacts_event).await.unwrap();
        database
            .save_signed_event(contacts_event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("contact_list_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // We should receive the contacts event
        match rx.recv().await {
            Some((
                RelayMessage::Event {
                    event,
                    subscription_id: sub_id,
                },
                _idx,
            )) => {
                assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(
                    event.kind,
                    Kind::ContactList,
                    "Event was not a contact list event"
                );
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Wait for subscription count to be updated and verify
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_empty_filter_returns_events_from_multiple_authors() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create events with different authors
        let keys1 = Keys::generate();
        let keys2 = Keys::generate();

        let event1 = EventBuilder::text_note("Event from author 1")
            .build_with_ctx(&Instant::now(), keys1.public_key());
        let event1 = keys1.sign_event(event1).await.unwrap();

        let event2 = EventBuilder::text_note("Event from author 2")
            .build_with_ctx(&Instant::now(), keys2.public_key());
        let event2 = keys2.sign_event(event2).await.unwrap();

        // Save events
        database
            .save_signed_event(event1.clone(), Scope::Default)
            .await
            .unwrap();
        database
            .save_signed_event(event2.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("multi_author_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // We should receive events from both authors
        let mut received_events = Vec::new();
        for _ in 0..2 {
            match rx.recv().await {
                Some((
                    RelayMessage::Event {
                        event,
                        subscription_id: sub_id,
                    },
                    _idx,
                )) => {
                    assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                    received_events.push(event.clone());
                }
                other => panic!("Expected Event message, got: {:?}", other),
            }
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Verify we got events from both authors
        assert!(
            received_events
                .iter()
                .any(|e| e.pubkey == keys1.public_key()),
            "No events from author 1"
        );
        assert!(
            received_events
                .iter()
                .any(|e| e.pubkey == keys2.public_key()),
            "No events from author 2"
        );

        // Wait for subscription count to be updated and verify
        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[ignore]
    #[tokio::test]
    async fn test_empty_filter_returns_all_event_kinds() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save events of different kinds
        let keys = Keys::generate();
        let text_note =
            EventBuilder::text_note("Text note").build_with_ctx(&Instant::now(), keys.public_key());
        let text_note = keys.sign_event(text_note).await.unwrap();

        let mut metadata = Metadata::new();
        metadata.name = Some("Test User".to_string());
        metadata.about = Some("about me".to_string());
        metadata.picture = Some("https://example.com/pic.jpg".to_string());
        let metadata_event =
            EventBuilder::metadata(&metadata).build_with_ctx(&Instant::now(), keys.public_key());
        let metadata_event = keys.sign_event(metadata_event).await.unwrap();

        let recommend_relay = EventBuilder::new(Kind::RelayList, "wss://relay.example.com")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let recommend_relay = keys.sign_event(recommend_relay).await.unwrap();

        // Save all events
        database
            .save_signed_event(text_note.clone(), Scope::Default)
            .await
            .unwrap();
        database
            .save_signed_event(metadata_event.clone(), Scope::Default)
            .await
            .unwrap();
        database
            .save_signed_event(recommend_relay.clone(), Scope::Default)
            .await
            .unwrap();

        // Wait longer for events to be saved
        sleep(std::time::Duration::from_millis(100)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("all_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .add_subscription(subscription_id.clone(), vec![filter])
            .unwrap();

        // Collect all received messages
        let mut received_messages = vec![];
        for _ in 0..4 {
            // Receive all 4 expected messages (3 events + 1 EOSE)
            if let Some(message) = rx.recv().await {
                received_messages.push(message);
            }
        }

        // Sort and process messages
        let mut received_kinds = vec![];
        let mut received_eose = false;

        for message in received_messages {
            match message {
                (
                    RelayMessage::Event {
                        event,
                        subscription_id: sub_id,
                    },
                    _idx,
                ) => {
                    assert_eq!(*sub_id, subscription_id, "Subscription ID mismatch");
                    received_kinds.push(event.kind);
                }
                (RelayMessage::EndOfStoredEvents(sub_id), _idx) => {
                    assert_eq!(*sub_id, subscription_id, "EOSE subscription ID mismatch");
                    received_eose = true;
                }
                other => panic!("Unexpected message: {:?}", other),
            }
        }

        // Verify we received both events and EOSE
        assert_eq!(received_kinds.len(), 3, "Should receive exactly 3 events");
        assert!(received_eose, "Should receive EOSE message");

        assert!(received_kinds.contains(&Kind::TextNote));
        assert!(received_kinds.contains(&Kind::Metadata));
        assert!(received_kinds.contains(&Kind::RelayList));

        assert!(
            connection.wait_for_subscription_count(1, 1000).await,
            "Subscription count did not reach expected value"
        );
        assert_eq!(connection.subscription_count(), 1);

        connection.cleanup();
    }
}
