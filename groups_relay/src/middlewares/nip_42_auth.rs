use crate::nostr_session_state::NostrConnectionState;
use anyhow::Error;
use async_trait::async_trait;
use nostr_sdk::{
    ClientMessage, Event, Kind, PublicKey, RelayMessage, TagKind, TagStandard, Timestamp,
};
use tracing::{debug, error, info, warn};
use websocket_builder::{ConnectionContext, InboundContext, Middleware, SendMessage};

const MAX_AUTH_EVENT_AGE: u64 = 5000;

#[derive(Debug)]
pub struct Nip42Auth {
    local_url: String,
}

impl Nip42Auth {
    pub fn new(local_url: String) -> Self {
        debug!("Initializing Nip42Auth with local_url: {}", local_url);
        let local_url = local_url.trim_end_matches('/').to_string();
        Self { local_url }
    }

    pub fn authed_pubkey(&self, event: &Event, challenge: Option<&str>) -> Option<PublicKey> {
        debug!(
            "Checking auth event: id={}, pubkey={}, kind={}",
            event.id, event.pubkey, event.kind
        );

        let challenge = match challenge {
            Some(c) => {
                debug!("Found challenge: {}", c);
                c
            }
            None => {
                warn!("No challenge provided");
                return None;
            }
        };

        if event.kind != Kind::Authentication {
            warn!(
                "Event kind is not authentication. Got {}, expected {}",
                event.kind,
                Kind::Authentication
            );
            return None;
        }

        if let Err(e) = event.verify() {
            warn!("Event signature verification failed: {:?}", e);
            return None;
        }
        debug!("Event signature verified successfully");

        let now = Timestamp::now();
        let event_age = now.as_u64().saturating_sub(event.created_at.as_u64());
        debug!(
            "Checking event age. Now: {}, Created: {}, Age: {}ms",
            now.to_human_datetime(),
            event.created_at.to_human_datetime(),
            event_age
        );

        if event_age > MAX_AUTH_EVENT_AGE {
            warn!(
                "Event is too old. Now: {}, Created: {}, Age: {}ms, Max: {}ms",
                now.to_human_datetime(),
                event.created_at.to_human_datetime(),
                event_age,
                MAX_AUTH_EVENT_AGE
            );
            return None;
        }

        let challenge_tag = event.tags.find_standardized(TagKind::Challenge);
        debug!("Found challenge tag: {:?}", challenge_tag);

        let has_valid_challenge = matches!(
            challenge_tag,
            Some(TagStandard::Challenge(c)) if c == challenge
        );

        if !has_valid_challenge {
            warn!(
                "Event has invalid challenge. Expected: {}, Got: {:?}",
                challenge, challenge_tag
            );
            return None;
        }
        debug!("Challenge validated successfully");

        let relay_tag = event.tags.find_standardized(TagKind::Relay);
        debug!("Found relay tag: {:?}", relay_tag);

        let relay_url = match relay_tag {
            Some(TagStandard::Relay(relay_url)) => relay_url
                .as_str_without_trailing_slash()
                .trim_end_matches('/')
                .to_string(),
            None => {
                warn!("Event has no relay tag");
                return None;
            }
            _ => {
                warn!("Event has invalid relay tag");
                return None;
            }
        };

        let has_valid_relay = relay_url == self.local_url;
        debug!(
            "Checking relay URL. Expected: {}, Got: {}, Match: {}",
            self.local_url, relay_url, has_valid_relay
        );

        if !has_valid_relay {
            warn!(
                "Event has invalid relay. Expected: {}, Got: {}",
                self.local_url, relay_url
            );
            return None;
        }

        info!("Authentication successful for pubkey: {}", event.pubkey);
        Some(event.pubkey)
    }

    fn get_auth_failure_reason(&self, event: &Event, challenge: Option<&str>) -> Option<String> {
        if challenge.is_none() {
            return Some("No challenge found in connection state".to_string());
        }
        let challenge = challenge.unwrap();

        if event.kind != Kind::Authentication {
            return Some(format!(
                "Event kind is not authentication. Got {}, expected {}",
                event.kind,
                Kind::Authentication
            ));
        }

        if let Err(e) = event.verify() {
            return Some(format!("Event signature verification failed: {:?}", e));
        }

        let now = Timestamp::now();
        let event_age = now.as_u64().saturating_sub(event.created_at.as_u64());
        if event_age > MAX_AUTH_EVENT_AGE {
            return Some(format!(
                "Event is too old. Created: {}, Age: {}ms, Max: {}ms",
                event.created_at.to_human_datetime(),
                event_age,
                MAX_AUTH_EVENT_AGE
            ));
        }

        let challenge_tag = event.tags.find_standardized(TagKind::Challenge);
        let has_valid_challenge = matches!(
            challenge_tag,
            Some(TagStandard::Challenge(c)) if c == challenge
        );
        if !has_valid_challenge {
            return Some(format!(
                "Invalid challenge. Expected: {}, Got: {:?}",
                challenge, challenge_tag
            ));
        }

        let relay_tag = event.tags.find_standardized(TagKind::Relay);
        let relay_url = match relay_tag {
            Some(TagStandard::Relay(relay_url)) => relay_url
                .as_str_without_trailing_slash()
                .trim_end_matches('/')
                .to_string(),
            None => return Some("No relay tag found in event".to_string()),
            _ => return Some("Invalid relay tag format".to_string()),
        };

        if relay_url != self.local_url {
            return Some(format!(
                "Invalid relay URL. Expected: {}, Got: {}",
                self.local_url, relay_url
            ));
        }

        None
    }
}

#[async_trait]
impl Middleware for Nip42Auth {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), Error> {
        let connection_id = ctx.connection_id.as_str();
        debug!("[{}] Processing inbound message", connection_id);

        match &ctx.message {
            ClientMessage::Auth(event) => {
                info!(
                    "[{}] Processing AUTH message. Event id: {}, pubkey: {}",
                    connection_id, event.id, event.pubkey
                );

                debug!(
                    "[{}] Current connection state - Challenge: {:?}, Authed: {:?}",
                    connection_id, ctx.state.challenge, ctx.state.authed_pubkey
                );

                if let Some(failure_reason) =
                    self.get_auth_failure_reason(event, ctx.state.challenge.as_deref())
                {
                    warn!(
                        "[{}] Authentication failed for event id {}: {}",
                        connection_id, event.id, failure_reason
                    );
                    ctx.send_message(RelayMessage::Ok {
                        event_id: event.id,
                        status: false,
                        message: format!("auth-failed: {}", failure_reason),
                    })
                    .await?;
                    return ctx.next().await;
                }

                // If we get here, authentication was successful
                ctx.state.authed_pubkey = Some(event.pubkey);
                info!(
                    "[{}] Authentication successful for pubkey: {}",
                    connection_id, event.pubkey
                );

                ctx.send_message(RelayMessage::Ok {
                    event_id: event.id,
                    status: true,
                    message: "".to_string(),
                })
                .await?;
                ctx.next().await
            }
            _ => {
                debug!("[{}] Non-AUTH message, passing through", connection_id);
                ctx.next().await
            }
        }
    }

    async fn on_connect(
        &self,
        ctx: &mut ConnectionContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), Error> {
        debug!(
            "[{}] New connection, generating challenge",
            ctx.connection_id.as_str()
        );
        let challenge_event = ctx.state.get_challenge_event();
        debug!(
            "[{}] Generated challenge event: {:?}",
            ctx.connection_id.as_str(),
            challenge_event
        );
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
