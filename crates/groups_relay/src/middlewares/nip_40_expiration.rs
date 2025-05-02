use crate::nostr_database::RelayDatabase;
use crate::nostr_session_state::NostrConnectionState;
use crate::subscription_manager::StoreCommand;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tracing::{debug, warn};
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

/// Middleware to handle NIP-40 expiration tags.
///
/// It checks incoming `EVENT` messages for an `expiration` tag.
/// If the tag exists and the timestamp is in the past, the event is dropped
/// and an `OK: false` message is sent back. On the outbound side, it filters
/// out events that have expired and queues them for lazy deletion.
#[derive(Debug, Clone)]
pub struct Nip40Middleware {
    database: Arc<RelayDatabase>,
}

impl Nip40Middleware {
    pub fn new(database: Arc<RelayDatabase>) -> Self {
        Self { database }
    }

    fn check_expiration(&self, event: &Event) -> Result<Option<Timestamp>, RelayMessage> {
        if let Some(tag) = event.tags.iter().find(|t| t.kind() == TagKind::Expiration) {
            if let Some(timestamp_str) = tag.content() {
                if let Ok(timestamp_secs) = timestamp_str.parse::<u64>() {
                    let timestamp = Timestamp::from(timestamp_secs);
                    let now = Timestamp::now();
                    if timestamp < now {
                        debug!(
                            target: "nip40",
                            event_id = %event.id,
                            pubkey = %event.pubkey,
                            expiration = %timestamp.as_u64(),
                            "Dropping expired event"
                        );
                        Err(RelayMessage::ok(event.id, false, "event is expired"))
                    } else {
                        Ok(Some(timestamp))
                    }
                } else {
                    warn!(
                        target: "nip40",
                        event_id = %event.id,
                        tag_content = %timestamp_str,
                        "Failed to parse expiration timestamp value"
                    );
                    Err(RelayMessage::ok(
                        event.id,
                        false,
                        "invalid expiration tag format",
                    ))
                }
            } else {
                warn!(
                    target: "nip40",
                    event_id = %event.id,
                    "Expiration tag found without content"
                );
                Err(RelayMessage::ok(
                    event.id,
                    false,
                    "invalid expiration tag: missing content",
                ))
            }
        } else {
            Ok(None)
        }
    }
}

