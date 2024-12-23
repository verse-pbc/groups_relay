use crate::database::NostrDatabase;
use crate::error::Error;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};
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

// Configuration struct for EventStoreConnection
#[derive(Debug)]
pub struct EventStoreConfig {
    pub id: String,
    pub database: Arc<NostrDatabase>,
    pub relay_url: String,
    pub cancellation_token: CancellationToken,
}

// Add after the SubscriptionMessage enum
struct ReplaceableEventsBuffer {
    buffer: HashMap<(PublicKey, Kind), UnsignedEvent>,
}

impl ReplaceableEventsBuffer {
    pub fn new() -> Self {
        Self {
            buffer: HashMap::new(),
        }
    }

    pub fn insert(&mut self, event: UnsignedEvent) {
        self.buffer.insert((event.pubkey, event.kind), event);
    }

    pub async fn flush(&mut self, database: &Arc<NostrDatabase>) {
        if self.buffer.is_empty() {
            return;
        }

        for (_, event) in self.buffer.drain() {
            match database.save_event(event).await {
                Ok(event) => {
                    debug!(
                        target: "relay_client",
                        "Saved replaceable event: kind={}",
                        event.kind
                    );
                }
                Err(e) => {
                    error!(
                        target: "relay_client",
                        "Error saving event: {:?}",
                        e
                    );
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct EventStoreConnection {
    id: String,
    database: Arc<NostrDatabase>,
    pub connection_token: CancellationToken,
    replaceable_event_queue: mpsc::UnboundedSender<UnsignedEvent>,
    subscription_sender: mpsc::UnboundedSender<SubscriptionMessage>,
    pub message_sender: Option<MessageSender<RelayMessage>>,
}

impl EventStoreConnection {
    pub async fn new(
        config: EventStoreConfig,
        outgoing_sender: MessageSender<RelayMessage>,
    ) -> Result<Self> {
        let id_clone = config.id.clone();
        debug!(
            target: "relay_client",
            "[{}] Creating new RelayClientConnection for {}",
            id_clone,
            config.relay_url
        );

        let (sender, receiver) = mpsc::unbounded_channel::<UnsignedEvent>();
        let (subscription_sender, subscription_receiver) = mpsc::unbounded_channel();

        let connection = Self {
            id: id_clone.clone(),
            database: config.database.clone(),
            connection_token: config.cancellation_token.child_token(),
            replaceable_event_queue: sender,
            subscription_sender,
            message_sender: Some(outgoing_sender.clone()),
        };

        // Spawn subscription management task
        let token = connection.connection_token.clone();
        let id_clone2 = id_clone.clone();
        tokio::spawn(Box::pin(async move {
            let mut subscriptions: HashMap<SubscriptionId, Vec<Filter>> = HashMap::new();
            let mut subscription_receiver = subscription_receiver;

            debug!(
                target: "relay_client",
                "[{}] Starting subscription manager",
                id_clone2
            );

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!(
                            target: "relay_client",
                            "[{}] Subscription manager shutting down",
                            id_clone2
                        );
                        break;
                    }
                    Some(msg) = subscription_receiver.recv() => {
                        match msg {
                            SubscriptionMessage::Add(subscription_id, filters) => {
                                debug!(
                                    target: "relay_client",
                                    "[{}] Adding subscription {} (current count: {})",
                                    id_clone2, subscription_id, subscriptions.len()
                                );
                                subscriptions.insert(subscription_id, filters);
                            }
                            SubscriptionMessage::Remove(subscription_id) => {
                                if subscriptions.remove(&subscription_id).is_some() {
                                    debug!(
                                        target: "relay_client",
                                        "[{}] Removed subscription {} (remaining count: {})",
                                        id_clone2,
                                        subscription_id,
                                        subscriptions.len()
                                    );
                                }
                            }
                            SubscriptionMessage::CheckEvent { event, sender } => {
                                let mut sender = sender;
                                for (subscription_id, filters) in subscriptions.iter() {
                                    let matches = filters.iter().any(|filter| filter.match_event(&event));
                                    if matches {
                                        debug!(
                                            target: "relay_client",
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
                                                target: "relay_client",
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

        // Spawn replaceable events handler
        let database_clone = connection.database.clone();
        let token = connection.connection_token.clone();
        let id_clone2 = id_clone.clone();

        tokio::spawn(Box::pin(async move {
            let mut buffer = ReplaceableEventsBuffer::new();
            let mut receiver = receiver;

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        debug!(
                            target: "relay_client",
                            "[{}] Replaceable events handler shutting down",
                            id_clone2
                        );
                        buffer.flush(&database_clone).await;
                        return;
                    }

                    event = receiver.recv() => {
                        if let Some(event) = event {
                            buffer.insert(event);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        buffer.flush(&database_clone).await;
                    }
                }
            }
        }));

        // Spawn broadcast handler
        let token_clone = connection.connection_token.clone();
        let subscription_sender_clone = connection.subscription_sender.clone();
        let id_clone3 = id_clone.clone();
        let outgoing_sender = outgoing_sender.clone();
        let mut broadcast_receiver = config.database.subscribe();
        tokio::spawn(Box::pin(async move {
            debug!(
                target: "relay_client",
                "[{}] Starting broadcast event handler",
                id_clone3
            );

            loop {
                tokio::select! {
                    _ = token_clone.cancelled() => {
                        debug!(
                            target: "relay_client",
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
                                target: "relay_client",
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
            target: "relay_client",
            "[{}] RelayClientConnection created successfully",
            id_clone
        );

        Ok(connection)
    }

    pub fn set_message_sender(&mut self, sender: MessageSender<RelayMessage>) {
        self.message_sender = Some(sender);
    }

    pub fn add_subscription(&self, subscription_id: SubscriptionId, filters: Vec<Filter>) {
        if let Err(e) = self
            .subscription_sender
            .send(SubscriptionMessage::Add(subscription_id, filters))
        {
            error!(
                target: "relay_client",
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
                target: "relay_client",
                "[{}] Failed to send remove subscription message: {:?}",
                self.id,
                e
            );
        }
    }

    pub async fn handle_broadcast_event(&self, event: &Event) -> Result<(), Error> {
        let Some(sender) = &self.message_sender else {
            error!(
                target: "relay_client",
                "[{}] No message sender available for connection", self.id
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
                target: "relay_client",
                "[{}] Failed to send check event message: {:?}",
                self.id,
                e
            );
        }
        Ok(())
    }

    pub async fn save_event(&self, event_builder: EventToSave) -> Result<(), Error> {
        match event_builder {
            EventToSave::UnsignedEvent(event) => {
                if let Err(e) = self.replaceable_event_queue.send(event) {
                    error!(
                        target: "relay_client",
                        "[{}] Error sending event to replaceable events sender: {:?}",
                        self.id,
                        e
                    );
                }
            }
            EventToSave::Event(event) => {
                self.save_signed_event(event.clone()).await?;
            }
        }
        Ok(())
    }

    pub async fn save_signed_event(&self, event: Event) -> Result<(), Error> {
        let client_message = RelayMessage::event(SubscriptionId::new("adfs"), event.clone());
        if let Err(e) = self.database.process_event(&client_message.as_json()) {
            error!(
                target: "relay_client",
                "[{}] Error sending event to database: {:?}",
                self.id,
                e
            );
        }
        Ok(())
    }

    pub async fn fetch_events(&self, filters: Vec<Filter>) -> Result<Events, Error> {
        match self.database.query(filters).await {
            Ok(events) => Ok(events),
            Err(e) => Err(Error::notice(format!("Failed to fetch events: {:?}", e))),
        }
    }
}

pub enum EventToSave {
    UnsignedEvent(UnsignedEvent),
    Event(Event),
}

impl EventToSave {
    pub fn is_replaceable(&self) -> bool {
        match self {
            EventToSave::UnsignedEvent(event) => event.kind.is_replaceable(),
            EventToSave::Event(event) => event.kind.is_replaceable(),
        }
    }
}
