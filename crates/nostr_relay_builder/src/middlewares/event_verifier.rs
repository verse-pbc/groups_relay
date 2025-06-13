//! Event verification middleware

use crate::crypto_worker::CryptoWorker;
use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::borrow::Cow;
use std::sync::Arc;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

/// Middleware that verifies event signatures and basic validity
#[derive(Clone, Debug)]
pub struct EventVerifierMiddleware<T = ()> {
    crypto_worker: Arc<CryptoWorker>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> EventVerifierMiddleware<T> {
    pub fn new(crypto_worker: Arc<CryptoWorker>) -> Self {
        Self {
            crypto_worker,
            _phantom: std::marker::PhantomData,
        }
    }
}

#[async_trait]
impl<T: Clone + Send + Sync + std::fmt::Debug + 'static> Middleware for EventVerifierMiddleware<T> {
    type State = NostrConnectionState<T>;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        if let Some(ClientMessage::Event(event_cow)) = &ctx.message {
            let event_id = event_cow.id;
            let event_to_verify: Event = event_cow.as_ref().clone();

            // Use the crypto worker for verification
            let verification_failed = match self.crypto_worker.verify_event(event_to_verify).await {
                Ok(()) => false,
                Err(_) => true,
            };

            if verification_failed {
                ctx.send_message(RelayMessage::ok(
                    event_id,
                    false,
                    Cow::Borrowed("invalid: event signature verification failed"),
                ))?;
                return Ok(());
            }
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
    use std::borrow::Cow;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    use websocket_builder::InboundContext;

    fn create_crypto_worker() -> Arc<CryptoWorker> {
        let keys = Arc::new(Keys::generate());
        let cancellation_token = CancellationToken::new();
        Arc::new(CryptoWorker::new(keys, cancellation_token))
    }

    fn create_middleware_chain(
        crypto_worker: Arc<CryptoWorker>,
    ) -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState<()>,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > {
        vec![Arc::new(EventVerifierMiddleware::<()>::new(crypto_worker))]
    }

    async fn create_signed_event() -> (Keys, Event) {
        let keys = Keys::generate();
        let event = EventBuilder::text_note("test message").build(keys.public_key());
        let event = keys.sign_event(event).await.expect("Failed to sign event");
        (keys, event)
    }

    fn create_test_state() -> NostrConnectionState<()> {
        NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL")
    }

    #[tokio::test]
    async fn test_valid_event_signature() {
        let crypto_worker = create_crypto_worker();
        let chain = create_middleware_chain(crypto_worker);
        let mut state = create_test_state();
        let (_, event) = create_signed_event().await;

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            Some(ClientMessage::Event(Cow::Owned(event))),
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
        let crypto_worker = create_crypto_worker();
        let chain = create_middleware_chain(crypto_worker);
        let mut state = create_test_state();
        let (_, mut event) = create_signed_event().await;
        let keys2 = Keys::generate();
        let event2 = EventBuilder::text_note("other message").build(keys2.public_key());
        let event2 = keys2
            .sign_event(event2)
            .await
            .expect("Failed to sign event");
        event.sig = event2.sig;

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            Some(ClientMessage::Event(Cow::Owned(event))),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_non_event_message_passes_through() {
        let crypto_worker = create_crypto_worker();
        let chain = create_middleware_chain(crypto_worker);
        let mut state = create_test_state();

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            Some(ClientMessage::Close(Cow::Owned(SubscriptionId::new(
                "test_sub",
            )))),
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
        let crypto_worker = create_crypto_worker();
        let chain = create_middleware_chain(crypto_worker);
        let mut state = create_test_state();
        let (_, auth_event) = create_signed_event().await;

        let mut ctx = InboundContext::new(
            "test_connection".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event))),
            None,
            &mut state,
            &chain,
            0,
        );

        let result = chain[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }
}
