use crate::error::Error;
use crate::nostr_database::RelayDatabase;
use nostr_sdk::prelude::*;
use snafu::Backtrace;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};
use websocket_builder::MessageSender;

#[derive(Debug)]
enum SubscriptionMessage {
    Add(SubscriptionId, Vec<Filter>),
    Remove(SubscriptionId),
    CheckEvent { event: Box<Event> },
}

#[derive(Debug, Clone)]
pub struct SubscriptionManager {
    id: String,
    database: Arc<RelayDatabase>,
    db_connection: String,
    subscription_sender: mpsc::UnboundedSender<SubscriptionMessage>,
    outgoing_sender: Option<MessageSender<RelayMessage>>,
    local_subscription_count: Arc<AtomicUsize>,
}

impl SubscriptionManager {
    pub async fn new(
        id: String,
        database: Arc<RelayDatabase>,
        db_connection: String,
        outgoing_sender: MessageSender<RelayMessage>,
    ) -> Result<Self, Error> {
        let local_subscription_count = Arc::new(AtomicUsize::new(0));
        let subscription_sender = Self::start_subscription_task(
            &id,
            outgoing_sender.clone(),
            local_subscription_count.clone(),
        )?;

        let manager = Self {
            id,
            database,
            db_connection,
            subscription_sender,
            outgoing_sender: Some(outgoing_sender),
            local_subscription_count,
        };

        manager.start_database_subscription_task()?;
        info!(target: "event_store", "[{}] Connection created successfully", manager.id);
        Ok(manager)
    }

    fn start_subscription_task(
        id: &str,
        mut outgoing_sender: MessageSender<RelayMessage>,
        local_subscription_count: Arc<AtomicUsize>,
    ) -> Result<mpsc::UnboundedSender<SubscriptionMessage>, Error> {
        let (subscription_sender, mut subscription_receiver) = mpsc::unbounded_channel();
        let id = id.to_string();

        tokio::spawn(Box::pin(async move {
            let mut subscriptions = HashMap::new();
            info!(target: "event_store", "[{}] Starting subscription manager", id);

            while let Some(msg) = subscription_receiver.recv().await {
                match msg {
                    SubscriptionMessage::Add(subscription_id, filters) => {
                        subscriptions.insert(subscription_id.clone(), filters);
                        local_subscription_count.fetch_add(1, Ordering::Relaxed);
                        crate::metrics::active_subscriptions().increment(1.0);
                    }
                    SubscriptionMessage::Remove(subscription_id) => {
                        if subscriptions.remove(&subscription_id).is_some() {
                            local_subscription_count.fetch_sub(1, Ordering::Relaxed);
                            crate::metrics::active_subscriptions().decrement(1.0);
                        }
                    }
                    SubscriptionMessage::CheckEvent { event } => {
                        for (subscription_id, filters) in &subscriptions {
                            if filters
                                .iter()
                                .any(|filter| filter.match_event(event.as_ref()))
                            {
                                let message = RelayMessage::Event {
                                    event: event.clone(),
                                    subscription_id: subscription_id.clone(),
                                };
                                if let Err(e) = outgoing_sender.send(message).await {
                                    error!(target: "event_store", "[{}] Failed to send event: {:?}", id, e);
                                }
                            }
                        }
                    }
                }
            }
            info!(target: "event_store", "[{}] Subscription manager stopped", id);
        }));

        Ok(subscription_sender)
    }

    fn start_database_subscription_task(&self) -> Result<(), Error> {
        let mut database_subscription = self.database.subscribe();
        let subscription_sender = self.subscription_sender.clone();

        tokio::spawn(async move {
            while let Ok(event) = database_subscription.recv().await {
                if let Err(e) = subscription_sender.send(SubscriptionMessage::CheckEvent { event })
                {
                    error!(target: "event_store", "Failed to send event: {:?}", e);
                    break;
                }
            }
            debug!(target: "event_store", "Database subscription task stopped");
        });

        Ok(())
    }

    pub fn sender_capacity(&self) -> usize {
        self.outgoing_sender
            .as_ref()
            .map_or(0, |sender| sender.capacity())
    }

