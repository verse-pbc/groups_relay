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

        let now = Timestamp::now();
        if (now.as_u64() - event.created_at.as_u64()) > MAX_AUTH_EVENT_AGE {
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
