use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

#[derive(Debug)]
pub struct EventVerifierMiddleware;

impl EventVerifierMiddleware {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EventVerifierMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Middleware for EventVerifierMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        if let ClientMessage::Event(event) = &ctx.message {
            if event.verify().is_err() {
                ctx.send_message(RelayMessage::ok(
                    event.id,
                    false,
                    "invalid: event signature verification failed",
                ))
                .await?;
                return Ok(());
            }
        }
        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_middleware_chain() -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage,
                OutgoingMessage = RelayMessage,
            >,
        >,
    > {
        vec![Arc::new(EventVerifierMiddleware::new())]
    }

    async fn create_signed_event() -> (Keys, Event) {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test message").build(keys.public_key());
        let event = keys.sign_event(event).await.expect("Failed to sign event");
        (keys, event)
    }

    fn create_test_state() -> NostrConnectionState {
        NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL")
    }

    #[tokio::test]
    async fn test_valid_event_signature() {
        let chain = create_middleware_chain();
        let mut state = create_test_state();
        let (_, event) = create_signed_event().await;

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            ClientMessage::Event(Box::new(event)),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_invalid_event_signature() {
        let chain = create_middleware_chain();
        let mut state = create_test_state();

        // Create an event with one key
        let (_, mut event) = create_signed_event().await;

        // Create another event with different keys
        let keys2 = Keys::generate();
        let event2 = EventBuilder::text_note("other message").build(keys2.public_key());
        let event2 = keys2
            .sign_event(event2)
            .await
            .expect("Failed to sign event");

        // Use the signature from the second event with the first event
        event.sig = event2.sig;

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            ClientMessage::Event(Box::new(event)),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok()); // Should be ok because we send an error message instead of returning an error
    }

    #[tokio::test]
    async fn test_non_event_message_passes_through() {
        let chain = create_middleware_chain();
        let mut state = create_test_state();

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            ClientMessage::Close(SubscriptionId::new("test_sub")),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_auth_message_passes_through() {
        let chain = create_middleware_chain();
        let mut state = create_test_state();
        let (_, auth_event) = create_signed_event().await;

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            ClientMessage::Auth(Box::new(auth_event)),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }
}
