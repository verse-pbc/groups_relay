use anyhow::Result;
use nostr_ndb::NdbDatabase;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct NostrDatabase {
    inner: Arc<NdbDatabase>,
    event_sender: broadcast::Sender<Event>,
    keys: Keys,
}

impl NostrDatabase {
    pub fn open(path: String, keys: Keys) -> Result<Self> {
        let database = NdbDatabase::open(path)?;
        let (event_sender, _) = broadcast::channel(1024);

        Ok(Self {
            inner: Arc::new(database),
            event_sender,
            keys,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.event_sender.subscribe()
    }

    pub async fn query(&self, filters: Vec<Filter>) -> Result<Events> {
        debug!("Querying database with filters: {:?}", filters);
        match self.inner.query(filters).await {
            Ok(events) => {
                debug!("Found {} events", events.len());
                Ok(events)
            }
            Err(e) => {
                error!("Error querying database: {:?}", e);
                Err(e.into())
            }
        }
    }

    pub fn process_event(&self, event_json: &str) -> Result<()> {
        debug!("Processing event: {}", event_json);
        match self.inner.process_event(event_json) {
            Ok(_) => {
                debug!("Event processed successfully");
                Ok(())
            }
            Err(e) => {
                error!("Error processing event: {:?}", e);
                Err(e.into())
            }
        }
    }

    pub async fn fetch_events(&self, filters: Vec<Filter>) -> Result<Events> {
        debug!("Fetching events with filters: {:?}", filters);
        match self.inner.query(filters).await {
            Ok(events) => {
                debug!("Fetched {} events", events.len());
                Ok(events)
            }
            Err(e) => {
                error!("Error fetching events: {:?}", e);
                Err(e.into())
            }
        }
    }

    pub fn save_signed_event(&self, event: &Event) -> Result<()> {
        debug!("Saving signed event: {}", event.id);
        let client_message = RelayMessage::event(SubscriptionId::new("save"), event.clone());
        self.process_event(&client_message.as_json())?;

        // Broadcast the event after successful save
        if let Err(e) = self.event_sender.send(event.clone()) {
            error!("Failed to broadcast saved event: {:?}", e);
        }

        Ok(())
    }

    pub async fn save_event(&self, unsigned_event: UnsignedEvent) -> Result<Event> {
        debug!("Signing and saving event");
        let event = self.keys.sign_event(unsigned_event).await?;
        self.save_signed_event(&event)?;
        Ok(event)
    }
}

impl AsRef<NdbDatabase> for NostrDatabase {
    fn as_ref(&self) -> &NdbDatabase {
        &self.inner
    }
}

impl From<Arc<NdbDatabase>> for NostrDatabase {
    fn from(database: Arc<NdbDatabase>) -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            inner: database,
            event_sender,
            keys: Keys::generate(), // This should never be used, just for From impl
        }
    }
}

impl From<NostrDatabase> for Arc<NdbDatabase> {
    fn from(database: NostrDatabase) -> Self {
        database.inner
    }
}
