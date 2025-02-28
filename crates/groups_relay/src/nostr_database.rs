use crate::error::Error;
use crate::subscription_manager::StoreCommand;
use crate::utils::get_blocking_runtime;
use anyhow::Result;
use nostr_database::nostr::{Event, Filter};
use nostr_database::{Events, NostrEventsDatabase};
use nostr_lmdb::NostrLMDB;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::task::spawn_blocking;
use tracing::{debug, error, info};
use tracing_futures::Instrument;

#[derive(Debug, Clone)]
pub struct RelayDatabase {
    inner: Arc<NostrLMDB>,
    // Broadcast events to REQ subscriptions.
    broadcast_sender: broadcast::Sender<Box<Event>>,
    // Queue store commands for sequential processing.
    store_sender: mpsc::UnboundedSender<StoreCommand>,
}

impl RelayDatabase {
    pub fn new(path: String, keys: Keys) -> Result<Self, Error> {
        let database = NostrLMDB::open(path)?;
        let inner = Arc::new(database);
        let (store_sender, store_receiver) = mpsc::unbounded_channel();
        let (broadcast_sender, _) = broadcast::channel(1024);

        let relay_db = Self {
            inner,
            broadcast_sender: broadcast_sender.clone(),
            store_sender,
        };

        // Shared keys for signing.
        let keys = Arc::new(keys);
        // Spawn a dedicated task to process store commands.
        Self::spawn_store_processor(store_receiver, keys, relay_db.clone(), broadcast_sender);

        Ok(relay_db)
    }

    fn spawn_store_processor(
        mut store_receiver: mpsc::UnboundedReceiver<StoreCommand>,
        keys: Arc<Keys>,
        relay_db: Self,
        broadcast_sender: broadcast::Sender<Box<Event>>,
    ) {
        // Capture the current span to propagate context to the spawned task
        let span = tracing::Span::current();

        tokio::spawn(
            async move {
                while let Some(store_command) = store_receiver.recv().await {
                    match store_command {
                        StoreCommand::DeleteEvents(filter) => {
                            info!("Deleting events with filter: {:?}", filter);
                            if let Err(e) = relay_db.delete(filter).await {
                                error!("Error deleting events: {:?}", e);
                            }
                        }
                        StoreCommand::SaveSignedEvent(event) => {
                            Self::handle_signed_event(&relay_db, event, &broadcast_sender).await;
                        }
                        StoreCommand::SaveUnsignedEvent(unsigned_event) => {
                            let keys = keys.clone();
                            let sign_result = spawn_blocking(move || {
                                get_blocking_runtime().block_on(keys.sign_event(unsigned_event))
                            })
                            .await;

                            match sign_result {
                                Ok(Ok(event)) => {
                                    Self::handle_signed_event(
                                        &relay_db,
                                        Box::new(event),
                                        &broadcast_sender,
                                    )
                                    .await;
                                }
                                Ok(Err(e)) => {
                                    error!("Error signing unsigned event: {:?}", e);
                                }
                                Err(e) => {
                                    error!("Spawn blocking task failed: {:?}", e);
                                }
                            }
                        }
                    }
                }
            }
            .instrument(span),
        );
    }

    async fn handle_signed_event(
        relay_db: &Self,
        event: Box<Event>,
        broadcast_sender: &broadcast::Sender<Box<Event>>,
    ) {
        info!("Saving event: {}", event.as_json());
        if let Err(e) = relay_db.save_event(event.as_ref()).await {
            error!("Error saving event: {:?}", e);
        } else if let Err(e) = broadcast_sender.send(event) {
            debug!("No subscribers available for broadcast: {:?}", e);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Box<Event>> {
        self.broadcast_sender.subscribe()
    }

    async fn save_event(&self, event: &Event) -> Result<()> {
        match self.inner.save_event(event).await {
            Ok(_) => {
                debug!("Event saved successfully: {}", event.as_json());
                Ok(())
            }
            Err(e) => {
                error!("Error saving event: {:?}", e);
                Err(e.into())
            }
        }
    }

    async fn delete(&self, filter: Filter) -> Result<()> {
        match self.inner.delete(filter).await {
            Ok(_) => {
                debug!("Deleted events successfully");
                Ok(())
            }
            Err(e) => {
                error!("Error deleting events: {:?}", e);
                Err(e.into())
            }
        }
    }

    pub async fn save_store_command(&self, store_command: StoreCommand) -> Result<(), Error> {
        self.store_sender
            .send(store_command)
            .map_err(|e| Error::internal(format!("Failed to queue store command: {}", e)))
    }

    pub async fn save_unsigned_event(&self, event: UnsignedEvent) -> Result<(), Error> {
        self.save_store_command(StoreCommand::SaveUnsignedEvent(event))
            .await
    }

    pub async fn save_signed_event(&self, event: Event) -> Result<(), Error> {
        self.save_store_command(StoreCommand::SaveSignedEvent(Box::new(event)))
            .await
    }

    pub async fn query(&self, filters: Vec<Filter>) -> Result<Events> {
        debug!("Fetching events with filters: {:?}", filters);
        let mut all_events = Events::new(&Filter::new());

        for filter in filters {
            match self.inner.query(filter).await {
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
}

impl AsRef<NostrLMDB> for RelayDatabase {
    fn as_ref(&self) -> &NostrLMDB {
        &self.inner
    }
}

impl From<RelayDatabase> for Arc<NostrLMDB> {
    fn from(database: RelayDatabase) -> Self {
        database.inner
    }
}