#[async_trait]
impl Middleware for Nip40Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(ClientMessage::Event(event)) = &ctx.message else {
            return ctx.next().await;
        };

        match self.check_expiration(event) {
            Ok(expiration_status) => {
                if expiration_status.is_some() {
                    debug!(
                        target: "nip40",
                        event_id = %event.id,
                        pubkey = %event.pubkey,
                        "Event has a valid future expiration tag"
                    );
                }
                ctx.next().await
            }
            Err(response_message) => ctx.send_message(response_message).await,
        }
    }

    /// Filters outgoing event messages, dropping events that have expired.
    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let Some(RelayMessage::Event { ref event, .. }) = ctx.message else {
            return ctx.next().await;
        };

        if let Some(tag) = event.tags.iter().find(|t| t.kind() == TagKind::Expiration) {
            if let Some(timestamp_str) = tag.content() {
                if let Ok(timestamp_secs) = timestamp_str.parse::<u64>() {
                    let timestamp = Timestamp::from(timestamp_secs);
                    let now = Timestamp::now();
                    if timestamp < now {
                        debug!(
                            target: "nip40",
                            event_id = %event.id,
                            pubkey = %event.pubkey,
                            expiration = %timestamp.as_u64(),
                            "Filtering expired outgoing event"
                        );

                        let filter = Filter::new().id(event.id);
                        let delete_command = StoreCommand::DeleteEvents(filter);

                        if let Err(e) = self.database.save_store_command(delete_command).await {
                            warn!(
                                target: "nip40_lazy_delete",
                                event_id = %event.id,
                                "Failed to queue lazy deletion for expired event: {}", e
                            );
                        } else {
                            debug!(
                                target: "nip40_lazy_delete",
                                event_id = %event.id,
                                "Queued lazy deletion for expired event"
                            );
                        }

                        ctx.message = None;
                        return Ok(());
                    }
                } else {
                    warn!(
                        target: "nip40",
                        event_id = %event.id,
                        tag_content = %timestamp_str,
                        "Ignoring invalid expiration timestamp format on outgoing event"
                    );
                }
            } else {
                warn!(
                    target: "nip40",
                    event_id = %event.id,
                    "Ignoring expiration tag without content on outgoing event"
                );
            }
        }

        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, create_test_state, setup_test};
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::sync::mpsc;
    use websocket_builder::{InboundContext, Middleware, OutboundContext};

    fn get_timestamp(offset_secs: i64) -> Timestamp {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();
        Timestamp::from(now.saturating_add_signed(offset_secs))
    }

    async fn create_expiring_event(keys: &Keys, _content: &str, offset_secs: i64) -> Box<Event> {
        let expiration_tag = Tag::expiration(get_timestamp(offset_secs));
        Box::new(create_test_event(keys, Kind::TextNote.as_u16(), vec![expiration_tag]).await)
    }

    async fn create_non_expiring_event(keys: &Keys, _content: &str) -> Box<Event> {
        Box::new(create_test_event(keys, Kind::TextNote.as_u16(), vec![]).await)
    }

    fn setup_test_context<'a>(
        message: Option<ClientMessage>,
        sender_channel: Option<mpsc::Sender<(RelayMessage, usize)>>,
        state: &'a mut NostrConnectionState,
        middlewares: &'a [Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage,
                OutgoingMessage = RelayMessage,
            >,
        >],
        current_middleware_index: usize,
    ) -> InboundContext<'a, NostrConnectionState, ClientMessage, RelayMessage> {
        InboundContext::<'a, NostrConnectionState, ClientMessage, RelayMessage>::new(
            "test_conn".to_string(),
            message,
            sender_channel,
            state,
            middlewares,
            current_middleware_index,
        )
    }

    fn setup_outbound_test_context<'a>(
        message: RelayMessage,
        sender_channel: Option<mpsc::Sender<(RelayMessage, usize)>>,
        state: &'a mut NostrConnectionState,
        middlewares: &'a [Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage,
                OutgoingMessage = RelayMessage,
            >,
        >],
        current_middleware_index: usize,
    ) -> OutboundContext<'a, NostrConnectionState, ClientMessage, RelayMessage> {
        OutboundContext::<'a, NostrConnectionState, ClientMessage, RelayMessage>::new(
            "test_conn".to_string(),
            message,
            sender_channel,
            state,
            middlewares,
            current_middleware_index,
        )
    }

    #[tokio::test]
    async fn test_inbound_event_without_expiration_passes() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware_under_test = Nip40Middleware::new(database.clone());
        let event = create_non_expiring_event(&keys, "test").await;
        let message = ClientMessage::Event(event);

        let (sender, mut receiver) = mpsc::channel(10);
        let mut state = create_test_state(None);

        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];

        let mut context = setup_test_context(
            Some(message),
            Some(sender.clone()),
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware_under_test.process_inbound(&mut context).await;
        assert!(result.is_ok());

        assert!(
            receiver.try_recv().is_err(),
            "No message should be sent back by the middleware"
        );
    }

    #[tokio::test]
    async fn test_inbound_event_with_future_expiration_passes() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware_under_test = Nip40Middleware::new(database.clone());
        let event = create_expiring_event(&keys, "future", 3600).await;
        let message = ClientMessage::Event(event);

        let (sender, mut receiver) = mpsc::channel(10);
        let mut state = create_test_state(None);

        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];

        let mut context = setup_test_context(
            Some(message),
            Some(sender.clone()),
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware_under_test.process_inbound(&mut context).await;
        assert!(result.is_ok());
        assert!(receiver.try_recv().is_err(), "No message should be sent");
    }

    #[tokio::test]
    async fn test_inbound_event_with_past_expiration_is_dropped() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware_under_test = Nip40Middleware::new(database.clone());
        let event = create_expiring_event(&keys, "past", -3600).await;
        let event_id = event.id;
        let message = ClientMessage::Event(event);

        let (sender, mut receiver) = mpsc::channel(10);
        let mut state = create_test_state(None);

        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];

        let mut context = setup_test_context(
            Some(message),
            Some(sender.clone()),
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware_under_test.process_inbound(&mut context).await;
        assert!(result.is_ok());

        let response = receiver.recv().await.unwrap();
        match response.0 {
            RelayMessage::Ok {
                event_id: resp_id,
                status,
                message,
            } => {
                assert_eq!(resp_id, event_id);
                assert!(!status);
                assert!(message.contains("expired"));
            }
            _ => panic!("Expected OK message, got {:?}", response),
        }
    }

    #[tokio::test]
    async fn test_inbound_non_event_message_passes() {
        let (_tmp_dir, database, _keys) = setup_test().await;
        let middleware_under_test = Nip40Middleware::new(database.clone());
        let req_filter = Filter::new().kind(Kind::TextNote).limit(10);
        let message = ClientMessage::req(SubscriptionId::generate(), req_filter);

        let (sender, mut receiver) = mpsc::channel(10);
        let mut state = create_test_state(None);

        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];

        let mut context = setup_test_context(
            Some(message),
            Some(sender.clone()),
            &mut state,
            &middlewares,
            0,
        );

        let result = middleware_under_test.process_inbound(&mut context).await;
        assert!(result.is_ok());
        assert!(receiver.try_recv().is_err(), "No message should be sent");
    }

    #[tokio::test]
    async fn test_outbound_filters_expired_event() {
        let (_tmp_dir, database, keys) = setup_test().await;
        let middleware_under_test = Nip40Middleware::new(database.clone());

        let event_valid = create_non_expiring_event(&keys, "valid").await;
        database
            .save_signed_event(*event_valid.clone())
            .await
            .unwrap();

        let event_expired = create_expiring_event(&keys, "expired", -3600).await;
        database
            .save_signed_event(*event_expired.clone())
            .await
            .unwrap();

        let event_future = create_expiring_event(&keys, "future", 3600).await;
        database
            .save_signed_event(*event_future.clone())
            .await
            .unwrap();

        let (sender, _receiver) = mpsc::channel(10);
        let mut state = create_test_state(None);

        let message_expired = RelayMessage::event(SubscriptionId::generate(), *event_expired);
        let middlewares_expired: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];
        let mut context_expired = setup_outbound_test_context(
            message_expired,
            Some(sender.clone()),
            &mut state,
            &middlewares_expired,
            0,
        );
        let result_expired = middleware_under_test
            .process_outbound(&mut context_expired)
            .await;
        assert!(result_expired.is_ok());

        assert!(
            context_expired.message.is_none(),
            "ctx.message should be None after filtering an expired event"
        );

        let message_valid = RelayMessage::event(SubscriptionId::generate(), *event_valid.clone());
        let middlewares_valid: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];
        let mut context_valid = setup_outbound_test_context(
            message_valid,
            Some(sender.clone()),
            &mut state,
            &middlewares_valid,
            0,
        );
        let original_valid_message = context_valid.message.clone();

        let result_valid = middleware_under_test
            .process_outbound(&mut context_valid)
            .await;
        assert!(result_valid.is_ok());

        assert!(
            context_valid.message.is_some(),
            "ctx.message should NOT be None for a valid event"
        );
        assert_eq!(
            context_valid.message, original_valid_message,
            "Message content should be unchanged for valid event"
        );

        let message_future = RelayMessage::event(SubscriptionId::generate(), *event_future);
        let middlewares_future: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];
        let mut context_future = setup_outbound_test_context(
            message_future,
            Some(sender.clone()),
            &mut state,
            &middlewares_future,
            0,
        );
        let original_future_message = context_future.message.clone();

        let result_future = middleware_under_test
            .process_outbound(&mut context_future)
            .await;
        assert!(result_future.is_ok());

        assert!(
            context_future.message.is_some(),
            "ctx.message should NOT be None for a future-expiring event"
        );
        assert_eq!(
            context_future.message, original_future_message,
            "Message content should be unchanged for future event"
        );
    }

    #[tokio::test]
    async fn test_outbound_non_event_message_passes() {
        let (_tmp_dir, database, _keys) = setup_test().await;
        let middleware_under_test = Nip40Middleware::new(database.clone());
        let message = RelayMessage::notice("Test notice");

        let (sender, _receiver) = mpsc::channel(10);
        let mut state = create_test_state(None);

        let middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage,
                    OutgoingMessage = RelayMessage,
                >,
            >,
        > = vec![Arc::new(middleware_under_test.clone())];

        let mut context =
            setup_outbound_test_context(message, Some(sender.clone()), &mut state, &middlewares, 0);

        let result = middleware_under_test.process_outbound(&mut context).await;
        assert!(result.is_ok());

        assert!(
            context.message.is_some(),
            "ctx.message should NOT be None for non-event message"
        );
    }
}
