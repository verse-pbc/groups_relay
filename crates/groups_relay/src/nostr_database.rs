use crate::error::Error;
use crate::subscription_manager::StoreCommand;
use crate::utils::get_blocking_runtime;
use nostr_database::nostr::{Event, Filter};
use nostr_database::Events;
use nostr_lmdb::{NostrLMDB, Scope};
use nostr_sdk::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::task::spawn_blocking;
use tracing::{debug, error, info};
use tracing_futures::Instrument;

#[derive(Debug)]
pub struct RelayDatabase {
    env: Arc<NostrLMDB>,
    db_path: PathBuf,
    broadcast_sender: broadcast::Sender<Box<Event>>,
    store_sender: mpsc::UnboundedSender<StoreCommand>,
}

impl RelayDatabase {
    pub fn new(db_path_param: impl AsRef<std::path::Path>, keys: Keys) -> Result<Self, Error> {
        let db_path = db_path_param.as_ref().to_path_buf();

        if let Some(parent) = db_path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Error::internal(format!(
                        "Failed to create database directory parent \'{:?}\': {}",
                        parent, e
                    ))
                })?;
            }
        }
        if !db_path.exists() {
            std::fs::create_dir_all(&db_path).map_err(|e| {
                Error::internal(format!(
                    "Failed to create database directory \'{:?}\': {}",
                    db_path, e
                ))
            })?;
        }

        let lmdb_instance = NostrLMDB::open(&db_path).map_err(|e| {
            Error::internal(format!(
                "Failed to open NostrLMDB at path \'{:?}\': {}",
                db_path, e
            ))
        })?;
        let env = Arc::new(lmdb_instance);

        let (store_sender, store_receiver) = mpsc::unbounded_channel();
        let (broadcast_sender, _) = broadcast::channel(1024);

        let relay_db = Self {
            env: Arc::clone(&env),
            db_path: db_path.clone(),
            broadcast_sender: broadcast_sender.clone(),
            store_sender,
        };

        let keys_arc = Arc::new(keys);
        Self::spawn_store_processor(
            store_receiver,
            keys_arc,
            Arc::clone(&relay_db.env),
            relay_db.db_path.clone(),
            broadcast_sender,
        );

        Ok(relay_db)
    }

    fn spawn_store_processor(
        mut store_receiver: mpsc::UnboundedReceiver<StoreCommand>,
        keys: Arc<Keys>,
        env_clone: Arc<NostrLMDB>,
        db_path_clone: PathBuf,
        broadcast_sender: broadcast::Sender<Box<Event>>,
    ) {
        // Create isolated span for store processor task
        let store_span = tracing::info_span!(parent: None, "store_processor_task");
        tokio::spawn(
            async move {
                struct EnvInfo {
                    env: Arc<NostrLMDB>,
                    path: PathBuf,
                }

                let get_processor_env = |_scope: &Scope| -> Result<EnvInfo, Error> {
                    Ok(EnvInfo {
                        env: Arc::clone(&env_clone),
                        path: db_path_clone.clone(),
                    })
                };

                while let Some(store_command) = store_receiver.recv().await {
                    let scope_clone = store_command.subdomain_scope().clone();

                    // Extract values before the match to avoid borrowing issues
                    match store_command {
                        StoreCommand::DeleteEvents(filter, _) => {
                            info!(
                                "Deleting events with filter: {:?} for scope: {:?} (using single DB)",
                                filter, scope_clone
                            );
                            match get_processor_env(&scope_clone) {
                                Ok(env_info) => {
                                    let scoped_view = match env_info.env.scoped(&scope_clone) {
                                        Ok(view) => view,
                                        Err(e) => {
                                            error!("Error getting scoped view: {:?}, env_path: {:?}", e, env_info.path);
                                            continue;
                                        }
                                    };
                                    match scoped_view.delete(filter).await {
                                        Ok(_) => debug!("Deleted events successfully from processor for path: {:?}", env_info.path),
                                        Err(e) => error!("Error deleting events from processor: {:?}, env_path: {:?}", e, env_info.path),
                                    }
                                },
                                Err(e) => error!("Processor: Failed to get env for delete: {:?} (should not happen with single DB)", e),
                            }
                        }
                        StoreCommand::SaveSignedEvent(event, _) => {
                            match get_processor_env(&scope_clone) {
                                Ok(env_info) => {
                                    Self::handle_signed_event(
                                        env_info.env,
                                        event,
                                        &broadcast_sender,
                                        &scope_clone,
                                    )
                                    .await;
                                }
                                Err(e) => error!("Processor: Failed to get env for signed event: {:?} (should not happen)", e),
                            }
                        }
                        StoreCommand::SaveUnsignedEvent(unsigned_event, _) => {
                            let keys_clone = Arc::clone(&keys);
                            let sign_result = spawn_blocking(move || {
                                get_blocking_runtime()
                                    .block_on(keys_clone.sign_event(unsigned_event))
                            })
                            .await;

                            match sign_result {
                                Ok(Ok(event)) => {
                                    match get_processor_env(&scope_clone) {
                                        Ok(env_info) => {
                                            Self::handle_signed_event(
                                                env_info.env,
                                                Box::new(event),
                                                &broadcast_sender,
                                                &scope_clone,
                                            )
                                            .await;
                                        }
                                        Err(e) => error!("Processor: Failed to get env for unsigned event: {:?} (should not happen)", e),
                                    }
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
            .instrument(store_span),
        );
    }

    async fn handle_signed_event(
        env: Arc<NostrLMDB>,
        event: Box<Event>,
        broadcast_sender: &broadcast::Sender<Box<Event>>,
        scope: &Scope,
    ) {
        info!(
            "Saving event: {} for scope: {:?} (in single DB)",
            event.as_json(),
            scope
        );

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

    pub fn subscribe(&self) -> broadcast::Receiver<Box<Event>> {
        self.broadcast_sender.subscribe()
    }

    pub async fn save_event(&self, event: &Event, scope: &Scope) -> Result<()> {
        let env = self.get_env(scope).await?;

        match env.scoped(scope) {
            Ok(scoped_view) => match scoped_view.save_event(event).await {
                Ok(_) => {
                    debug!(
                        "Event saved successfully: {} for scope: {:?} (in single DB)",
                        event.as_json(),
                        scope
                    );
                    Ok(())
                }
                Err(e) => {
                    error!(
                        "Error saving event for scope {:?} (in single DB): {:?}",
                        scope, e
                    );
                    Err(e.into())
                }
            },
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                Err(Box::new(Error::internal(format!(
                    "Failed to get scoped view: {}",
                    e
                ))))
            }
        }
    }

    pub async fn delete(&self, filter: Filter, scope: &Scope) -> Result<()> {
        let env = self.get_env(scope).await?;

        match env.scoped(scope) {
            Ok(scoped_view) => match scoped_view.delete(filter).await {
                Ok(_) => {
                    debug!(
                        "Deleted events successfully for scope: {:?} (from single DB)",
                        scope
                    );
                    Ok(())
                }
                Err(e) => {
                    error!(
                        "Error deleting events for scope {:?} (from single DB): {:?}",
                        scope, e
                    );
                    Err(e.into())
                }
            },
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                Err(Box::new(Error::internal(format!(
                    "Failed to get scoped view: {}",
                    e
                ))))
            }
        }
    }

    pub async fn save_store_command(
        &self,
        store_command: StoreCommand,
    ) -> std::result::Result<(), Error> {
        self.store_sender
            .send(store_command)
            .map_err(|e| Error::internal(format!("Failed to queue store command: {}", e)))
    }

    pub async fn save_unsigned_event(
        &self,
        event: UnsignedEvent,
        scope: Scope,
    ) -> std::result::Result<(), Error> {
        self.save_store_command(StoreCommand::SaveUnsignedEvent(event, scope))
            .await
    }

    pub async fn save_signed_event(
        &self,
        event: Event,
        scope: Scope,
    ) -> std::result::Result<(), Error> {
        self.save_store_command(StoreCommand::SaveSignedEvent(Box::new(event), scope))
            .await
    }

    pub async fn query(
        &self,
        filters: Vec<Filter>,
        scope: &Scope,
    ) -> std::result::Result<Events, Error> {
        let env = self.get_env(scope).await?;
        let log_path = &self.db_path;
        debug!(
            "Fetching events with filters: {:?} for scope: {:?} using single env at path: {:?}",
            filters, scope, log_path
        );
        let mut all_events = Events::new(&Filter::new());

        let scoped_view = match env.scoped(scope) {
            Ok(view) => view,
            Err(e) => {
                error!("Error getting scoped view: {:?}", e);
                return Err(Error::internal(format!("Failed to get scoped view: {}", e)));
            }
        };

        for filter in filters {
            match scoped_view.query(filter).await {
                Ok(events) => {
                    debug!(
                        "Fetched {} events for filter for scope: {:?} (single DB), env_path: {:?}",
                        events.len(),
                        scope,
                        log_path
                    );
                    all_events.extend(events);
                }
                Err(e) => {
                    error!(
                        "Error fetching events for scope {:?} (single DB): {:?}",
                        scope, e
                    );
                    return Err(e.into());
                }
            }
        }

        debug!(
            "Fetched {} total events for scope: {:?} (from single DB)",
            all_events.len(),
            scope
        );
        Ok(all_events)
    }

    async fn get_env(&self, _scope: &Scope) -> Result<Arc<NostrLMDB>, Error> {
        Ok(Arc::clone(&self.env))
    }

    /// List all available scopes in the database
    pub async fn list_scopes(&self) -> Result<Vec<Scope>, Error> {
        let env = Arc::clone(&self.env);
        // Run list_scopes on a blocking thread since it's a potentially expensive operation
        let scopes = tokio::task::spawn_blocking(move || env.list_scopes())
            .await
            .map_err(|e| {
                Error::internal(format!(
                    "Failed to spawn blocking task for list_scopes: {}",
                    e
                ))
            })?
            .map_err(|e| Error::internal(format!("Failed to list scopes: {}", e)))?;

        Ok(scopes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_event;
    use nostr_sdk::{Keys, Kind};
    use std::collections::HashSet;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_subdomain_data_isolation() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test_single_db");
        let relay_keys = Keys::generate();
        let db = RelayDatabase::new(&db_path, relay_keys).unwrap();

        let root_event_keys = Keys::generate();
        let root_event = create_test_event(&root_event_keys, Kind::TextNote.as_u16(), vec![]).await;

        let oslo_event_keys = Keys::generate();
        let oslo_event = create_test_event(&oslo_event_keys, Kind::TextNote.as_u16(), vec![]).await;

        db.save_event(&root_event, &Scope::Default).await.unwrap();
        db.save_event(&oslo_event, &Scope::named("oslo").unwrap())
            .await
            .unwrap();

        // With the new scoped-heed, there's proper isolation between subdomains
        // so we should update our expectations
        let root_results = db
            .query(vec![Filter::new()], &Scope::Default)
            .await
            .unwrap();
        assert_eq!(
            root_results.len(),
            1,
            "Root query should only return 1 event (with isolation)"
        );

        let oslo_results = db
            .query(vec![Filter::new()], &Scope::named("oslo").unwrap())
            .await
            .unwrap();
        assert_eq!(
            oslo_results.len(),
            1,
            "Oslo query should only return 1 event (with isolation)"
        );

        let root_ids: std::collections::HashSet<EventId> =
            root_results.iter().map(|e| e.id).collect();
        let oslo_ids: std::collections::HashSet<EventId> =
            oslo_results.iter().map(|e| e.id).collect();

        assert!(
            root_ids.contains(&root_event.id),
            "Root results should contain root_event"
        );
        assert!(
            !root_ids.contains(&oslo_event.id),
            "Root results should NOT contain oslo_event"
        );
        assert!(
            !oslo_ids.contains(&root_event.id),
            "Oslo results should NOT contain root_event"
        );
        assert!(
            oslo_ids.contains(&oslo_event.id),
            "Oslo results should contain oslo_event"
        );
    }

    #[tokio::test]
    async fn test_concurrent_operations_single_db() {
        // Tests concurrent save and query operations against the DB model.
        // With the updated scoped-heed model, operations are isolated by subdomain.
        // This test ensures that concurrent access doesn't lead to panics, deadlocks,
        // or lost writes, and that appropriate isolation is maintained.

        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("concurrent_test_db");
        let relay_keys = Keys::generate();
        let db = Arc::new(RelayDatabase::new(&db_path, relay_keys).unwrap());

        let num_writer_tasks = 5;
        let num_events_per_writer = 10;
        let num_reader_tasks = 5;
        let num_reads_per_reader = 10;

        let mut mut_writer_handles = Vec::new();

        for i in 0..num_writer_tasks {
            let db_clone = Arc::clone(&db);
            let writer_keys = Keys::generate();
            let handle = tokio::spawn(async move {
                let scope = match i % 3 {
                    0 => Scope::Default,
                    1 => Scope::named("sub1").unwrap(),
                    _ => Scope::named("sub2").unwrap(),
                };
                for j in 0..num_events_per_writer {
                    // Create a unique event content/tag to identify it
                    let content = format!("writer_{}_event_{}", i, j);
                    let event = create_test_event(
                        &writer_keys,
                        Kind::TextNote.as_u16(),
                        vec![Tag::custom("content_id".into(), vec![&content])],
                    )
                    .await;
                    db_clone.save_event(&event, &scope).await.unwrap();
                }
            });
            mut_writer_handles.push(handle);
        }

        let mut mut_reader_handles = Vec::new();
        for i in 0..num_reader_tasks {
            let db_clone = Arc::clone(&db);
            let handle = tokio::spawn(async move {
                let scope = match i % 3 {
                    0 => Scope::Default,
                    1 => Scope::named("sub1").unwrap(),
                    _ => Scope::named("sub2").unwrap(),
                };
                for _ in 0..num_reads_per_reader {
                    // Query with a broad filter
                    let _events = db_clone.query(vec![Filter::new()], &scope).await.unwrap();
                    // In a real scenario with many events, we might check _events.len()
                    // but for this concurrency test, just ensuring no panics is key.
                }
            });
            mut_reader_handles.push(handle);
        }

        futures::future::join_all(mut_writer_handles)
            .await
            .into_iter()
            .for_each(|res| res.unwrap());
        futures::future::join_all(mut_reader_handles)
            .await
            .into_iter()
            .for_each(|res| res.unwrap());

        // With isolation, we need to check each subdomain separately
        // First, check how many events were actually stored in each subdomain
        let none_domain_events = db
            .query(vec![Filter::new()], &Scope::Default)
            .await
            .unwrap();
        let sub1_events = db
            .query(vec![Filter::new()], &Scope::named("sub1").unwrap())
            .await
            .unwrap();
        let sub2_events = db
            .query(vec![Filter::new()], &Scope::named("sub2").unwrap())
            .await
            .unwrap();

        // Print the actual counts to help debug
        println!("Events in None subdomain: {}", none_domain_events.len());
        println!("Events in sub1 subdomain: {}", sub1_events.len());
        println!("Events in sub2 subdomain: {}", sub2_events.len());

        // The total across all subdomains should match the expected total events
        let total_events = none_domain_events.len() + sub1_events.len() + sub2_events.len();
        let expected_total_events = num_writer_tasks * num_events_per_writer;
        assert_eq!(
            total_events, expected_total_events,
            "Total events across all subdomains should match expected count"
        );
    }

    #[tokio::test]
    async fn test_deletion_boundaries_single_db() {
        // Tests deletion operations with the scoped-heed model.
        // With the updated scoped-heed model, the `subdomain` parameter
        // properly isolates operations to a specific keyspace.

        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("deletion_test_db");
        let relay_keys = Keys::generate();
        let db = RelayDatabase::new(&db_path, relay_keys).unwrap();

        let keys_a = Keys::generate();
        let event_a = create_test_event(
            &keys_a,
            Kind::TextNote.as_u16(),
            vec![Tag::custom("id".into(), vec!["A"])],
        )
        .await;
        let keys_b = Keys::generate();
        let event_b = create_test_event(
            &keys_b,
            Kind::TextNote.as_u16(),
            vec![Tag::custom("id".into(), vec!["B"])],
        )
        .await;
        let keys_c = Keys::generate();
        let event_c = create_test_event(
            &keys_c,
            Kind::TextNote.as_u16(),
            vec![Tag::custom("id".into(), vec!["C"])],
        )
        .await;

        // Save initial events
        db.save_event(&event_a, &Scope::Default).await.unwrap(); // Subdomain: None
        db.save_event(&event_b, &Scope::named("s1").unwrap())
            .await
            .unwrap(); // Subdomain: s1
        db.save_event(&event_c, &Scope::named("s2").unwrap())
            .await
            .unwrap(); // Subdomain: s2

        // --- Test Case 1: Delete event_a (from None) using specific filter & None subdomain ---
        let filter_a = Filter::new().id(event_a.id);
        db.delete(filter_a.clone(), &Scope::Default).await.unwrap();

        let results_after_delete_a_none = db
            .query(vec![Filter::new().id(event_a.id)], &Scope::Default)
            .await
            .unwrap();
        assert!(
            results_after_delete_a_none.is_empty(),
            "Event A should be deleted from None scope"
        );
        let results_after_delete_a_s1 = db
            .query(
                vec![Filter::new().id(event_b.id)],
                &Scope::named("s1").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            results_after_delete_a_s1.len(),
            1,
            "Event B should still exist in s1 scope"
        );
        let results_after_delete_a_s2 = db
            .query(
                vec![Filter::new().id(event_c.id)],
                &Scope::named("s2").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            results_after_delete_a_s2.len(),
            1,
            "Event C should still exist in s2 scope"
        );

        // --- Test Case 2: Delete event_b (from s1) using specific filter & s1 subdomain ---
        let filter_b = Filter::new().id(event_b.id);
        db.delete(filter_b.clone(), &Scope::named("s1").unwrap())
            .await
            .unwrap();

        let results_after_delete_b_s1 = db
            .query(
                vec![Filter::new().id(event_b.id)],
                &Scope::named("s1").unwrap(),
            )
            .await
            .unwrap();
        assert!(
            results_after_delete_b_s1.is_empty(),
            "Event B should be deleted from s1 scope"
        );
        let results_after_delete_b_s2 = db
            .query(
                vec![Filter::new().id(event_c.id)],
                &Scope::named("s2").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            results_after_delete_b_s2.len(),
            1,
            "Event C should still exist in s2 scope after B deleted"
        );

        // --- Test Case 3: Broad filter, different subdomain ---
        // Re-save A and B for this test case
        db.save_event(&event_a, &Scope::Default).await.unwrap();
        db.save_event(&event_b, &Scope::named("s1").unwrap())
            .await
            .unwrap();
        // Event C is still in s2 from before

        // Delete all TextNote events, specifying subdomain "s2" for the delete operation.
        // With isolated operations, this should ONLY delete events in the s2 subdomain
        let broad_filter = Filter::new().kind(Kind::TextNote);
        db.delete(broad_filter.clone(), &Scope::named("s2").unwrap())
            .await
            .unwrap();

        let remaining_a = db
            .query(vec![Filter::new().id(event_a.id)], &Scope::Default)
            .await
            .unwrap();
        assert_eq!(
            remaining_a.len(),
            1,
            "Event A (None) should NOT be deleted by broad filter on s2 with proper isolation"
        );

        let remaining_b = db
            .query(
                vec![Filter::new().id(event_b.id)],
                &Scope::named("s1").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            remaining_b.len(),
            1,
            "Event B (s1) should NOT be deleted by broad filter on s2 with proper isolation"
        );

        let remaining_c = db
            .query(
                vec![Filter::new().id(event_c.id)],
                &Scope::named("s2").unwrap(),
            )
            .await
            .unwrap();
        assert!(
            remaining_c.is_empty(),
            "Event C (s2) should be deleted by broad filter on s2"
        );
    }

    #[tokio::test]
    async fn test_list_scopes_functionality() {
        // Create a temporary database
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("scopes_test_db");

        // Open the database
        let lmdb = NostrLMDB::open(&db_path).unwrap();

        // Create events in different scopes
        let event_keys = Keys::generate();

        // Create and save events to different scopes
        let default_event = create_test_event(&event_keys, Kind::TextNote.as_u16(), vec![]).await;
        let group1_event = create_test_event(&event_keys, Kind::TextNote.as_u16(), vec![]).await;
        let group2_event = create_test_event(&event_keys, Kind::TextNote.as_u16(), vec![]).await;
        let group3_event = create_test_event(&event_keys, Kind::TextNote.as_u16(), vec![]).await;

        // Save events to their respective scopes
        lmdb.scoped(&Scope::Default)
            .unwrap()
            .save_event(&default_event)
            .await
            .unwrap();
        lmdb.scoped(&Scope::named("group1").unwrap())
            .unwrap()
            .save_event(&group1_event)
            .await
            .unwrap();
        lmdb.scoped(&Scope::named("group2").unwrap())
            .unwrap()
            .save_event(&group2_event)
            .await
            .unwrap();
        lmdb.scoped(&Scope::named("group3").unwrap())
            .unwrap()
            .save_event(&group3_event)
            .await
            .unwrap();

        // Call list_scopes and verify it returns all scopes
        let scopes = lmdb.list_scopes().unwrap();

        // Convert to a HashSet for easier verification
        let scope_set: HashSet<String> = scopes
            .into_iter()
            .map(|s| match s {
                Scope::Default => "default".to_string(),
                Scope::Named { name, .. } => name,
            })
            .collect();

        // Verify all expected scopes are included
        assert!(
            scope_set.contains("default"),
            "Default scope should be included"
        );
        assert!(
            scope_set.contains("group1"),
            "group1 scope should be included"
        );
        assert!(
            scope_set.contains("group2"),
            "group2 scope should be included"
        );
        assert!(
            scope_set.contains("group3"),
            "group3 scope should be included"
        );

        // Verify the number of scopes is correct (4 = default + 3 named scopes)
        assert_eq!(scope_set.len(), 4, "Should have exactly 4 scopes");
    }
}
