use crate::nostr_session_state::NostrConnectionState;
use async_trait::async_trait;
use nostr_sdk::{
    ClientMessage, Event, Kind, PublicKey, RelayMessage, TagKind, TagStandard, Timestamp,
};
use tracing::{debug, warn};
use websocket_builder::{ConnectionContext, InboundContext, Middleware, SendMessage};

const MAX_AUTH_EVENT_AGE: u64 = 5000;

#[derive(Debug)]
pub struct Nip42Auth {
    local_url: String,
}

impl Nip42Auth {
    pub fn new(local_url: String) -> Self {
        Self { local_url }
    }

    pub fn authed_pubkey(&self, event: &Event, challenge: Option<&str>) -> Option<PublicKey> {
        let challenge = match challenge {
            Some(c) => c,
            None => {
                warn!("No challenge provided");
                return None;
            }
        };

        if event.kind != Kind::Authentication {
            warn!(
                "Event kind is not authentication. It should be {}",
                Kind::Authentication
            );
            return None;
        }

        if event.verify().is_err() {
            warn!("Event signature verification failed");
            return None;
        }

        let now = Timestamp::now();
        if now.as_u64().saturating_sub(event.created_at.as_u64()) > MAX_AUTH_EVENT_AGE {
            warn!(
                "Event is too old. Now is: {}, event created at: {}",
                now.to_human_datetime(),
                event.created_at.to_human_datetime()
            );
            return None;
        }

        let has_valid_challenge = matches!(
            event.tags.find_standardized(TagKind::Challenge),
            Some(TagStandard::Challenge(c)) if c == challenge
        );

        if !has_valid_challenge {
            warn!("Event has invalid challenge");
            return None;
        }

        let relay_url = match event.tags.find_standardized(TagKind::Relay) {
            Some(TagStandard::Relay(relay_url)) => relay_url.as_str_without_trailing_slash(),
            None => {
                warn!("Event has no relay");
                return None;
            }
            _ => {
                warn!("Event has invalid relay");
                return None;
            }
        };

        let has_valid_relay = *relay_url == self.local_url;

        if !has_valid_relay {
            warn!(
                "Event has invalid relay. It should be {}, it was {}",
                self.local_url, relay_url
            );
            return None;
        }

        Some(event.pubkey)
    }
}

#[async_trait]
impl Middleware for Nip42Auth {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let connection_id = ctx.connection_id.as_str();

        match &ctx.message {
            ClientMessage::Auth(event) => {
                debug!(
                    "[{}] Received AUTH message with event id: {}",
                    connection_id, event.id
                );

                ctx.state.authed_pubkey = self.authed_pubkey(event, ctx.state.challenge.as_deref());

                if !ctx.state.is_authenticated() {
                    return ctx
                        .send_message(RelayMessage::Ok {
                            event_id: event.id,
                            status: false,
                            message: "auth-failed: invalid authentication event".to_string(),
                        })
                        .await;
                }

                ctx.send_message(RelayMessage::Ok {
                    event_id: event.id,
                    status: true,
                    message: "".to_string(),
                })
                .await
            }
            _ => {
                return ctx.next().await;
            }
        }
    }

    async fn on_connect<'a>(
        &'a self,
        ctx: &mut ConnectionContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let challenge_event = ctx.state.get_challenge_event();
        ctx.send_message(challenge_event).await?;
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, NostrSigner, RelayUrl, Tag, TagStandard};
    use std::time::Instant;

    #[tokio::test]
    async fn test_authed_pubkey_valid_auth() {
        let keys = Keys::generate();
        let local_url = RelayUrl::parse("wss://test.relay").unwrap();
        let auth = Nip42Auth::new(local_url.as_str().to_string());
        let challenge = "test_challenge";

        // Create valid auth event
        let unsigned_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                challenge.to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let event = keys.sign_event(unsigned_event).await.unwrap();

        // Verify event signature
        assert!(event.verify().is_ok());

        // Test valid auth
        let result = auth.authed_pubkey(&event, Some(challenge));
        assert_eq!(result, Some(keys.public_key()));
    }

    #[tokio::test]
    async fn test_authed_pubkey_wrong_challenge() {
        let keys = Keys::generate();
        let local_url = RelayUrl::parse("wss://test.relay").unwrap();
        let auth = Nip42Auth::new(local_url.as_str().to_string());
        let challenge = "test_challenge";

        // Create valid auth event
        let unsigned_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                challenge.to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let event = keys.sign_event(unsigned_event).await.unwrap();

        // Test with wrong challenge
        let result = auth.authed_pubkey(&event, Some("wrong_challenge"));
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_authed_pubkey_missing_challenge() {
        let keys = Keys::generate();
        let local_url = RelayUrl::parse("wss://test.relay").unwrap();
        let auth = Nip42Auth::new(local_url.as_str().to_string());
        let challenge = "test_challenge";

        // Create valid auth event
        let unsigned_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                challenge.to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let event = keys.sign_event(unsigned_event).await.unwrap();

        // Test with missing challenge
        let result = auth.authed_pubkey(&event, None);
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_expired_auth() {
        let keys = Keys::generate();
        let local_url = RelayUrl::parse("wss://test.relay").unwrap();
        let auth = Nip42Auth::new(local_url.as_str().to_string());
        let challenge = "test_challenge";

        // Create expired auth event with manual timestamp
        let unsigned_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                challenge.to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
            .custom_created_at(Timestamp::from(0))
            .build(keys.public_key());

        let event = keys.sign_event(unsigned_event).await.unwrap();

        // Verify event signature
        assert!(event.verify().is_ok());

        let result = auth.authed_pubkey(&event, Some(challenge));
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_wrong_relay() {
        let keys = Keys::generate();
        let local_url = RelayUrl::parse("wss://test.relay").unwrap();
        let wrong_url = RelayUrl::parse("wss://wrong.relay").unwrap();
        let auth = Nip42Auth::new(local_url.as_str().to_string());
        let challenge = "test_challenge";

        // Create auth event with wrong relay
        let unsigned_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                challenge.to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(wrong_url)))
            .build_with_ctx(&Instant::now(), keys.public_key());
        let event = keys.sign_event(unsigned_event).await.unwrap();

        // Verify event signature
        assert!(event.verify().is_ok());

        let result = auth.authed_pubkey(&event, Some(challenge));
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_wrong_signature() {
        let auth_keys = Keys::generate();
        let wrong_keys = Keys::generate();
        let local_url = RelayUrl::parse("wss://test.relay").unwrap();
        let auth = Nip42Auth::new(local_url.as_str().to_string());
        let challenge = "test_challenge";

        // Create auth event with wrong_keys but claiming to be from auth_keys
        let unsigned_event = EventBuilder::new(Kind::Authentication, "")
            .tag(Tag::from_standardized(TagStandard::Challenge(
                challenge.to_string(),
            )))
            .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
            .build_with_ctx(&Instant::now(), auth_keys.public_key()); // Claim to be auth_keys
        let event = wrong_keys.sign_event(unsigned_event).await.unwrap(); // But sign with wrong_keys

        // Verify event signature should fail
        assert!(event.verify().is_err());

        // Should fail because signature doesn't match the claimed pubkey
        let result = auth.authed_pubkey(&event, Some(challenge));
        assert_eq!(result, None);
    }
}
