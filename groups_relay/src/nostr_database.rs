use anyhow::Result;
use nostr_database::nostr::{Event, Filter};
use nostr_database::{Events, NostrEventsDatabase};
use nostr_lmdb::NostrLMDB;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct RelayDatabase {
    inner: Arc<NostrLMDB>,
    event_sender: broadcast::Sender<Event>,
    keys: Keys,
}

impl RelayDatabase {
    pub fn new(path: String, keys: Keys) -> Result<Self> {
        let database = NostrLMDB::open(path)?;
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

    async fn save(&self, event: &Event) -> Result<()> {
        debug!("Saving event to database: {}", event.as_json());
        match NostrEventsDatabase::save_event(&*self.inner, event).await {
            Ok(_) => {
                debug!("Event saved successfully, event: {}", event.as_json());
                Ok(())
            }
            Err(e) => {
                error!("Error saving event: {:?}", e);
                Err(e.into())
            }
        }
    }

    pub async fn query(&self, filters: Vec<Filter>) -> Result<Events> {
        debug!("Fetching events with filters: {:?}", filters);

        // Create an empty events collection with a default filter
        let mut all_events = Events::new(&Filter::new());

        // Handle each filter individually and combine results
        for filter in filters {
            match NostrEventsDatabase::query(&*self.inner, filter).await {
                Ok(events) => {
                    debug!("Fetched {} events for filter", events.len());
                    all_events.extend(events);
                }
                Err(e) => {
                    error!("Error fetching events: {:?}", e);
                    return Err(e.into());
                }
            }
        }

        debug!("Fetched {} total events", all_events.len());
        Ok(all_events)
    }

    pub async fn save_signed_event(&self, event: &Event) -> Result<()> {
        debug!("Saving signed event: {}", event.id);
        self.save(event).await?;

        // Broadcast the event after successful save
        // It's normal for this to fail if there are no subscribers
        if let Err(e) = self.event_sender.send(event.clone()) {
            debug!("No subscribers available for broadcast: {:?}", e);
        }

        Ok(())
    }

    pub async fn save_unsigned_event(&self, unsigned_event: UnsignedEvent) -> Result<Event> {
        debug!("Signing and saving event");
        let event = self.keys.sign_event(unsigned_event).await?;
        self.save_signed_event(&event).await?;
        Ok(event)
    }

    pub async fn delete(&self, filter: Filter) -> Result<()> {
        debug!("Deleting events with filter: {:?}", filter);
        match NostrEventsDatabase::delete(&*self.inner, filter).await {
            Ok(_) => {
                debug!("Deleted events");
                Ok(())
            }
            Err(e) => {
                error!("Error deleting events: {:?}", e);
                Err(e.into())
            }
        }
    }
}

impl AsRef<NostrLMDB> for RelayDatabase {
    fn as_ref(&self) -> &NostrLMDB {
        &self.inner
    }
}

impl From<Arc<NostrLMDB>> for RelayDatabase {
    fn from(database: Arc<NostrLMDB>) -> Self {
        let (event_sender, _) = broadcast::channel(1024);
        Self {
            inner: database,
            event_sender,
            keys: Keys::generate(), // This should never be used, just for From impl
        }
    }
}

impl From<RelayDatabase> for Arc<NostrLMDB> {
    fn from(database: RelayDatabase) -> Self {
        database.inner
    }
}
