use crate::groups::NON_GROUP_ALLOWED_KINDS;
use async_trait::async_trait;
use nostr_relay_builder::NostrConnectionState;
use nostr_sdk::prelude::*;
use tracing::{debug, warn};
use websocket_builder::{InboundContext, Middleware, SendMessage};

use crate::groups::{
    ADDRESSABLE_EVENT_KINDS, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022,
};

#[derive(Debug, Clone)]
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
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<Self::State, ClientMessage<'static>, RelayMessage<'static>>,
    ) -> Result<(), anyhow::Error> {
        let Some(ClientMessage::Event(event)) = &ctx.message else {
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
            ctx.send_message(RelayMessage::ok(event.id, false, reason))?;

            // Stop the chain here with Ok since we've handled the error
            return Ok(());
        }

        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_relay_builder::NostrConnectionState;
    use std::borrow::Cow;
    use std::sync::Arc;
    use parking_lot::RwLock;
    use websocket_builder::InboundContext;
    extern crate flume;

    fn create_test_inbound_context(
        connection_id: String,
        message: Option<ClientMessage<'static>>,
        sender: Option<flume::Sender<(RelayMessage<'static>, usize)>>,
        state: NostrConnectionState,
        middlewares: Vec<
            Arc<
                dyn Middleware<
                    State = NostrConnectionState,
                    IncomingMessage = ClientMessage<'static>,
                    OutgoingMessage = RelayMessage<'static>,
                >,
            >,
        >,
        index: usize,
    ) -> InboundContext<NostrConnectionState, ClientMessage<'static>, RelayMessage<'static>> {
        let state_arc = Arc::new(RwLock::new(state));
        let middlewares_arc = Arc::new(middlewares);

        InboundContext::new(
            connection_id,
            message,
            sender,
            state_arc,
            middlewares_arc,
            index,
        )
    }

    fn create_test_chain(
        middleware: ValidationMiddleware,
    ) -> Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
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

        let normal_filter = Filter::default().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            "test_group".to_string(),
        );

        let message = ClientMessage::Req {
            subscription_id: Cow::Owned(SubscriptionId::new("test")),
            filter: Cow::Owned(normal_filter),
        };
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_inbound_context(
            "test_conn".to_string(),
            Some(message),
            None,
            state,
            chain.clone(),
            0,
        );

        assert!(chain[0].process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_d_tag() {
        let keys = nostr_sdk::Keys::generate();
        let middleware = ValidationMiddleware::new(keys.public_key());
        let chain = create_test_chain(middleware);

        let meta_filter = Filter::default()
            .kind(Kind::Custom(9007))
            .identifier("test_group".to_string());

        let message = ClientMessage::Req {
            subscription_id: Cow::Owned(SubscriptionId::new("test")),
            filter: Cow::Owned(meta_filter),
        };
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_inbound_context(
            "test_conn".to_string(),
            Some(message),
            None,
            state,
            chain.clone(),
            0,
        );

        assert!(chain[0].process_inbound(&mut ctx).await.is_ok());
    }

    #[tokio::test]
    async fn test_filter_verification_reference_filter_with_e_tag() {
        let keys = nostr_sdk::Keys::generate();
        let middleware = ValidationMiddleware::new(keys.public_key());
        let chain = create_test_chain(middleware);

        let event_id =
            EventId::from_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                .unwrap_or_else(|_| EventId::all_zeros()); // Placeholder if "test_id" is not valid hex
        let ref_filter = Filter::default().kind(Kind::Custom(11)).event(event_id);

        let message = ClientMessage::Req {
            subscription_id: Cow::Owned(SubscriptionId::new("test_id")),
            filter: Cow::Owned(ref_filter),
        };
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_inbound_context(
            "test_conn".to_string(),
            Some(message),
            None,
            state,
            chain.clone(),
            0,
        );

        assert!(chain[0].process_inbound(&mut ctx).await.is_ok());
    }
}
