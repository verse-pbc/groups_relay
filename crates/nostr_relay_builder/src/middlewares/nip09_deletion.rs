//! NIP-09: Event Deletion middleware

use crate::database::RelayDatabase;
use crate::error::Error;
use crate::state::NostrConnectionState;
use crate::subscription_service::StoreCommand;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tracing::{debug, error};
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

/// Middleware that implements NIP-09: Event Deletion
///
/// This middleware handles deletion requests (kind 5 events) and removes
/// events from the database when the deletion request is from the same
/// pubkey that created the original event.
#[derive(Debug)]
pub struct Nip09Middleware {
    database: Arc<RelayDatabase>,
}

impl Nip09Middleware {
    pub fn new(database: Arc<RelayDatabase>) -> Self {
        Self { database }
    }

    async fn handle_deletion_request(
        &self,
        event: &Event,
        state: &NostrConnectionState,
    ) -> Result<(), Error> {
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
            .save_signed_event(event.clone(), state.subdomain().clone())
            .await?;

        // Process 'e' tags (direct event references) and 'a' tags (addresses)
        for tag in event.tags.iter() {
            match tag.kind() {
                k if k == TagKind::e() => {
                    self.handle_event_deletion(event, tag, state).await?;
                }
                k if k == TagKind::a() => {
                    self.handle_address_deletion(event, tag, state).await?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn handle_event_deletion(
        &self,
        event: &Event,
        tag: &Tag,
        state: &NostrConnectionState,
    ) -> Result<(), Error> {
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
                let target_events = self
                    .database
                    .query(vec![filter.clone()], state.subdomain())
                    .await?;

                if let Some(target_event) = target_events.first() {
                    // Only allow deletion if the event was created by the same pubkey
                    if target_event.pubkey == event.pubkey {
                        debug!(
                            target: "nip09",
                            "User {} is authorized to delete event {}",
                            event.pubkey,
                            event_id
                        );

                        let delete_command =
                            StoreCommand::DeleteEvents(filter, state.subdomain().clone());

                        state.save_and_broadcast(delete_command).await?
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

    async fn handle_address_deletion(
        &self,
        event: &Event,
        tag: &Tag,
        state: &NostrConnectionState,
    ) -> Result<(), Error> {
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

                        let delete_command =
                            StoreCommand::DeleteEvents(filter, state.subdomain().clone());

                        state.save_and_broadcast(delete_command).await?
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
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(ClientMessage::Event(event_cow)) = &ctx.message else {
            return ctx.next().await;
        };

        if event_cow.kind == Kind::EventDeletion {
            if let Err(e) = self
                .handle_deletion_request(event_cow.as_ref(), ctx.state)
                .await
            {
                error!(
                    target: "nip09",
                    "Failed to process deletion request {}: {}",
                    event_cow.id,
                    e
                );
                return ctx.send_message(RelayMessage::ok(
                    event_cow.id,
                    false,
                    format!("Failed to process deletion request: {}", e),
                ));
            }

            return ctx.send_message(RelayMessage::ok(
                event_cow.id,
                true,
                "Deletion request processed successfully",
            ));
        }

        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{
        create_test_event, create_test_state_with_subscription_service, setup_test,
    };
    use nostr_lmdb::Scope;
    use std::borrow::Cow;
    use std::time::Duration;
    use tokio::time::sleep;
    use websocket_builder::{InboundContext, Middleware};

    #[tokio::test]
    async fn test_delete_event_with_matching_pubkey() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware = Nip09Middleware::new(database.clone());

        // Create and save an event
        let event_to_delete = create_test_event(&keys, 1, vec![]).await;
        database
            .save_signed_event(event_to_delete.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(Duration::from_millis(30)).await;

        // Create deletion request
        let deletion_request =
            create_test_event(&keys, 5, vec![Tag::event(event_to_delete.id)]).await;

        // Process deletion request
        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let mut ctx = InboundContext::new(
            "test".to_string(),
            Some(ClientMessage::Event(Cow::Owned(deletion_request.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify event is deleted
        sleep(Duration::from_millis(30)).await; // Give time for async deletion
        let filter = Filter::new().id(event_to_delete.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert!(events.is_empty(), "Event should have been deleted");

        // Verify deletion request is saved
        let filter = Filter::new().id(deletion_request.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
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
            .save_signed_event(event_to_delete.clone(), Scope::Default)
            .await
            .unwrap();

        // Create deletion request from keys1
        let deletion_request =
            create_test_event(&keys1, 5, vec![Tag::event(event_to_delete.id)]).await;

        // Process deletion request
        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let mut ctx = InboundContext::new(
            "test".to_string(),
            Some(ClientMessage::Event(Cow::Owned(deletion_request.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify event is not deleted
        sleep(Duration::from_millis(30)).await;
        let filter = Filter::new().id(event_to_delete.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
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
            .save_signed_event(replaceable_event.clone(), Scope::Default)
            .await
            .unwrap();

        // Create deletion request with 'a' tag for the replaceable event
        let addr = format!("10002:{}:test", keys.public_key());
        let deletion_request =
            create_test_event(&keys, 5, vec![Tag::parse(vec!["a", &addr]).unwrap()]).await;

        // Process deletion request
        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let mut ctx = InboundContext::new(
            "test".to_string(),
            Some(ClientMessage::Event(Cow::Owned(deletion_request.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify replaceable event is deleted - add longer wait time
        sleep(Duration::from_millis(200)).await;
        let filter = Filter::new().id(replaceable_event.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
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
        database
            .save_signed_event(event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(Duration::from_millis(30)).await;

        // Process the event
        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let mut ctx = InboundContext::new(
            "test".to_string(),
            Some(ClientMessage::Event(Cow::Owned(event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        // Verify event is saved
        let filter = Filter::new().id(event.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
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
            .save_signed_event(replaceable_event.clone(), Scope::Default)
            .await
            .unwrap();

        sleep(Duration::from_millis(30)).await;

        // Create and process deletion event with 'a' tag
        let addr = format!("10002:{}:test", keys.public_key());
        let deletion_event =
            create_test_event(&keys, 5, vec![Tag::parse(vec!["a", &addr]).unwrap()]).await;

        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let mut ctx = InboundContext::new(
            "test".to_string(),
            Some(ClientMessage::Event(Cow::Owned(deletion_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        middleware.process_inbound(&mut ctx).await.unwrap();

        sleep(Duration::from_millis(30)).await;

        // Verify deletion event is saved
        let filter = Filter::new().id(deletion_event.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert_eq!(events.len(), 1, "Deletion event should have been saved");

        // Verify replaceable event is deleted - add longer wait time
        sleep(Duration::from_millis(200)).await;
        let filter = Filter::new().id(replaceable_event.id);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();
        assert!(
            events.is_empty(),
            "Replaceable event should have been deleted"
        );
    }
}
