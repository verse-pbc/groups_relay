use crate::error::Error;
use crate::event_store_connection::StoreCommand;
use crate::utils::get_blocking_runtime;
use anyhow::Result;
use nostr_database::nostr::{Event, Filter};
use nostr_database::{Events, NostrEventsDatabase};
use nostr_lmdb::NostrLMDB;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::{mpsc, oneshot};
use tokio::task::spawn_blocking;
use tracing::{debug, error};

#[derive(Debug)]
pub struct StoreMessage {
    pub command: StoreCommand,
    pub reply_to: oneshot::Sender<Event>,
}

#[derive(Debug, Clone)]
pub struct RelayDatabase {
    inner: Arc<NostrLMDB>,
    event_sender: broadcast::Sender<Event>,
    store_sender: mpsc::UnboundedSender<StoreMessage>,
}

impl RelayDatabase {
    pub fn new(path: String, keys: Keys) -> Result<Self> {
        let database = NostrLMDB::open(path)?;
        let (event_sender, _) = broadcast::channel(1024);

        let (store_sender, mut store_receiver): (
            mpsc::UnboundedSender<StoreMessage>,
            mpsc::UnboundedReceiver<StoreMessage>,
        ) = mpsc::unbounded_channel();

        let (store_command_sender, mut store_command_receiver): (
            mpsc::UnboundedSender<StoreCommand>,
            mpsc::UnboundedReceiver<StoreCommand>,
        ) = mpsc::unbounded_channel();

        let inner = Arc::new(database);

        // Spawn a task that processes store operations sequentially
        let this = Self {
            inner,
            event_sender,
            store_sender,
        };
        let this_clone = this.clone();

        // Save task
        tokio::spawn(async move {
            while let Some(command) = store_command_receiver.recv().await {
                match command {
                    StoreCommand::SaveSignedEvent(event) => {
                        debug!("Processing save signed event: {}", event.as_json());
                        if let Err(e) = this_clone.save_event(&event).await {
                            error!("Error saving signed event: {:?}", e);
                        }
                    }
                    StoreCommand::SaveUnsignedEvent(_) => {
                        unreachable!(
                            "Unsigned events commands should be handled by the store_sender"
                        );
                    }
                    StoreCommand::DeleteEvents(filter) => {
                        debug!("Processing delete events with filter: {:?}", filter);
                        if let Err(e) = this_clone.delete(filter).await {
                            error!("Error deleting events: {:?}", e);
                        }
                    }
                }
            }
            debug!("Store command processor shutting down");
        });

        // Signs if needed and reply after sending to save task, we don't wait for save task to finish
        let keys = Arc::new(keys);
        tokio::spawn(async move {
            while let Some(message) = store_receiver.recv().await {
                if let StoreCommand::DeleteEvents(filter) = message.command {
                    if let Err(e) = store_command_sender.send(StoreCommand::DeleteEvents(filter)) {
                        error!("Error sending delete command: {:?}", e);
                    }
                    continue;
                }

                let signed_event = match message.command {
                    StoreCommand::SaveSignedEvent(event) => event,
                    StoreCommand::SaveUnsignedEvent(unsigned_event) => {
                        let keys = keys.clone();
                        // Signing is CPU-intensive and blocks, so we offload it
                        // to a blocking thread using spawn_blocking. Using a shared
                        // runtime prevents resource exhaustion.
                        let sign_result = spawn_blocking(move || {
                            get_blocking_runtime().block_on(keys.sign_event(unsigned_event))
                        })
                        .await;

                        match sign_result {
                            Ok(Ok(event)) => event,
                            Ok(Err(e)) => {
                                error!("Error signing unsigned event: {:?}", e);
                                continue;
                            }
                            Err(e) => {
                                error!("Spawn blocking task failed: {:?}", e);
                                continue;
                            }
                        }
                    }
                    StoreCommand::DeleteEvents(_) => continue,
                };

                if let Err(e) =
                    store_command_sender.send(StoreCommand::SaveSignedEvent(signed_event.clone()))
                {
                    error!("Error sending save command: {:?}", e);
                }

                if let Err(e) = message.reply_to.send(signed_event) {
                    error!("Error sending reply: {:?}", e);
                }
            }
        });

        Ok(this)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.event_sender.subscribe()
    }

    pub fn get_store_sender(&self) -> mpsc::UnboundedSender<StoreMessage> {
        self.store_sender.clone()
    }

    async fn save_event(&self, event: &Event) -> Result<()> {
        debug!("Saving event to database: {}", event.as_json());
        match self.inner.save_event(event).await {
            Ok(_) => {
                debug!("Event saved successfully, event: {}", event.as_json());
                // Broadcast the event after successful save
                if let Err(e) = self.event_sender.send(event.clone()) {
                    debug!("No subscribers available for broadcast: {:?}", e);
                }
                Ok(())
            }
            Err(e) => {
                error!("Error saving event: {:?}", e);
                Err(e.into())
            }
        }
    }

    async fn delete(&self, filter: Filter) -> Result<()> {
        debug!("Deleting events with filter: {:?}", filter);
        match self.inner.delete(filter).await {
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

    pub async fn save_store_command(
        &self,
        store_command: StoreCommand,
    ) -> Result<oneshot::Receiver<Event>, Error> {
        let (reply_to_sender, reply_to_receiver) = oneshot::channel();

        let store_message = StoreMessage {
            command: store_command,
            reply_to: reply_to_sender,
        };

        self.store_sender
            .send(store_message)
            .map_err(|e| Error::internal(format!("Failed to queue store command: {}", e)))?;

        Ok(reply_to_receiver)
    }

    pub async fn save_unsigned_event(&self, event: UnsignedEvent) -> Result<Event, Error> {
        let receiver = self
            .save_store_command(StoreCommand::SaveUnsignedEvent(event))
            .await?;

        let event = receiver
            .await
            .map_err(|e| Error::internal(format!("Failed to receive stored event: {}", e)))?;

        Ok(event)
    }

    pub async fn save_signed_event(&self, event: Event) -> Result<(), Error> {
        drop(
            self.save_store_command(StoreCommand::SaveSignedEvent(event))
                .await?,
        );

        Ok(())
    }

    pub async fn query(&self, filters: Vec<Filter>) -> Result<Events> {
        debug!("Fetching events with filters: {:?}", filters);

        // Create an empty events collection with a default filter
        let mut all_events = Events::new(&Filter::new());

        // Handle each filter individually and combine results
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
