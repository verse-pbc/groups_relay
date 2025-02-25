use crate::error::Error;
use crate::event_store_connection::StoreCommand;
use crate::nostr_database::RelayDatabase;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tracing::{debug, error};
use websocket_builder::{InboundContext, Middleware, SendMessage};

#[derive(Debug)]
pub struct Nip09Middleware {
    database: Arc<RelayDatabase>,
}

impl Nip09Middleware {
    pub fn new(database: Arc<RelayDatabase>) -> Self {
        Self { database }
    }

    async fn handle_deletion_request(&self, event: &Event) -> Result<(), Error> {
        // Only process kind 5 (deletion request) events
        if event.kind != Kind::EventDeletion {
            return Ok(());
        }

        debug!(
            target: "nip09",
            "Processing deletion request from {}: {}",
            event.pubkey,
            event.id
        );

        // First save the deletion request event itself
        self.database
            .save_signed_event(event.clone())
            .await
            .map_err(|e| Error::notice(format!("Failed to save deletion request: {}", e)))?;

        // Process 'e' tags (direct event references) and 'a' tags (addresses)
        for tag in event.tags.iter() {
            match tag.kind() {
                k if k == TagKind::e() => {
                    self.handle_event_deletion(event, tag).await?;
                }
                k if k == TagKind::a() => {
                    self.handle_address_deletion(event, tag).await?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_event_deletion(&self, event: &Event, tag: &Tag) -> Result<(), Error> {
        if let [_, event_id, ..] = tag.as_slice() {
            if let Ok(event_id) = EventId::parse(event_id) {
                debug!(
                    target: "nip09",
                    "Checking event {} for deletion request {}",
                    event_id,
                    event.id
                );

                // First query the event to check ownership
                let filter = Filter::new().id(event_id);
                let events = self
                    .database
                    .query(vec![filter.clone()])
                    .await
                    .map_err(|e| Error::notice(format!("Failed to query event: {}", e)))?;

                if let Some(target_event) = events.first() {
                    // Only allow deletion if the event was created by the same pubkey
                    if target_event.pubkey == event.pubkey {
                        debug!(
                            target: "nip09",
                            "Deleting event {} referenced by deletion request {}",
                            event_id,
                            event.id
                        );

                        let delete_command = StoreCommand::DeleteEvents(filter);

                        drop(
                            self.database
                                .save_store_command(delete_command)
                                .await
                                .map_err(|e| {
                                    Error::notice(format!("Failed to delete event: {}", e))
                                })?,
                        );
                    } else {
                        debug!(
                            target: "nip09",
                            "Skipping deletion of event {} - different pubkey",
                            event_id
                        );
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_address_deletion(&self, event: &Event, tag: &Tag) -> Result<(), Error> {
        if let [_, addr, ..] = tag.as_slice() {
            debug!(
                target: "nip09",
                "Processing address deletion {} referenced by deletion request {}",
                addr,
                event.id
            );

            // Parse the address format: <kind>:<pubkey>:<d-tag>
            let parts: Vec<&str> = addr.split(':').collect();
            if parts.len() == 3 {
                if let (Ok(kind), Ok(pubkey), d_tag) = (
                    parts[0].parse::<u64>(),
                    PublicKey::parse(parts[1]),
                    parts[2],
                ) {
                    // Only allow deletion if the pubkey matches
                    if pubkey == event.pubkey {
                        let filter = Filter::new()
                            .kind(Kind::Custom(kind as u16))
                            .author(pubkey)
                            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), d_tag);

                        let delete_command = StoreCommand::DeleteEvents(filter);

                        drop(
                            self.database
                                .save_store_command(delete_command)
                                .await
                                .map_err(|e| {
                                    Error::notice(format!(
                                        "Failed to delete events by address: {}",
                                        e
                                    ))
                                })?,
                        );
                    } else {
                        debug!(
                            target: "nip09",
                            "Skipping deletion of address {} - different pubkey",
                            addr
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Middleware for Nip09Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let ClientMessage::Event(event) = &ctx.message else {
            return ctx.next().await;
        };

        if event.kind == Kind::EventDeletion {
            if let Err(e) = self.handle_deletion_request(event).await {
                error!(
                    target: "nip09",
                    "Failed to process deletion request {}: {}",
                    event.id,
                    e
                );
                return ctx
                    .send_message(RelayMessage::ok(
                        event.id,
                        false,
                        format!("Failed to process deletion request: {}", e),
                    ))
                    .await;
            }

            // Send OK for the deletion request itself
            return ctx
                .send_message(RelayMessage::ok(
                    event.id,
                    true,
                    "Deletion request processed successfully",
                ))
                .await;
        }

        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, setup_test};
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_delete_event_with_matching_pubkey() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware = Nip09Middleware::new(database.clone());

        // Create and save an event
        let event_to_delete = create_test_event(&keys, 1, vec![]).await;
        database
            .save_signed_event(event_to_delete.clone())
            .await
            .unwrap();

        sleep(Duration::from_millis(30)).await;

        // Create deletion request
        let deletion_request =
            create_test_event(&keys, 5, vec![Tag::event(event_to_delete.id)]).await;

        // Process deletion request
        let mut state = NostrConnectionState::default();
        let mut ctx = InboundContext::new(
            "test".to_string(),
            ClientMessage::Event(Box::new(deletion_request.clone())),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify event is deleted
        sleep(Duration::from_millis(30)).await; // Give time for async deletion
        let filter = Filter::new().id(event_to_delete.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert!(events.is_empty(), "Event should have been deleted");

        // Verify deletion request is saved
        let filter = Filter::new().id(deletion_request.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert_eq!(events.len(), 1, "Deletion request should be saved");
    }

    #[tokio::test]
    async fn test_delete_event_with_different_pubkey() {
        let (_tmp_dir, database, keys1) = setup_test().await;
        let keys2 = Keys::generate();
        let middleware = Nip09Middleware::new(database.clone());

        // Create and save an event from keys2
        let event_to_delete = create_test_event(&keys2, 1, vec![]).await;
        database
            .save_signed_event(event_to_delete.clone())
            .await
            .unwrap();

        // Create deletion request from keys1
        let deletion_request =
            create_test_event(&keys1, 5, vec![Tag::event(event_to_delete.id)]).await;

        // Process deletion request
        let mut state = NostrConnectionState::default();
        let mut ctx = InboundContext::new(
            "test".to_string(),
            ClientMessage::Event(Box::new(deletion_request.clone())),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify event is not deleted
        sleep(Duration::from_millis(30)).await;
        let filter = Filter::new().id(event_to_delete.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert_eq!(events.len(), 1, "Event should not have been deleted");
    }

    #[tokio::test]
    async fn test_delete_replaceable_event() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware = Nip09Middleware::new(database.clone());

        // Create and save a replaceable event with a 'd' tag
        let replaceable_event = create_test_event(
            &keys,
            10002, // Some replaceable event kind
            vec![Tag::parse(vec!["d", "test"]).unwrap()],
        )
        .await;
        database
            .save_signed_event(replaceable_event.clone())
            .await
            .unwrap();

        // Create deletion request with 'a' tag for the replaceable event
        let addr = format!("10002:{}:test", keys.public_key());
        let deletion_request =
            create_test_event(&keys, 5, vec![Tag::parse(vec!["a", &addr]).unwrap()]).await;

        // Process deletion request
        let mut state = NostrConnectionState::default();
        let mut ctx = InboundContext::new(
            "test".to_string(),
            ClientMessage::Event(Box::new(deletion_request.clone())),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify replaceable event is deleted
        sleep(Duration::from_millis(30)).await;
        let filter = Filter::new().id(replaceable_event.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert!(
            events.is_empty(),
            "Replaceable event should have been deleted"
        );
    }

    #[tokio::test]
    async fn test_process_inbound_with_identifier_tag() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware = Nip09Middleware::new(database.clone());

        // Create and save an event with a 'd' tag
        let event = create_test_event(&keys, 5, vec![Tag::parse(vec!["d", "test"]).unwrap()]).await;
        database.save_signed_event(event.clone()).await.unwrap();

        sleep(Duration::from_millis(30)).await;

        // Process the event
        let mut state = NostrConnectionState::default();
        let mut ctx = InboundContext::new(
            "test".to_string(),
            ClientMessage::Event(Box::new(event.clone())),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify event is saved
        let filter = Filter::new().id(event.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert_eq!(events.len(), 1, "Event should have been saved");
    }

    #[tokio::test]
    async fn test_process_inbound_with_address_tag() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware = Nip09Middleware::new(database.clone());

        // Create and save a replaceable event first
        let replaceable_event =
            create_test_event(&keys, 10002, vec![Tag::parse(vec!["d", "test"]).unwrap()]).await;
        database
            .save_signed_event(replaceable_event.clone())
            .await
            .unwrap();

        sleep(Duration::from_millis(30)).await;

        // Create and process deletion event with 'a' tag
        let addr = format!("10002:{}:test", keys.public_key());
        let deletion_event =
            create_test_event(&keys, 5, vec![Tag::parse(vec!["a", &addr]).unwrap()]).await;

        let mut state = NostrConnectionState::default();
        let mut ctx = InboundContext::new(
            "test".to_string(),
            ClientMessage::Event(Box::new(deletion_event.clone())),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        sleep(Duration::from_millis(30)).await;

        // Verify deletion event is saved
        let filter = Filter::new().id(deletion_event.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert_eq!(events.len(), 1, "Deletion event should have been saved");

        // Verify replaceable event is deleted
        sleep(Duration::from_millis(30)).await;
        let filter = Filter::new().id(replaceable_event.id);
        let events = database.query(vec![filter]).await.unwrap();
        assert!(
            events.is_empty(),
            "Replaceable event should have been deleted"
        );
    }
}
