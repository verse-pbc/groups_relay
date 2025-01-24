use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::{debug, warn};
use websocket_builder::{InboundContext, Middleware, SendMessage};

use crate::groups::{
    ADDRESSABLE_EVENT_KINDS, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022, NON_GROUP_ALLOWED_KINDS,
};

#[derive(Debug)]
pub struct ValidationMiddleware {
    relay_pubkey: PublicKey,
}

impl ValidationMiddleware {
    pub fn new(relay_pubkey: PublicKey) -> Self {
        Self { relay_pubkey }
    }

    fn validate_event(&self, event: &Event) -> Result<(), &'static str> {
        if event.tags.find(TagKind::h()).is_none() && !NON_GROUP_ALLOWED_KINDS.contains(&event.kind)
        {
            return Err("invalid: group events must contain an 'h' tag");
        }

        Ok(())
    }

    // This was too much, may remove it
    #[allow(unused)]
    fn validate_filter(
        &self,
        filter: &Filter,
        authed_pubkey: Option<&PublicKey>,
    ) -> Result<(), &'static str> {
        // If the authed pubkey is the relay's pubkey, skip validation
        if authed_pubkey.map_or(false, |pk| pk == &self.relay_pubkey) {
            debug!("Skipping filter validation for relay pubkey");
            return Ok(());
        }

        // Check if filter has either 'h' or 'd' tag
        let has_h_tag = filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::H));

        let has_d_tag = filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::D));

        // Check if kinds are supported (if specified)
        let has_valid_kinds = if let Some(kinds) = &filter.kinds {
            kinds.iter().all(|kind| {
                NON_GROUP_ALLOWED_KINDS.contains(kind)
                    || matches!(
                        kind,
                        k if *k == KIND_GROUP_CREATE_9007
                            || *k == KIND_GROUP_DELETE_9008
                            || *k == KIND_GROUP_ADD_USER_9000
                            || *k == KIND_GROUP_REMOVE_USER_9001
                            || *k == KIND_GROUP_EDIT_METADATA_9002
                            || *k == KIND_GROUP_DELETE_EVENT_9005
                            || *k == KIND_GROUP_SET_ROLES_9006
                            || *k == KIND_GROUP_CREATE_INVITE_9009
                            || *k == KIND_GROUP_USER_JOIN_REQUEST_9021
                            || *k == KIND_GROUP_USER_LEAVE_REQUEST_9022
                            || ADDRESSABLE_EVENT_KINDS.contains(k)
                    )
            })
        } else {
            false
        };

        // Filter must either have valid tags or valid kinds
        if !has_h_tag && !has_d_tag && !has_valid_kinds {
            return Err("invalid: filter must contain either 'h'/'d' tag or supported kinds");
        }

        Ok(())
    }
}

#[async_trait]
impl Middleware for ValidationMiddleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        match &ctx.message {
            ClientMessage::Event(event) => {
                debug!(
                    "[{}] Validating event kind {} with id {}",
                    ctx.connection_id, event.kind, event.id
                );

                if let Err(reason) = self.validate_event(event) {
                    warn!(
                        "[{}] Event {} validation failed: {}",
                        ctx.connection_id, event.id, reason
                    );

                    ctx.send_message(RelayMessage::ok(event.id, false, reason))
                        .await?;

                    ctx.state.connection_token.cancel();
                    return Ok(());
                }

                ctx.next().await
            }
            _ => ctx.next().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, create_test_keys};
    use nostr_sdk::{Filter, Kind, SingleLetterTag, Tag, TagKind};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    use websocket_builder::Middleware;

    type TestMiddleware = dyn Middleware<
        IncomingMessage = ClientMessage,
        OutgoingMessage = RelayMessage,
        State = NostrConnectionState,
    >;

    #[tokio::test]
    async fn test_event_validation_valid_event_with_h_tag() {
        let (admin_keys, _, _) = create_test_keys().await;
        let middleware = Arc::new(ValidationMiddleware::new(admin_keys.public_key()));
        let chain: Vec<Arc<TestMiddleware>> = vec![middleware.clone()];

        let event = create_test_event(
            &admin_keys,
            9,
            vec![Tag::custom(TagKind::h(), ["test_group"])],
        )
        .await;

        let mut state = NostrConnectionState {
            relay_url: "wss://test.relay".to_string(),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
        };

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Event(Box::new(event)),
            None,
            &mut state,
            &chain,
            1,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_event_validation_accepts_event_with_different_kind() {
        let (admin_keys, _, _) = create_test_keys().await;
        let middleware = Arc::new(ValidationMiddleware::new(admin_keys.public_key()));
        let chain: Vec<Arc<TestMiddleware>> = vec![middleware.clone()];

        let event = create_test_event(
            &admin_keys,
            10009, // This kind doesn't need an 'h' tag
            vec![],
        )
        .await;

        let mut state = NostrConnectionState {
            relay_url: "wss://test.relay".to_string(),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
        };

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Event(Box::new(event)),
            None,
            &mut state,
            &chain,
            1,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_validation_accepts_h_tag() {
        let (admin_keys, _, _) = create_test_keys().await;
        let middleware = Arc::new(ValidationMiddleware::new(admin_keys.public_key()));
        let chain: Vec<Arc<TestMiddleware>> = vec![middleware.clone()];

        let filter = Filter::new()
            .kind(Kind::Custom(9))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), vec!["test_group"]);

        let mut state = NostrConnectionState {
            relay_url: "wss://test.relay".to_string(),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
        };

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filters: vec![filter],
            },
            None,
            &mut state,
            &chain,
            1,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_validation_accepts_d_tag() {
        let (admin_keys, _, _) = create_test_keys().await;
        let middleware = Arc::new(ValidationMiddleware::new(admin_keys.public_key()));
        let chain: Vec<Arc<TestMiddleware>> = vec![middleware.clone()];

        let filter = Filter::new()
            .kind(Kind::Custom(9))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec!["test_group"]);

        let mut state = NostrConnectionState {
            relay_url: "wss://test.relay".to_string(),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
        };

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filters: vec![filter],
            },
            None,
            &mut state,
            &chain,
            1,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_validation_accepts_non_group_supported_tag() {
        let (admin_keys, _, _) = create_test_keys().await;
        let middleware = Arc::new(ValidationMiddleware::new(admin_keys.public_key()));
        let chain: Vec<Arc<TestMiddleware>> = vec![middleware.clone()];

        let filter = Filter::new()
            .kind(Kind::Custom(10009)) // This kind doesn't need an 'h' tag
            .custom_tag(SingleLetterTag::lowercase(Alphabet::E), vec!["test_id"]);

        let mut state = NostrConnectionState {
            relay_url: "wss://test.relay".to_string(),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
        };

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filters: vec![filter],
            },
            None,
            &mut state,
            &chain,
            1,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_validation_accepts_relay_pubkey() {
        let (admin_keys, _, _) = create_test_keys().await;
        let middleware = Arc::new(ValidationMiddleware::new(admin_keys.public_key()));
        let chain: Vec<Arc<TestMiddleware>> = vec![middleware.clone()];

        let filter = Filter::new()
            .kind(Kind::Custom(9))
            .authors(vec![admin_keys.public_key()]);

        let mut state = NostrConnectionState {
            relay_url: "wss://test.relay".to_string(),
            challenge: None,
            authed_pubkey: None,
            relay_connection: None,
            connection_token: CancellationToken::new(),
        };

        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filters: vec![filter],
            },
            None,
            &mut state,
            &chain,
            1,
        );

        assert!(middleware.process_inbound(&mut ctx).await.is_ok());
    }
}
