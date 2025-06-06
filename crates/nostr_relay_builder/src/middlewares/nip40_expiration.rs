//! NIP-40: Expiration Timestamp middleware

use crate::state::NostrConnectionState;
use crate::subscription_service::StoreCommand;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::{error, warn};
use websocket_builder::{InboundContext, Middleware, OutboundContext};

// Helper function to get expiration timestamp from event tags
fn get_event_expiration(event: &Event) -> Option<Timestamp> {
    event.tags.iter().find_map(|tag| {
        if tag.kind() == TagKind::Expiration {
            tag.content()
                .and_then(|s| s.parse::<u64>().ok().map(Timestamp::from))
        } else {
            None
        }
    })
}

/// Middleware to handle NIP-40 expiration tags.
///
/// It checks incoming `EVENT` messages for an `expiration` tag.
/// If the tag exists and the timestamp is in the past, the event is dropped
/// and an `OK: false` message is sent back. On the outbound side, it filters
/// out events that have expired and queues them for lazy deletion.
#[derive(Debug, Clone)]
pub struct Nip40ExpirationMiddleware;

impl Default for Nip40ExpirationMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl Nip40ExpirationMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Middleware for Nip40ExpirationMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> anyhow::Result<()> {
        if let Some(ClientMessage::Event(event_cow)) = &ctx.message {
            let event_ref: &Event = event_cow.as_ref();
            if let Some(expiration) = get_event_expiration(event_ref) {
                if expiration < Timestamp::now() {
                    warn!(
                        target: "nip40",
                        "Event {} (kind {}) with expiration {} is expired. Publisher: {}.",
                        event_ref.id, event_ref.kind, expiration, event_ref.pubkey
                    );

                    let filter = Filter::new().id(event_ref.id);
                    let delete_command =
                        StoreCommand::DeleteEvents(filter, ctx.state.subdomain().clone());

                    if let Err(e) = ctx.state.save_and_broadcast(delete_command).await {
                        error!(
                            target: "nip40",
                            "Failed to send delete command for expired event {}: {}", event_ref.id, e
                        );
                    }
                }
            }
        }
        ctx.next().await
    }

    /// Filters outgoing event messages, dropping events that have expired.
    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> anyhow::Result<()> {
        if let Some(RelayMessage::Event { event, .. }) = &mut ctx.message {
            let event_ref: &Event = event.as_ref();
            if let Some(expiration) = get_event_expiration(event_ref) {
                if expiration < Timestamp::now() {
                    warn!(
                        target: "nip40",
                        "Dropping expired event {} (kind {}) with expiration {} from outbound. Publisher: {}.",
                        event_ref.id, event_ref.kind, expiration, event_ref.pubkey
                    );
                    ctx.message = None;
                }
            }
        }
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_state_with_subscription_service, setup_test};
    use nostr_lmdb::Scope;
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::time::sleep;
    use websocket_builder::{InboundContext, Middleware};

    #[tokio::test]
    async fn test_expired_event_is_deleted() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let middleware = Nip40ExpirationMiddleware::new();

        let unsigned_expired_event = EventBuilder::new(Kind::Custom(1234), "expired content")
            .tags([Tag::expiration(
                Timestamp::now() - Duration::from_secs(3600),
            )])
            .build(admin_keys.public_key());
        let expired_event = admin_keys.sign_event(unsigned_expired_event).await.unwrap();

        database
            .save_signed_event(expired_event.clone(), Scope::Default)
            .await
            .unwrap();

        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage<'static>,
                    OutgoingMessage = RelayMessage<'static>,
                >,
            >,
        > = vec![Arc::new(middleware.clone())];

        let mut context = InboundContext::new(
            "test_connection_id".to_string(),
            Some(ClientMessage::Event(Cow::Owned(expired_event.clone()))),
            None,
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware.process_inbound(&mut context).await;
        assert!(result.is_ok());

        sleep(Duration::from_millis(100)).await;
        let events = database
            .query(vec![Filter::new().id(expired_event.id)], &Scope::Default)
            .await
            .unwrap();
        assert!(events.is_empty(), "Expired event should be deleted");
    }

    #[tokio::test]
    async fn test_non_expired_event_is_not_deleted() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let middleware = Nip40ExpirationMiddleware::new();

        let unsigned_non_expired_event =
            EventBuilder::new(Kind::Custom(1234), "non-expired content")
                .tags([Tag::expiration(
                    Timestamp::now() + Duration::from_secs(3600),
                )])
                .build(admin_keys.public_key());
        let non_expired_event = admin_keys
            .sign_event(unsigned_non_expired_event)
            .await
            .unwrap();

        database
            .save_signed_event(non_expired_event.clone(), Scope::Default)
            .await
            .unwrap();

        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage<'static>,
                    OutgoingMessage = RelayMessage<'static>,
                >,
            >,
        > = vec![Arc::new(middleware.clone())];

        let mut context = InboundContext::new(
            "test_connection_id".to_string(),
            Some(ClientMessage::Event(Cow::Owned(non_expired_event.clone()))),
            None,
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware.process_inbound(&mut context).await;
        assert!(result.is_ok());

        sleep(Duration::from_millis(100)).await;
        let events = database
            .query(
                vec![Filter::new().id(non_expired_event.id)],
                &Scope::Default,
            )
            .await
            .unwrap();
        assert_eq!(events.len(), 1, "Non-expired event should not be deleted");
    }

    #[tokio::test]
    async fn test_event_without_expiration_tag_is_not_deleted() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let middleware = Nip40ExpirationMiddleware::new();

        let unsigned_no_expiration_event =
            EventBuilder::new(Kind::Custom(1234), "no expiration tag")
                .build(admin_keys.public_key());
        let no_expiration_event = admin_keys
            .sign_event(unsigned_no_expiration_event)
            .await
            .unwrap();

        database
            .save_signed_event(no_expiration_event.clone(), Scope::Default)
            .await
            .unwrap();

        let (mut state, _rx) =
            create_test_state_with_subscription_service(None, database.clone()).await;
        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage<'static>,
                    OutgoingMessage = RelayMessage<'static>,
                >,
            >,
        > = vec![Arc::new(middleware.clone())];

        let mut context = InboundContext::new(
            "test_connection_id".to_string(),
            Some(ClientMessage::Event(Cow::Owned(
                no_expiration_event.clone(),
            ))),
            None,
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware.process_inbound(&mut context).await;
        assert!(result.is_ok());

        sleep(Duration::from_millis(100)).await;
        let events = database
            .query(
                vec![Filter::new().id(no_expiration_event.id)],
                &Scope::Default,
            )
            .await
            .unwrap();
        assert_eq!(
            events.len(),
            1,
            "Event without expiration tag should not be deleted"
        );
    }
}
