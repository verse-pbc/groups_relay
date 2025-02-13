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
                                local_subscription_count.fetch_add(1, Ordering::Relaxed);
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
                    }
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
                    "[{}] Saved signed event: kind={} {}",
                    self.id,
                    event.kind,
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
    pub fn get_local_subscription_count(&self) -> usize {
        self.local_subscription_count.load(Ordering::Relaxed)
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