    pub fn set_outgoing_sender(&mut self, sender: MessageSender<RelayMessage>) {
        self.outgoing_sender = Some(sender);
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
        self.database.save_store_command(store_command).await
    }

    pub async fn fetch_events(&self, filters: Vec<Filter>) -> Result<Events, Error> {
        self.database
            .query(filters)
            .await
            .map_err(|e| Error::notice(format!("Failed to fetch events: {:?}", e)))
    }

    pub async fn handle_unsubscribe(&self, subscription_id: SubscriptionId) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Handling unsubscribe {} from {}",
            self.id, subscription_id, self.db_connection
        );
        self.remove_subscription(subscription_id)
    }

    fn get_local_subscription_count(&self) -> usize {
        self.local_subscription_count.load(Ordering::Relaxed)
    }

    pub async fn fetch_historical_events(
        &self,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<usize, Error> {
        let events = self.fetch_events(filters.to_vec()).await?;
        let capacity = sender.capacity() / 2;
        let events_len = events.len();

        for event in events.into_iter().take(capacity) {
            if let Err(e) = sender
                .send(RelayMessage::Event {
                    subscription_id: subscription_id.clone(),
                    event: Box::new(event),
                })
                .await
            {
                return Err(Error::Internal {
                    message: format!("Failed to send event: {}", e),
                    backtrace: Backtrace::capture(),
                });
            }
        }

        if let Err(e) = sender
            .send(RelayMessage::EndOfStoredEvents(subscription_id.clone()))
            .await
        {
            return Err(Error::Internal {
                message: format!("Failed to send EOSE: {}", e),
                backtrace: Backtrace::capture(),
            });
        }

        Ok(events_len)
    }

    pub async fn handle_subscription_request(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
    ) -> Result<(), Error> {
        let sender = self
            .outgoing_sender
            .clone()
            .ok_or_else(|| Error::Internal {
                message: "No outgoing sender available".to_string(),
                backtrace: Backtrace::capture(),
            })?;

        self.add_subscription(subscription_id.clone(), filters.clone())?;
        if let Err(fetch_err) = self
            .fetch_historical_events(&subscription_id, &filters, sender)
            .await
        {
            if let Err(remove_err) = self.remove_subscription(subscription_id.clone()) {
                return Err(Error::Internal {
                    message: format!(
                        "Failed to fetch historical events: {}; rollback failed: {}",
                        fetch_err, remove_err
                    ),
                    backtrace: Backtrace::capture(),
                });
            }
            return Err(fetch_err);
        }
        Ok(())
    }

    pub fn cleanup(&self) {
        let remaining_subs = self.get_local_subscription_count();
        if remaining_subs > 0 {
            crate::metrics::active_subscriptions().decrement(remaining_subs as f64);
        }
        info!(
            target: "event_store",
            "[{}] Cleaned up connection with {} remaining subscriptions",
            self.id, remaining_subs
        );
    }
}

