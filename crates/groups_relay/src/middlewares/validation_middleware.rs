use crate::groups::NON_GROUP_ALLOWED_KINDS;
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
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022,
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
        // If the event is from the relay pubkey and has a 'd' tag, allow it.
        if event.pubkey == self.relay_pubkey && event.tags.find(TagKind::d()).is_some() {
            return Ok(());
        }

        // For all other cases, require an 'h' tag for group events unless the kind is in the non-group allowed set.
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
        // If the authed pubkey is the relay's pubkey, skip validation.
        if authed_pubkey == Some(&self.relay_pubkey) {
            debug!("Skipping filter validation for relay pubkey");
            return Ok(());
        }

        // Check if filter has either 'h' or 'd' tag.
        let has_h_tag = filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::H));

        let has_d_tag = filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::D));

        // Check if kinds are supported (if specified).
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

        // Filter must either have valid tags or valid kinds.
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

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<(), anyhow::Error> {
        let ClientMessage::Event(event) = &ctx.message else {
            return ctx.next().await;
        };

        debug!(
            "[{}] Validating event kind {} with id {}",
            ctx.connection_id, event.kind, event.id
        );

        if let Err(reason) = self.validate_event(event) {
            warn!(
                "[{}] Event {} validation failed: {}",
                ctx.connection_id, event.id, reason
            );

            // Send error message
            ctx.send_message(RelayMessage::ok(event.id, false, reason))
                .await?;

            // Stop the chain here with Ok since we've handled the error
            return Ok(());
        }

        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    fn create_test_chain(
        middleware: ValidationMiddleware,
    ) -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage,
                OutgoingMessage = RelayMessage,
            >,
        >,
    > {
        vec![Arc::new(middleware)]
    }

    #[tokio::test]
    async fn test_filter_verification_normal_filter_with_h_tag() {
        let keys = nostr_sdk::Keys::generate();
        let middleware = ValidationMiddleware::new(keys.public_key());
        let chain = create_test_chain(middleware);

        let normal_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), "test_group");

        let mut state =
            NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filter: Box::new(normal_filter),
            },
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        assert!(chain[0].process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_d_tag() {
        let keys = nostr_sdk::Keys::generate();
        let middleware = ValidationMiddleware::new(keys.public_key());
        let chain = create_test_chain(middleware);

        let meta_filter = Filter::new()
            .kind(Kind::Custom(9007))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group");

        let mut state =
            NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filter: Box::new(meta_filter),
            },
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        assert!(chain[0].process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_reference_filter_with_e_tag() {
        let keys = nostr_sdk::Keys::generate();
        let middleware = ValidationMiddleware::new(keys.public_key());
        let chain = create_test_chain(middleware);

        let ref_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::E), "test_id");

        let mut state =
            NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = InboundContext::new(
            "test_conn".to_string(),
            ClientMessage::Req {
                subscription_id: SubscriptionId::new("test"),
                filter: Box::new(ref_filter),
            },
            None,
            &mut state,
            chain.as_slice(),
            0,
        );

        assert!(chain[0].process_inbound(&mut ctx).await.is_ok());
    }
}
