use crate::create_client::create_client;
use crate::error::Error;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use tokio::sync::mpsc::{channel, Sender};
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

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

    pub async fn flush(&mut self, client: &Client, broadcast_sender: &Sender<Event>) {
        if self.buffer.is_empty() {
            return;
        }

        let Ok(signer) = client.signer().await else {
            error!("Error getting signer");
            return;
        };

        for (_, event) in self.buffer.drain() {
            match signer.sign_event(event).await {
                Ok(event) => {
                    debug!("Saving replaceable event: kind={}", event.kind);
                    if let Err(e) = client.send_event(event.clone()).await {
                        error!("Error sending replaceable event: {:?}", e);
                    } else {
                        debug!("Broadcasting replaceable event: kind={}", event.kind);
                        if let Err(e) = broadcast_sender.send(event).await {
                            error!("Error sending event to broadcast channel: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    error!("Error signing event: {:?}", e);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct RelayClientConnection {
    client: Client,
    pub connection_token: CancellationToken,
    replaceable_event_queue: Sender<UnsignedEvent>,
    broadcast_sender: Sender<Event>,
}

impl RelayClientConnection {
    pub async fn new(
        relay_url: String,
        relay_keys: Keys,
        cancellation_token: CancellationToken,
        broadcast_sender: Sender<Event>,
    ) -> Result<Self> {
        let client = create_client(&relay_url, relay_keys).await?;
        let (sender, mut receiver) = channel::<UnsignedEvent>(10);

        let connection = Self {
            client,
            connection_token: cancellation_token.child_token(),
            replaceable_event_queue: sender,
            broadcast_sender,
        };

        let client_clone = connection.client.clone();
        let token = connection.connection_token.clone();
        let broadcast_sender = connection.broadcast_sender.clone();

        // Box::pin used to avoid filling the stack with the captured variables
        tokio::spawn(Box::pin(async move {
            let mut buffer = ReplaceableEventsBuffer::new();

            loop {
                tokio::select! {
                    _ = token.cancelled() => {
                        buffer.flush(&client_clone, &broadcast_sender).await;
                        return;
                    }

                    event = receiver.recv() => {
                        if let Some(event) = event {
                            buffer.insert(event);
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        buffer.flush(&client_clone, &broadcast_sender).await;
                    }
                }
            }
        }));

        Ok(connection)
    }

    pub async fn save_event(&self, event_builder: EventToSave) -> Result<(), Error> {
        match event_builder {
            EventToSave::UnsignedEvent(event) => {
                debug!("Queueing unsigned event for signing: kind={}", event.kind);
                if let Err(e) = self.replaceable_event_queue.send(event).await {
                    error!("Error sending event to replaceable events sender: {:?}", e);
                }
            }
            EventToSave::Event(event) => {
                debug!("Saving regular event: kind={}", event.kind);
                self.client.send_event(event.clone()).await?;
                debug!("Broadcasting regular event: kind={}", event.kind);
                if let Err(e) = self.broadcast_sender.send(event).await {
                    error!("Error sending event to broadcast channel: {:?}", e);
                }
            }
        }
        Ok(())
    }

    pub async fn send_event(&self, event: Event) -> Result<(), Error> {
        debug!("Sending event directly: kind={}", event.kind);
        self.client.send_event(event.clone()).await?;
        debug!("Broadcasting directly sent event: kind={}", event.kind);
        if let Err(e) = self.broadcast_sender.send(event).await {
            error!("Error sending event to broadcast channel: {:?}", e);
        }
        Ok(())
    }

    pub async fn fetch_events(
        &self,
        filters: Vec<Filter>,
        timeout: Option<Duration>,
    ) -> Result<Events, Error> {
        Ok(self.client.fetch_events(filters, timeout).await?)
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
