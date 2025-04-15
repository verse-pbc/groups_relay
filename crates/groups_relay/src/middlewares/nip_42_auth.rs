use crate::error::Error;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error};
use websocket_builder::{
    ConnectionContext, InboundContext, Middleware, OutboundContext, SendMessage,
};

#[derive(Debug, Clone)]
pub struct Nip42Middleware {
    auth_url: String,
}

impl Nip42Middleware {
    pub fn new(auth_url: String) -> Self {
        Self { auth_url }
    }
}

#[async_trait]
impl Middleware for Nip42Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        match &ctx.message {
            Some(ClientMessage::Auth(event)) => {
                debug!(
                    target: "auth",
                    "[{}] Processing AUTH message",
                    ctx.connection_id
                );

                // Verify the challenge matches
                let Some(challenge) = &ctx.state.challenge else {
                    error!(
                        target: "auth",
                        "[{}] No challenge found for AUTH message",
                        ctx.connection_id
                    );
                    return Err(Error::auth_required("No challenge found").into());
                };

                // Verify the event is a NIP-42 auth event
                if event.kind != Kind::Authentication {
                    error!(
                        target: "auth",
                        "[{}] Invalid event kind for AUTH message: {}",
                        ctx.connection_id,
                        event.kind
                    );
                    return Err(Error::auth_required("Invalid event kind").into());
                }

                // Verify the event is signed by the correct pubkey
                if event.verify().is_err() {
                    error!(
                        target: "auth",
                        "[{}] Invalid signature for AUTH message",
                        ctx.connection_id
                    );
                    return Err(Error::auth_required("Invalid signature").into());
                }

                // Verify the challenge tag matches
                let challenge_tag = event.tags.find_standardized(TagKind::Challenge);
                if let Some(TagStandard::Challenge(tag_challenge)) = challenge_tag {
                    if tag_challenge != challenge {
                        error!(
                            target: "auth",
                            "[{}] Challenge mismatch for AUTH message",
                            ctx.connection_id
                        );
                        return Err(Error::auth_required("Challenge mismatch").into());
                    }
                } else {
                    error!(
                        target: "auth",
                        "[{}] No challenge tag found in AUTH message",
                        ctx.connection_id
                    );
                    return Err(Error::auth_required("No challenge tag found").into());
                }

                // Verify the relay tag matches
                let relay_tag = event.tags.find_standardized(TagKind::Relay);
                if let Some(TagStandard::Relay(tag_url)) = relay_tag {
                    if tag_url.as_str_without_trailing_slash() != self.auth_url {
                        error!(
                            target: "auth",
                            "[{}] Relay mismatch for AUTH message, wants {} but got {}",
                            ctx.connection_id,
                            self.auth_url,
                            tag_url.as_str_without_trailing_slash()
                        );
                        return Err(Error::auth_required("Relay mismatch").into());
                    }
                } else {
                    error!(
                        target: "auth",
                        "[{}] No relay tag found in AUTH message",
                        ctx.connection_id
                    );
                    return Err(Error::auth_required("No relay tag found").into());
                }

                // Verify the event is not expired
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                if event.created_at.as_u64() < now - 600 {
                    error!(
                        target: "auth",
                        "[{}] Expired AUTH message",
                        ctx.connection_id
                    );
                    return Err(Error::auth_required("Expired auth event").into());
                }

                // Set the authed pubkey
                ctx.state.authed_pubkey = Some(event.pubkey);
                debug!(
                    target: "auth",
                    "[{}] Successfully authenticated pubkey {}",
                    ctx.connection_id,
                    event.pubkey
                );

                // Send OK message
                ctx.send_message(RelayMessage::ok(
                    event.id,
                    true,
                    "Successfully authenticated",
                ))
                .await?;

                Ok(())
            }
            _ => ctx.next().await,
        }
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        ctx.next().await
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        debug!(
            target: "auth",
            "[{}] New connection, sending auth challenge",
            ctx.connection_id
        );
        let challenge_event = ctx.state.get_challenge_event();
        debug!(
            target: "auth",
            "[{}] Generated challenge event: {:?}",
            ctx.connection_id,
            challenge_event
        );
        ctx.send_message(challenge_event).await?;
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::create_test_state;
    use std::sync::Arc;
    use std::time::Instant;

    fn create_middleware_chain() -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage,
                OutgoingMessage = RelayMessage,
            >,
        >,
    > {
        vec![Arc::new(Nip42Middleware::new(
            "wss://test.relay".to_string(),
        ))]
    }

    #[tokio::test]
    async fn test_authed_pubkey_valid_auth() {
        let keys = Keys::generate();
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url.clone());
        let mut state = create_test_state(None);
        let challenge = "test_challenge".to_string();
        state.challenge = Some(challenge.clone());

        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(challenge)))
            .tag(Tag::from_standardized(TagStandard::Relay(
                RelayUrl::parse(&auth_url).unwrap(),
            )))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let auth_event = keys.sign_event(auth_event).await.unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Box::new(auth_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
        assert_eq!(state.authed_pubkey, Some(keys.public_key()));
    }

    #[tokio::test]
    async fn test_authed_pubkey_missing_challenge() {
        let keys = Keys::generate();
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url.clone());
        let mut state = create_test_state(None);

        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Relay(
                RelayUrl::parse(&auth_url).unwrap(),
            )))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let auth_event = keys.sign_event(auth_event).await.unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Box::new(auth_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_err());
        assert_eq!(state.authed_pubkey, None);
    }

    #[tokio::test]
    async fn test_authed_pubkey_wrong_challenge() {
        let keys = Keys::generate();
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url.clone());
        let mut state = create_test_state(None);
        let challenge = "test_challenge".to_string();
        state.challenge = Some(challenge);

        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                "wrong_challenge".to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(
                RelayUrl::parse(&auth_url).unwrap(),
            )))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let auth_event = keys.sign_event(auth_event).await.unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Box::new(auth_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_err());
        assert_eq!(state.authed_pubkey, None);
    }

    #[tokio::test]
    async fn test_wrong_relay() {
        let keys = Keys::generate();
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url);
        let mut state = create_test_state(None);
        let challenge = "test_challenge".to_string();
        state.challenge = Some(challenge.clone());

        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(challenge)))
            .tag(Tag::from_standardized(TagStandard::Relay(
                RelayUrl::parse("wss://wrong.relay").unwrap(),
            )))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let auth_event = keys.sign_event(auth_event).await.unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Box::new(auth_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_err());
        assert_eq!(state.authed_pubkey, None);
    }

    #[tokio::test]
    async fn test_wrong_signature() {
        let keys = Keys::generate();
        let wrong_keys = Keys::generate();
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url.clone());
        let mut state = create_test_state(None);
        let challenge = "test_challenge".to_string();
        state.challenge = Some(challenge.clone());

        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(challenge)))
            .tag(Tag::from_standardized(TagStandard::Relay(
                RelayUrl::parse(&auth_url).unwrap(),
            )))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let auth_event = wrong_keys.sign_event(auth_event).await.unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Box::new(auth_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_err());
        assert_eq!(state.authed_pubkey, None);
    }

    #[tokio::test]
    async fn test_expired_auth() {
        let keys = Keys::generate();
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url.clone());
        let mut state = create_test_state(None);
        let challenge = "test_challenge".to_string();
        state.challenge = Some(challenge.clone());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let expired_time = now - 601; // Just over 10 minutes ago

        let auth_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(challenge)))
            .tag(Tag::from_standardized(TagStandard::Relay(
                RelayUrl::parse(&auth_url).unwrap(),
            )))
            .custom_created_at(Timestamp::from(expired_time))
            .build(keys.public_key());
        let auth_event = keys.sign_event(auth_event).await.unwrap();

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Box::new(auth_event.clone()))),
            None,
            &mut state,
            &[],
            0,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_err());
        assert_eq!(state.authed_pubkey, None);
    }

    #[tokio::test]
    async fn test_on_connect_sends_challenge() {
        let auth_url = "wss://test.relay".to_string();
        let middleware = Nip42Middleware::new(auth_url.clone());
        let mut state = create_test_state(None);
        let chain = create_middleware_chain();

        let mut ctx = ConnectionContext::new("test_conn".to_string(), None, &mut state, &chain, 0);

        assert!(middleware.on_connect(&mut ctx).await.is_ok());
        assert!(state.challenge.is_some());
    }
}
