use crate::error::Error;
use crate::nostr_database::NostrDatabase;
use anyhow::Result;
use nostr_sdk::prelude::*;
use snafu::Backtrace;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;
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
    GetCount(oneshot::Sender<usize>),
}

// Add after the SubscriptionMessage enum
struct ReplaceableEventsBuffer {
    buffer: HashMap<(PublicKey, Kind), UnsignedEvent>,
    sender: mpsc::UnboundedSender<UnsignedEvent>,
    receiver: Option<mpsc::UnboundedReceiver<UnsignedEvent>>,
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

    pub fn get_sender(&self) -> mpsc::UnboundedSender<UnsignedEvent> {
        self.sender.clone()
    }

    pub fn insert(&mut self, event: UnsignedEvent) {
        self.buffer.insert((event.pubkey, event.kind), event);
    }

    async fn flush(&mut self, database: &Arc<NostrDatabase>) {
        if self.buffer.is_empty() {
            return;
        }

        for (_, event) in self.buffer.drain() {
            match database.save_unsigned_event(event).await {
                Ok(event) => {
                    info!(
                        target: "event_store",
                        "Saved replaceable event: kind={}",
                        event.kind
                    );
                }
                Err(e) => {
                    error!(
                        target: "event_store",
                        "Error saving event: {:?}",
                        e
                    );
                }
            }
        }
    }

    pub fn start(mut self, database: Arc<NostrDatabase>, token: CancellationToken, id: String) {
        let mut receiver = self.receiver.take().expect("Receiver already taken");

        tokio::spawn(Box::pin(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!(
                            target: "event_store",
                            "[{}] Replaceable events handler shutting down",
                            id
                        );
                        self.flush(&database).await;
                        return;
                    }

                    event = receiver.recv() => {
                        if let Some(event) = event {
                            self.insert(event);
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
pub struct EventStoreConnection {
    id: String,
    database: Arc<NostrDatabase>,
    db_connection: String,
    connection_token: CancellationToken,
    replaceable_event_queue: mpsc::UnboundedSender<UnsignedEvent>,
    subscription_sender: mpsc::UnboundedSender<SubscriptionMessage>,
    pub outgoing_sender: Option<MessageSender<RelayMessage>>,
}

impl EventStoreConnection {
    pub async fn new(
        id: String,
        database: Arc<NostrDatabase>,
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

        let buffer = ReplaceableEventsBuffer::new();
        let replaceable_event_queue = buffer.get_sender();
        let (subscription_sender, subscription_receiver) = mpsc::unbounded_channel();

        let connection = Self {
            id: id_clone.clone(),
            database: database.clone(),
            db_connection,
            connection_token: cancellation_token.child_token(),
            replaceable_event_queue,
            subscription_sender,
            outgoing_sender: Some(outgoing_sender.clone()),
        };

        // Start the buffer task
        buffer.start(
            database.clone(),
            connection.connection_token.clone(),
            id_clone.clone(),
        );

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
                                crate::metrics::active_subscriptions().increment(1.0);
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
                            SubscriptionMessage::GetCount(response) => {
                                let _ = response.send(subscriptions.len());
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
        match &self.outgoing_sender {
            Some(sender) => sender.capacity(),
            None => 0,
        }
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

    pub async fn save_event(&self, event_builder: StoreCommand) -> Result<(), Error> {
        match event_builder {
            StoreCommand::SaveUnsignedEvent(event) => {
                if let Err(e) = self.replaceable_event_queue.send(event) {
                    error!(
                        target: "event_store",
                        "[{}] Error sending event to replaceable events sender: {:?}",
                        self.id,
                        e
                    );
                }
            }
            StoreCommand::SaveSignedEvent(event) => {
                if let Err(e) = self.database.save_signed_event(&event).await {
                    error!(
                        target: "event_store",
                        "[{}] Error saving signed event: {:?}",
                        self.id,
                        e
                    );
                }
                info!(
                    target: "event_store",
                    "[{}] Saved signed event: {}",
                    self.id,
                    event.id
                );
            }
            StoreCommand::DeleteEvents(filter) => {
                let filter_string = format!("{:?}", filter);
                if let Err(e) = self.database.delete(filter).await {
                    error!(
                        target: "event_store",
                        "[{}] Error deleting events: {:?}",
                        self.id,
                        e
                    );
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

    pub async fn handle_unsigned_event(&self, event: UnsignedEvent) -> Result<(), Error> {
        debug!(
            target: "event_store",
            "[{}] Handling unsigned event from {}",
            self.id,
            self.db_connection
        );

        if let Err(e) = self.replaceable_event_queue.send(event) {
            error!(
                target: "event_store",
                "[{}] Failed to send event to replaceable events buffer: {:?}",
                self.id,
                e
            );
            return Err(Error::Internal {
                message: format!("Failed to send event to replaceable events buffer: {}", e),
                backtrace: Backtrace::capture(),
            });
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

        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::Add(subscription_id, filters))
        {
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
    pub async fn subscription_count(&self) -> Result<usize, Error> {
        let (tx, rx) = oneshot::channel();
        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::GetCount(tx))
        {
            error!(
                target: "event_store",
                "[{}] Failed to get subscription count: {:?}",
                self.id,
                e
            );
            return Ok(0); // Return 0 on error to avoid breaking metrics
        }

        match rx.await {
            Ok(count) => Ok(count),
            Err(e) => {
                error!(
                    target: "event_store",
                    "[{}] Failed to receive subscription count: {:?}",
                    self.id,
                    e
                );
                Ok(0) // Return 0 on error to avoid breaking metrics
            }
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
