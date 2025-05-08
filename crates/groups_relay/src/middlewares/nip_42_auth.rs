use crate::error::Error;
use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
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
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, ClientMessage<'static>, RelayMessage<'static>>,
    ) -> Result<(), anyhow::Error> {
        match ctx.message.as_ref() {
            Some(ClientMessage::Auth(auth_event_cow)) => {
                let auth_event = auth_event_cow.as_ref();
                debug!(
                    target: "auth",
                    "[{}] Processing AUTH message for event ID {}",
                    ctx.connection_id, auth_event.id
                );

                let Some(expected_challenge) = ctx.state.challenge.as_ref() else {
                    error!(
                        target: "auth",
                        "[{}] No challenge found in state for AUTH message (event ID {}).",
                        ctx.connection_id, auth_event.id
                    );
                    ctx.send_message(RelayMessage::ok(
                        auth_event.id,
                        false,
                        "auth-required: no challenge pending",
                    ))
                    .await?;
                    return Err(Error::auth_required("No challenge found in state").into());
                };

                if auth_event.kind != Kind::Authentication {
                    error!(
                        target: "auth",
                        "[{}] Invalid event kind for AUTH message: {} (event ID {}).",
                        ctx.connection_id, auth_event.kind, auth_event.id
                    );
                    ctx.send_message(RelayMessage::ok(
                        auth_event.id,
                        false,
                        "auth-required: invalid event kind",
                    ))
                    .await?;
                    return Err(Error::auth_required("Invalid event kind").into());
                }

                if auth_event.verify().is_err() {
                    error!(
                        target: "auth",
                        "[{}] Invalid signature for AUTH message (event ID {}).",
                        ctx.connection_id, auth_event.id
                    );
                    ctx.send_message(RelayMessage::ok(
                        auth_event.id,
                        false,
                        "auth-required: invalid signature",
                    ))
                    .await?;
                    return Err(Error::auth_required("Invalid signature").into());
                }

                let found_challenge_in_tag: Option<String> =
                    auth_event.tags.iter().find_map(|tag_ref: &Tag| {
                        match tag_ref.as_standardized() {
                            Some(TagStandard::Challenge(s)) => Some(s.clone()),
                            _ => None,
                        }
                    });

                match found_challenge_in_tag {
                    Some(tag_challenge_str) => {
                        if tag_challenge_str != *expected_challenge {
                            error!(
                                target: "auth",
                                "[{}] Challenge mismatch for AUTH. Expected '{}', got '{}'. Event ID: {}.",
                                ctx.connection_id, expected_challenge, tag_challenge_str, auth_event.id
                            );
                            ctx.send_message(RelayMessage::ok(
                                auth_event.id,
                                false,
                                "auth-required: challenge mismatch",
                            ))
                            .await?;
                            return Err(Error::auth_required("Challenge mismatch").into());
                        }
                    }
                    None => {
                        error!(
                            target: "auth",
                            "[{}] No challenge tag found in AUTH message. Event ID: {}.",
                            ctx.connection_id, auth_event.id
                        );
                        ctx.send_message(RelayMessage::ok(
                            auth_event.id,
                            false,
                            "auth-required: missing challenge tag",
                        ))
                        .await?;
                        return Err(Error::auth_required("No challenge tag found").into());
                    }
                }

                let found_relay_in_tag: Option<RelayUrl> =
                    auth_event.tags.iter().find_map(|tag_ref: &Tag| {
                        match tag_ref.as_standardized() {
                            Some(TagStandard::Relay(r)) => Some(r.clone()),
                            _ => None,
                        }
                    });

                match found_relay_in_tag {
                    Some(tag_relay_url) => {
                        if tag_relay_url.as_str_without_trailing_slash()
                            != self.auth_url.trim_end_matches('/')
                        {
                            error!(
                                target: "auth",
                                "[{}] Relay URL mismatch for AUTH. Expected '{}', got '{}'. Event ID: {}.",
                                ctx.connection_id, self.auth_url, tag_relay_url.as_str_without_trailing_slash(), auth_event.id
                            );
                            ctx.send_message(RelayMessage::ok(
                                auth_event.id,
                                false,
                                "auth-required: relay mismatch",
                            ))
                            .await?;
                            return Err(Error::auth_required("Relay mismatch").into());
                        }
                    }
                    None => {
                        error!(
                            target: "auth",
                            "[{}] No relay tag found in AUTH message. Event ID: {}.",
                            ctx.connection_id, auth_event.id
                        );
                        ctx.send_message(RelayMessage::ok(
                            auth_event.id,
                            false,
                            "auth-required: missing relay tag",
                        ))
                        .await?;
                        return Err(Error::auth_required("No relay tag found").into());
                    }
                }

                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_else(|_| Duration::from_secs(0))
                    .as_secs();
                if auth_event.created_at.as_u64() < now.saturating_sub(600) {
                    error!(
                        target: "auth",
                        "[{}] Expired AUTH message (event ID {}). Created at: {}, Now: {}",
                        ctx.connection_id, auth_event.id, auth_event.created_at.as_u64(), now
                    );
                    ctx.send_message(RelayMessage::ok(
                        auth_event.id,
                        false,
                        "auth-required: expired auth event",
                    ))
                    .await?;
                    return Err(Error::auth_required("Expired auth event").into());
                }

                ctx.state.authed_pubkey = Some(auth_event.pubkey);
                ctx.state.challenge = None;
                debug!(
                    target: "auth",
                    "[{}] Successfully authenticated pubkey {} (event ID {}).",
                    ctx.connection_id, auth_event.pubkey, auth_event.id
                );
                ctx.send_message(RelayMessage::ok(auth_event.id, true, "authenticated"))
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
    use crate::nostr_session_state::NostrConnectionState;
    use crate::test_utils::create_test_state;
    use nostr_sdk::{ClientMessage, EventBuilder, Keys, Kind, RelayMessage, Tag, Timestamp};
    use std::borrow::Cow;
    use std::sync::Arc;
    use websocket_builder::{ConnectionContext, InboundContext};

    fn create_middleware_chain() -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
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

        let mut ctx = InboundContext::<
            '_,
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
        >::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event.clone()))),
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

        let mut ctx = InboundContext::<
            '_,
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
        >::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event.clone()))),
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

        let mut ctx = InboundContext::<
            '_,
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
        >::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event.clone()))),
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

        let mut ctx = InboundContext::<
            '_,
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
        >::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event.clone()))),
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

        let mut ctx = InboundContext::<
            '_,
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
        >::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event.clone()))),
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

        let mut ctx = InboundContext::<
            '_,
            NostrConnectionState,
            ClientMessage<'static>,
            RelayMessage<'static>,
        >::new(
            "test_conn".to_string(),
            Some(ClientMessage::Auth(Cow::Owned(auth_event.clone()))),
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

        let mut ctx = ConnectionContext::new(
            "test_conn".to_string(),
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        assert!(middleware.on_connect(&mut ctx).await.is_ok());
        assert!(state.challenge.is_some());
    }
}
