use crate::error::Error;
use crate::nostr_database::RelayDatabase;
use anyhow::Result;
use nostr_sdk::prelude::*;
use snafu::Backtrace;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};
use websocket_builder::MessageSender;

// Message types for subscription management
#[derive(Debug)]
enum SubscriptionMessage {
    Add(SubscriptionId, Vec<Filter>),
    Remove(SubscriptionId),
    CheckEvent {
        event: Event,
        sender: MessageSender<RelayMessage>,
    },
}

#[derive(Debug, Clone)]
pub struct EventStoreConnection {
    id: String,
    database: Arc<RelayDatabase>,
    db_connection: String,
    connection_token: CancellationToken,
    subscription_sender: mpsc::UnboundedSender<SubscriptionMessage>,
    pub outgoing_sender: Option<MessageSender<RelayMessage>>,
    local_subscription_count: Arc<AtomicUsize>,
}

impl EventStoreConnection {
    pub async fn new(
        id: String,
        database: Arc<RelayDatabase>,
        db_connection: String,
        cancellation_token: CancellationToken,
        outgoing_sender: MessageSender<RelayMessage>,
    ) -> Result<Self> {
        let id_clone = id.clone();
        debug!(
            target: "event_store",
            "[{}] Creating new connection for {}",
            id_clone,
            db_connection
        );

        let (subscription_sender, subscription_receiver) = mpsc::unbounded_channel();
        let local_subscription_count = Arc::new(AtomicUsize::new(0));

        let connection = Self {
            id: id_clone.clone(),
            database: database.clone(),
            db_connection,
            connection_token: cancellation_token.child_token(),
            subscription_sender,
            outgoing_sender: Some(outgoing_sender.clone()),
            local_subscription_count: local_subscription_count.clone(),
        };

        // Spawn subscription management task
        let token = connection.connection_token.clone();
        let id_clone2 = id_clone.clone();
        tokio::spawn(Box::pin(async move {
            let mut subscriptions: HashMap<SubscriptionId, Vec<Filter>> = HashMap::new();
            let mut subscription_receiver = subscription_receiver;

            debug!(
                target: "event_store",
                "[{}] Starting subscription manager",
                id_clone2
            );

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!(
                            target: "event_store",
                            "[{}] Subscription manager shutting down",
                            id_clone2
                        );
                        break;
                    }
                    Some(msg) = subscription_receiver.recv() => {
                        match msg {
                            SubscriptionMessage::Add(subscription_id, filters) => {
                                debug!(
                                    target: "event_store",
                                    "[{}] Adding subscription {} (current count: {})",
                                    id_clone2, subscription_id, subscriptions.len()
                                );
                                subscriptions.insert(subscription_id, filters);
                            }
                            SubscriptionMessage::Remove(subscription_id) => {
                                if subscriptions.remove(&subscription_id).is_some() {
                                    debug!(
                                        target: "event_store",
                                        "[{}] Removed subscription {} (remaining count: {})",
                                        id_clone2,
                                        subscription_id,
                                        subscriptions.len()
                                    );
                                    local_subscription_count.fetch_sub(1, Ordering::Relaxed);
                                    crate::metrics::active_subscriptions().decrement(1.0);
                                }
                            }
                            SubscriptionMessage::CheckEvent { event, sender } => {
                                let mut sender = sender;
                                for (subscription_id, filters) in subscriptions.iter() {
                                    let matches = filters.iter().any(|filter| filter.match_event(&event));
                                    if matches {
                                        debug!(
                                            target: "event_store",
                                            "[{}] Matched event {} to subscription {}",
                                            id_clone2,
                                            event.id,
                                            subscription_id
                                        );
                                        let message = RelayMessage::Event {
                                            event: Box::new(event.clone()),
                                            subscription_id: subscription_id.clone(),
                                        };

                                        if let Err(e) = sender.send(message).await {
                                            error!(
                                                target: "event_store",
                                                "[{}] Failed to send event {} to subscription {}: {:?}",
                                                id_clone2,
                                                event.id,
                                                subscription_id,
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }));

        // Spawn broadcast handler
        let token_clone = connection.connection_token.clone();
        let subscription_sender_clone = connection.subscription_sender.clone();
        let id_clone3 = id_clone.clone();
        let outgoing_sender = outgoing_sender.clone();
        let mut broadcast_receiver = database.subscribe();
        tokio::spawn(Box::pin(async move {
            debug!(
                target: "event_store",
                "[{}] Starting broadcast event handler",
                id_clone3
            );

            loop {
                tokio::select! {
                    _ = token_clone.cancelled() => {
                        debug!(
                            target: "event_store",
                            "[{}] Broadcast event handler shutting down",
                            id_clone3
                        );
                        break;
                    }
                    Ok(event) = broadcast_receiver.recv() => {
                        if let Err(e) = subscription_sender_clone.send(SubscriptionMessage::CheckEvent {
                            event,
                            sender: outgoing_sender.clone(),
                        }) {
                            error!(
                                target: "event_store",
                                "[{}] Failed to send event to subscription manager: {:?}",
                                id_clone3,
                                e
                            );
                        }
                    }
                }
            }
        }));

        debug!(
            target: "event_store",
            "[{}] Connection created successfully",
            id_clone
        );

        Ok(connection)
    }

    /// Returns the capacity of the outgoing sender
    pub fn sender_capacity(&self) -> usize {
        let Some(sender) = &self.outgoing_sender else {
            return 0;
        };
        sender.capacity()
    }

    pub fn set_outgoing_sender(&mut self, sender: MessageSender<RelayMessage>) {
        self.outgoing_sender = Some(sender);
    }

    pub fn add_subscription(&self, subscription_id: SubscriptionId, filters: Vec<Filter>) {
        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::Add(subscription_id, filters))
        {
            error!(
                target: "event_store",
                "[{}] Failed to send add subscription message: {:?}",
                self.id,
                e
            );
        }
    }

    pub fn remove_subscription(&self, subscription_id: &SubscriptionId) {
        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::Remove(subscription_id.clone()))
        {
            error!(
                target: "event_store",
                "[{}] Failed to send remove subscription message: {:?}",
                self.id,
                e
            );
        }
    }

    pub async fn handle_broadcast_event(&self, event: &Event) -> Result<(), Error> {
        let Some(sender) = &self.outgoing_sender else {
            error!(
                target: "event_store",
                "[{}] No outgoing sender available for connection", self.id
            );
            return Ok(());
        };

        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::CheckEvent {
                event: event.clone(),
                sender: sender.clone(),
            })
        {
            error!(
                target: "event_store",
                "[{}] Failed to send check event message: {:?}",
                self.id,
                e
            );
        }
        Ok(())
    }

    pub async fn save_event(&self, store_command: StoreCommand) -> Result<(), Error> {
        match store_command {
            // These events are signed by the relay key
            StoreCommand::SaveUnsignedEvent(event) => {
                match self.database.save_unsigned_event(event).await {
                    Ok(event) => {
                        info!(
                            target: "event_store",
                            "[{}] Saved unsigned event: kind={}",
                            self.id,
                            event.kind
                        );
                    }
                    Err(e) => {
                        error!(
                            target: "event_store",
                            "[{}] Error saving unsigned event: {:?}",
                            self.id,
                            e
                        );
                        return Err(Error::Internal {
                            message: format!("Error saving unsigned event: {:?}", e),
                            backtrace: Backtrace::capture(),
                        });
                    }
                }
            }
            StoreCommand::SaveSignedEvent(event) => {
                if let Err(e) = self.database.save_signed_event(&event).await {
                    return Err(Error::Internal {
                        message: format!("Error saving signed event: {:?}", e),
                        backtrace: Backtrace::capture(),
                    });
                }
                info!(
                    target: "event_store",
                    "[{}] Saved signed event: kind={} {}",
                    self.id,
                    event.kind,
                    event.id
                );
            }
            StoreCommand::DeleteEvents(filter) => {
                let filter_string = format!("{:?}", filter);
                if let Err(e) = self.database.delete(filter).await {
                    return Err(Error::Internal {
                        message: format!("Error deleting events: {:?}", e),
                        backtrace: Backtrace::capture(),
                    });
                }
                info!(
                    target: "event_store",
                    "[{}] Deleted events: {}",
                    self.id,
                    filter_string
                );
            }
        }
        Ok(())
    }

    pub async fn fetch_events(&self, filters: Vec<Filter>) -> Result<Events, Error> {
        match self.database.query(filters).await {
            Ok(events) => Ok(events),
            Err(e) => Err(Error::notice(format!("Failed to fetch events: {:?}", e))),
        }
    }

    pub async fn save_and_broadcast(&self, event: Event) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Handling event {} from {}",
            self.id,
            event.id,
            self.db_connection
        );

        // First save the event to the database
        if let Err(e) = self.database.save_signed_event(&event).await {
            error!(
                target: "event_store",
                "[{}] Failed to save event {}: {:?}",
                self.id,
                event.id,
                e
            );
            return Err(Error::notice(format!("Failed to save event: {:?}", e)));
        }

        // Then check subscriptions
        if let Some(sender) = &self.outgoing_sender {
            if let Err(e) = self
                .subscription_sender
                .send(SubscriptionMessage::CheckEvent {
                    event: event.clone(),
                    sender: sender.clone(),
                })
            {
                error!(
                    target: "event_store",
                    "[{}] Failed to send event {} to subscription manager: {:?}",
                    self.id,
                    event.id,
                    e
                );
            }
        }

        Ok(())
    }

    pub async fn handle_subscription(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
    ) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Handling subscription {} from {}",
            self.id,
            subscription_id,
            self.db_connection
        );

