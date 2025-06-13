//! Database abstraction for Nostr relays

use crate::crypto_worker::CryptoWorker;
use crate::error::Error;
use crate::subscription_service::StoreCommand;
use nostr_database::nostr::{Event, Filter};
use nostr_database::Events;
use nostr_lmdb::{NostrLMDB, Scope};
use nostr_sdk::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error};
use tracing_futures::Instrument;

/// A Nostr relay database that wraps NostrLMDB with async operations and event broadcasting
#[derive(Debug)]
pub struct RelayDatabase {
    env: Arc<NostrLMDB>,
    #[allow(dead_code)]
    db_path: PathBuf,
    broadcast_sender: broadcast::Sender<Box<Event>>,
    store_sender: mpsc::UnboundedSender<StoreCommand>,
    #[allow(dead_code)]
    crypto_worker: Arc<CryptoWorker>,
}

impl RelayDatabase {
    /// Create a new relay database
    ///
    /// # Arguments
    /// * `db_path_param` - Path where the database should be stored
    /// * `crypto_worker` - Crypto worker for signing unsigned events
    pub fn new(
        db_path_param: impl AsRef<std::path::Path>,
        crypto_worker: Arc<CryptoWorker>,
    ) -> Result<Self, Error> {
        let db_path = db_path_param.as_ref().to_path_buf();

        // Ensure database directory exists
        if let Some(parent) = db_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::database(format!(
                        "Failed to create database directory parent '{:?}': {}",
                        parent, e
                    ))
                })?;
            }
        }
        if !db_path.exists() {
            std::fs::create_dir_all(&db_path).map_err(|e| {
                Error::database(format!(
                    "Failed to create database directory '{:?}': {}",
                    db_path, e
                ))
            })?;
        }

        // Open LMDB database
        let lmdb_instance = NostrLMDB::open(&db_path).map_err(|e| {
            Error::database(format!(
                "Failed to open NostrLMDB at path '{:?}': {}",
                db_path, e
            ))
        })?;
        let env = Arc::new(lmdb_instance);

        // Create channels for async operations
        let (store_sender, store_receiver) = mpsc::unbounded_channel();
        let (broadcast_sender, _) = broadcast::channel(1024);

        let relay_db = Self {
            env: Arc::clone(&env),
            db_path: db_path.clone(),
            broadcast_sender: broadcast_sender.clone(),
            store_sender,
            crypto_worker: Arc::clone(&crypto_worker),
        };

        // Spawn background task to process store commands
        Self::spawn_store_processor(
            store_receiver,
            Arc::clone(&crypto_worker),
            Arc::clone(&relay_db.env),
            broadcast_sender,
        );

        Ok(relay_db)
    }

    /// Spawn the background task that processes store commands
    fn spawn_store_processor(
        mut store_receiver: mpsc::UnboundedReceiver<StoreCommand>,
        crypto_worker: Arc<CryptoWorker>,
        env_clone: Arc<NostrLMDB>,
        broadcast_sender: broadcast::Sender<Box<Event>>,
    ) {
        // Create isolated span for store processor task
        let store_span = tracing::info_span!(parent: None, "store_processor_task");
        tokio::spawn(
            async move {
                let get_processor_env = |_scope: &Scope| -> Result<Arc<NostrLMDB>, Error> {
                    Ok(Arc::clone(&env_clone))
                };

                while let Some(store_command) = store_receiver.recv().await {
                    let scope_clone = store_command.subdomain_scope().clone();

                    match store_command {
                        StoreCommand::DeleteEvents(filter, _) => {
                            debug!(
                                "Deleting events with filter: {:?} for scope: {:?}",
                                filter, scope_clone
                            );
                            match get_processor_env(&scope_clone) {
                                Ok(env) => {
                                    let scoped_view = match env.scoped(&scope_clone) {
                                        Ok(view) => view,
                                        Err(e) => {
                                            error!("Error getting scoped view: {:?}", e);
                                            continue;
                                        }
                                    };
                                    match scoped_view.delete(filter).await {
                                        Ok(_) => debug!("Deleted events successfully"),
                                        Err(e) => error!("Error deleting events: {:?}", e),
                                    }
                                }
                                Err(e) => error!("Failed to get env for delete: {:?}", e),
                            }
                        }
                        StoreCommand::SaveSignedEvent(event, _) => {
                            match get_processor_env(&scope_clone) {
                                Ok(env) => {
                                    Self::handle_signed_event(
                                        env,
                                        event,
                                        &broadcast_sender,
                                        &scope_clone,
                                    )
                                    .await;
                                }
                                Err(e) => error!("Failed to get env for signed event: {:?}", e),
                            }
                        }
                        StoreCommand::SaveUnsignedEvent(unsigned_event, _) => {
                            // Use the crypto worker for signing
                            match crypto_worker.sign_event(unsigned_event).await {
                                Ok(event) => match get_processor_env(&scope_clone) {
                                    Ok(env) => {
                                        Self::handle_signed_event(
                                            env,
                                            Box::new(event),
                                            &broadcast_sender,
                                            &scope_clone,
                                        )
                                        .await;
                                    }
                                    Err(e) => {
                                        error!("Failed to get env for unsigned event: {:?}", e)
                                    }
                                },
                                Err(e) => {
                                    error!("Error signing unsigned event: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            .instrument(store_span),
        );
    }

    /// Handle saving a signed event
    async fn handle_signed_event(
        env: Arc<NostrLMDB>,
        event: Box<Event>,
        broadcast_sender: &broadcast::Sender<Box<Event>>,
        scope: &Scope,
    ) {
        debug!("Saving event: {} for scope: {:?}", event.as_json(), scope);

        match env.scoped(scope) {
            Ok(scoped_view) => {
                if let Err(e) = scoped_view.save_event(event.as_ref()).await {
                    error!("Error saving event: {:?}", e);
                } else if let Err(e) = broadcast_sender.send(event) {
                    debug!("No subscribers available for broadcast: {:?}", e);
                }
            }
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
            }
        }
    }

    /// Subscribe to receive all saved events
    pub fn subscribe(&self) -> broadcast::Receiver<Box<Event>> {
        self.broadcast_sender.subscribe()
    }

    /// Save an event directly (synchronous)
    pub async fn save_event(&self, event: &Event, scope: &Scope) -> Result<()> {
        let env = self.get_env(scope).await?;

        match env.scoped(scope) {
            Ok(scoped_view) => match scoped_view.save_event(event).await {
                Ok(_) => {
                    debug!(
                        "Event saved successfully: {} for scope: {:?}",
                        event.as_json(),
                        scope
                    );
                    Ok(())
                }
                Err(e) => {
                    error!("Error saving event for scope {:?}: {:?}", scope, e);
                    Err(e.into())
                }
            },
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                Err(Box::new(Error::database(format!(
                    "Failed to get scoped view: {}",
                    e
                ))))
            }
        }
    }

    /// Delete events matching a filter
    pub async fn delete(&self, filter: Filter, scope: &Scope) -> Result<()> {
        let env = self.get_env(scope).await?;

        match env.scoped(scope) {
            Ok(scoped_view) => match scoped_view.delete(filter).await {
                Ok(_) => {
                    debug!("Deleted events successfully for scope: {:?}", scope);
                    Ok(())
                }
                Err(e) => {
                    error!("Error deleting events for scope {:?}: {:?}", scope, e);
                    Err(e.into())
                }
            },
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                Err(Box::new(Error::database(format!(
                    "Failed to get scoped view: {}",
                    e
                ))))
            }
        }
    }

    /// Query events from the database
    pub async fn query(&self, filters: Vec<Filter>, scope: &Scope) -> Result<Events, Error> {
        let env = self.get_env(scope).await?;

        match env.scoped(scope) {
            Ok(scoped_view) => {
                let mut all_events = Events::new(&Filter::new());

                // Query each filter separately and combine results
                for filter in filters {
                    match scoped_view.query(filter).await {
                        Ok(events) => {
                            debug!(
                                "Fetched {} events for filter for scope: {:?}",
                                events.len(),
                                scope
                            );
                            all_events.extend(events);
                        }
                        Err(e) => {
                            error!("Error querying events: {:?}", e);
                            return Err(Error::database(format!("Failed to query events: {}", e)));
                        }
                    }
                }

                Ok(all_events)
            }
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                Err(Error::database(format!("Failed to get scoped view: {}", e)))
            }
        }
    }

    /// Get count of events matching filters
    pub async fn count(&self, filters: Vec<Filter>, scope: &Scope) -> Result<usize, Error> {
        let env = self.get_env(scope).await?;

        match env.scoped(scope) {
            Ok(scoped_view) => {
                let mut total_count = 0;

                // Count each filter separately and sum results
                for filter in filters {
                    match scoped_view.count(filter).await {
                        Ok(count) => total_count += count,
                        Err(e) => {
                            error!("Error counting events: {:?}", e);
                            return Err(Error::database(format!("Failed to count events: {}", e)));
                        }
                    }
                }

                Ok(total_count)
            }
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                Err(Error::database(format!("Failed to get scoped view: {}", e)))
            }
        }
    }

    /// Get the database environment for a scope
    async fn get_env(&self, _scope: &Scope) -> Result<Arc<NostrLMDB>, Error> {
        // In this implementation, we use a single database for all scopes
        Ok(Arc::clone(&self.env))
    }

    /// List all scopes available in the database
    pub async fn list_scopes(&self) -> Result<Vec<Scope>, Error> {
        let env = Arc::clone(&self.env);

        let scopes = tokio::task::spawn_blocking(move || env.list_scopes())
            .await
            .map_err(|e| Error::database(format!("Failed to spawn blocking task: {}", e)))?
            .map_err(|e| Error::database(format!("Failed to list scopes: {}", e)))?;

        Ok(scopes)
    }

    /// Save a signed event through the async processor
    pub async fn save_signed_event(&self, event: Event, scope: Scope) -> Result<(), Error> {
        self.save_store_command(StoreCommand::SaveSignedEvent(Box::new(event), scope))
            .await
    }

    /// Save an unsigned event (will be signed in the processor)
    pub async fn save_unsigned_event(
        &self,
        event: UnsignedEvent,
        scope: Scope,
    ) -> Result<(), Error> {
        self.save_store_command(StoreCommand::SaveUnsignedEvent(event, scope))
            .await
    }

    /// Save a store command to the async processor
    pub async fn save_store_command(&self, command: StoreCommand) -> Result<(), Error> {
        self.store_sender
            .send(command)
            .map_err(|e| Error::internal(format!("Failed to send store command: {}", e)))
    }
}

pub type NostrDatabase = Arc<RelayDatabase>;