impl Drop for SubscriptionManager {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum StoreCommand {
    SaveUnsignedEvent(UnsignedEvent),
    SaveSignedEvent(Box<Event>),
    DeleteEvents(Filter),
}

impl StoreCommand {
    pub fn is_replaceable(&self) -> bool {
        match self {
            StoreCommand::SaveUnsignedEvent(event) => event.kind.is_replaceable(),
            StoreCommand::SaveSignedEvent(event) => event.kind.is_replaceable(),
            StoreCommand::DeleteEvents(_) => false,
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

    #[tokio::test]
    async fn test_subscription_receives_historical_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a historical event
        let keys = Keys::generate();
        let historical_event = EventBuilder::text_note("Historical event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let historical_event = keys.sign_event(historical_event).await.unwrap();
        database
            .save_signed_event(historical_event.clone())
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(5);

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(*event, historical_event, "Event content mismatch");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Verify subscription count
        assert_eq!(connection.get_local_subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[tokio::test]
    async fn test_subscription_receives_new_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database.clone(),
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(5);

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
            .unwrap();

        // Verify we receive EOSE since there are no historical events
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
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
            .save_and_broadcast(StoreCommand::SaveSignedEvent(new_event))
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(event, new_event_clone, "Event content mismatch");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify subscription count
        assert_eq!(connection.get_local_subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[tokio::test]
    async fn test_subscription_receives_both_historical_and_new_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a historical event
        let keys = Keys::generate();
        let historical_event = EventBuilder::text_note("Historical event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let historical_event = keys.sign_event(historical_event).await.unwrap();
        database
            .save_signed_event(historical_event.clone())
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database.clone(),
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription
        let subscription_id = SubscriptionId::new("test_sub");
        let filter = Filter::new().kinds(vec![Kind::TextNote]).limit(5);

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
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
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
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
            .save_and_broadcast(StoreCommand::SaveSignedEvent(new_event))
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(event, new_event_clone, "New event content mismatch");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify subscription count
        assert_eq!(connection.get_local_subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

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
            database.save_signed_event(event.clone()).await.unwrap();
            events.push(event);
        }

        // Wait for events to be saved
        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database.clone(),
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription with limit filter
        let subscription_id = SubscriptionId::new("limited_events");
        let filter = Filter::new().limit(3); // Only get last 3 events

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
            .unwrap();

        // Collect received events
        let mut received_events = vec![];
        for _ in 0..3 {
            match rx.recv().await {
                Some((
                    RelayMessage::Event {
                        event,
                        subscription_id: sub_id,
                    },
                    _idx,
                )) => {
                    assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                    received_events.push(*event);
                }
                other => panic!("Expected Event message, got: {:?}", other),
            }
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

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

        // Verify subscription count
        assert_eq!(connection.get_local_subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_text_note_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a text note event
        let keys = Keys::generate();
        let text_note = EventBuilder::text_note("Text note event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let text_note = keys.sign_event(text_note).await.unwrap();
        database.save_signed_event(text_note.clone()).await.unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("text_note_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(event.kind, Kind::TextNote, "Event was not a text note");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Clean up
        connection.cleanup();
    }

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
            .save_signed_event(metadata_event.clone())
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("metadata_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                assert_eq!(event.kind, Kind::Metadata, "Event was not a metadata event");
            }
            other => panic!("Expected Event message, got: {:?}", other),
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Clean up
        connection.cleanup();
    }

    #[tokio::test]
    async fn test_empty_filter_returns_contact_list_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a contact list event
        let keys = Keys::generate();
        let contacts_event = EventBuilder::new(Kind::ContactList, "[]")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let contacts_event = keys.sign_event(contacts_event).await.unwrap();
        database
            .save_signed_event(contacts_event.clone())
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("contact_list_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
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
                assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
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
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Clean up
        connection.cleanup();
    }

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
        database.save_signed_event(event1.clone()).await.unwrap();
        database.save_signed_event(event2.clone()).await.unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("multi_author_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
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
                    assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                    received_events.push(*event);
                }
                other => panic!("Expected Event message, got: {:?}", other),
            }
        }

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
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

        // Clean up
        connection.cleanup();
    }

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
        database.save_signed_event(text_note.clone()).await.unwrap();
        database
            .save_signed_event(metadata_event.clone())
            .await
            .unwrap();
        database
            .save_signed_event(recommend_relay.clone())
            .await
            .unwrap();

        sleep(std::time::Duration::from_millis(30)).await;

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = SubscriptionManager::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            MessageSender::new(tx, 0),
        )
        .await
        .unwrap();

        // Set up subscription with empty filter
        let subscription_id = SubscriptionId::new("all_events");
        let filter = Filter::new();

        // Handle subscription request
        connection
            .handle_subscription_request(subscription_id.clone(), vec![filter])
            .await
            .unwrap();

        let mut received_kinds = vec![];
        for _ in 0..3 {
            match rx.recv().await {
                Some((
                    RelayMessage::Event {
                        event,
                        subscription_id: sub_id,
                    },
                    _idx,
                )) => {
                    assert_eq!(sub_id, subscription_id, "Subscription ID mismatch");
                    received_kinds.push(event.kind);
                }
                other => panic!("Expected Event message, got: {:?}", other),
            }
        }

        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        assert!(received_kinds.contains(&Kind::TextNote));
        assert!(received_kinds.contains(&Kind::Metadata));
        assert!(received_kinds.contains(&Kind::RelayList));

        assert_eq!(connection.get_local_subscription_count(), 1);

        connection.cleanup();
    }
}