        // Increment subscription count before sending message
        self.local_subscription_count
            .fetch_add(1, Ordering::Relaxed);
        crate::metrics::active_subscriptions().increment(1.0);

        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::Add(subscription_id, filters))
        {
            // Decrement count if we failed to send
            self.local_subscription_count
                .fetch_sub(1, Ordering::Relaxed);
            crate::metrics::active_subscriptions().decrement(1.0);

            error!(
                target: "event_store",
                "[{}] Failed to send subscription to manager: {:?}",
                self.id,
                e
            );
            return Err(Error::Internal {
                message: format!("Failed to send subscription to manager: {}", e),
                backtrace: Backtrace::capture(),
            });
        }

        Ok(())
    }

    pub async fn handle_unsubscribe(&self, subscription_id: SubscriptionId) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Handling unsubscribe {} from {}",
            self.id,
            subscription_id,
            self.db_connection
        );

        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::Remove(subscription_id))
        {
            error!(
                target: "event_store",
                "[{}] Failed to send unsubscribe to manager: {:?}",
                self.id,
                e
            );
            return Err(Error::Internal {
                message: format!("Failed to send unsubscribe to manager: {}", e),
                backtrace: Backtrace::capture(),
            });
        }

        Ok(())
    }

    /// Returns the current number of active subscriptions for this connection
    pub fn get_local_subscription_count(&self) -> usize {
        self.local_subscription_count.load(Ordering::Relaxed)
    }

    /// Fetches historical events for a subscription and sends them through the provided sender
    /// Returns the number of events sent
    pub async fn fetch_historical_events(
        &self,
        subscription_id: &SubscriptionId,
        filters: &[Filter],
        mut sender: MessageSender<RelayMessage>,
    ) -> Result<usize, Error> {
        let events = self.fetch_events(filters.to_vec()).await?;

        // Send each event
        let len = events.len();
        let capacity = self.sender_capacity() / 2;

        for event in events.into_iter().take(capacity) {
            if let Err(e) = sender
                .send(RelayMessage::Event {
                    subscription_id: subscription_id.clone(),
                    event: Box::new(event),
                })
                .await
            {
                error!(
                    target: "event_store",
                    "[{}] Failed to send historical event to subscription {}: {:?}",
                    self.id,
                    subscription_id,
                    e
                );
            }
        }

        debug!(
            target: "event_store",
            "[{}] Sending EOSE for subscription {} after {} historical events",
            self.id,
            subscription_id,
            len
        );

        // Send EOSE
        if let Err(e) = sender
            .send(RelayMessage::EndOfStoredEvents(subscription_id.clone()))
            .await
        {
            error!(
                target: "event_store",
                "[{}] Failed to send EOSE to subscription {}: {:?}",
                self.id,
                subscription_id,
                e
            );
        }

        Ok(len)
    }

    /// Handles a complete subscription request by:
    /// 1. Adding the subscription
    /// 2. Fetching and sending historical events
    /// 3. Sending EOSE
    pub async fn handle_subscription_request(
        &self,
        subscription_id: SubscriptionId,
        filters: Vec<Filter>,
    ) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Processing subscription request for {}",
            self.id,
            subscription_id
        );

        let Some(sender) = &self.outgoing_sender else {
            error!(
                target: "event_store",
                "[{}] No outgoing sender available for subscription {}",
                self.id,
                subscription_id
            );
            return Err(Error::Internal {
                message: "No outgoing sender available".to_string(),
                backtrace: Backtrace::capture(),
            });
        };

        // First add the subscription
        if let Err(e) = self
            .handle_subscription(subscription_id.clone(), filters.clone())
            .await
        {
            error!(
                target: "event_store",
                "[{}] Failed to add subscription {}: {}",
                self.id,
                subscription_id,
                e
            );
            return Err(e);
        }

        debug!(
            target: "event_store",
            "[{}] Successfully added subscription {}",
            self.id,
            subscription_id
        );

        // Then fetch and send historical events
        if let Err(e) = self
            .fetch_historical_events(&subscription_id, &filters, sender.clone())
            .await
        {
            error!(
                target: "event_store",
                "[{}] Failed to fetch historical events for subscription {}: {}",
                self.id,
                subscription_id,
                e
            );
            // Clean up subscription since we failed
            self.remove_subscription(&subscription_id);
            return Err(e);
        }

        debug!(
            target: "event_store",
            "[{}] Successfully completed subscription request for {}",
            self.id,
            subscription_id
        );

        Ok(())
    }

    /// Cleans up resources and metrics when the connection is closed
    pub fn cleanup(&self) {
        let remaining_subs = self.get_local_subscription_count();
        if remaining_subs > 0 {
            debug!(
                target: "event_store",
                "[{}] Cleaning up {} remaining subscriptions from metrics",
                self.id,
                remaining_subs
            );
            // Decrement the global metrics by the number of subscriptions that weren't explicitly closed
            crate::metrics::active_subscriptions().decrement(remaining_subs as f64);
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum StoreCommand {
    SaveUnsignedEvent(UnsignedEvent),
    SaveSignedEvent(Event),
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
    use websocket_builder::MessageSender;

    #[tokio::test]
    async fn test_subscription_receives_historical_events() {
        let (_tmp_dir, database, _admin_keys) = setup_test().await;

        // Create and save a historical event
        let keys = Keys::generate();
        let historical_event = EventBuilder::text_note("Historical event")
            .build_with_ctx(&Instant::now(), keys.public_key());
        let historical_event = keys.sign_event(historical_event).await.unwrap();
        database.save_signed_event(&historical_event).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            CancellationToken::new(),
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
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database.clone(),
            "test_db".to_string(),
            CancellationToken::new(),
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
        let new_event = keys.sign_event(new_event).await.unwrap();

        // Save and broadcast the event
        connection
            .save_and_broadcast(new_event.clone())
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
                assert_eq!(*event, new_event, "Event content mismatch");
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
        database.save_signed_event(&historical_event).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database.clone(),
            "test_db".to_string(),
            CancellationToken::new(),
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
        let new_event = keys.sign_event(new_event).await.unwrap();

        // Save and broadcast the event
        connection
            .save_and_broadcast(new_event.clone())
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
                assert_eq!(*event, new_event, "New event content mismatch");
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
            database.save_signed_event(&event).await.unwrap();
            events.push(event);
        }

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database.clone(),
            "test_db".to_string(),
            CancellationToken::new(),
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
        database.save_signed_event(&text_note).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            CancellationToken::new(),
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
        database.save_signed_event(&metadata_event).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            CancellationToken::new(),
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
        database.save_signed_event(&contacts_event).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            CancellationToken::new(),
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
        database.save_signed_event(&event1).await.unwrap();
        database.save_signed_event(&event2).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            CancellationToken::new(),
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
        database.save_signed_event(&text_note).await.unwrap();
        database.save_signed_event(&metadata_event).await.unwrap();
        database.save_signed_event(&recommend_relay).await.unwrap();

        // Create a connection with a channel to receive messages
        let (tx, mut rx) = mpsc::channel::<(RelayMessage, usize)>(10);
        let connection = EventStoreConnection::new(
            "test_conn".to_string(),
            database,
            "test_db".to_string(),
            CancellationToken::new(),
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

        // We should receive all events (order may vary)
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

        // Verify we receive EOSE
        match rx.recv().await {
            Some((RelayMessage::EndOfStoredEvents(sub_id), _idx)) => {
                assert_eq!(sub_id, subscription_id, "EOSE subscription ID mismatch");
            }
            other => panic!("Expected EOSE message, got: {:?}", other),
        }

        // Verify we received all kinds
        assert!(received_kinds.contains(&Kind::TextNote));
        assert!(received_kinds.contains(&Kind::Metadata));
        assert!(received_kinds.contains(&Kind::RelayList));

        // Verify subscription count
        assert_eq!(connection.get_local_subscription_count(), 1);

        // Clean up
        connection.cleanup();
    }
}
