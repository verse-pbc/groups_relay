use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::{debug, warn};
use websocket_builder::{InboundContext, Middleware, SendMessage};

use crate::groups::{
    ADDRESSABLE_EVENT_KINDS, GROUP_CONTENT_KINDS, KIND_GROUP_ADD_USER, KIND_GROUP_CREATE,
    KIND_GROUP_CREATE_INVITE, KIND_GROUP_DELETE, KIND_GROUP_DELETE_EVENT, KIND_GROUP_EDIT_METADATA,
    KIND_GROUP_REMOVE_USER, KIND_GROUP_SET_ROLES, KIND_GROUP_USER_JOIN_REQUEST,
    KIND_GROUP_USER_LEAVE_REQUEST,
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
        // Check if event kind is supported
        let supported = GROUP_CONTENT_KINDS.contains(&event.kind)
            || matches!(
                event.kind,
                k if k == KIND_GROUP_CREATE
                    || k == KIND_GROUP_DELETE
                    || k == KIND_GROUP_ADD_USER
                    || k == KIND_GROUP_REMOVE_USER
                    || k == KIND_GROUP_EDIT_METADATA
                    || k == KIND_GROUP_DELETE_EVENT
                    || k == KIND_GROUP_SET_ROLES
                    || k == KIND_GROUP_CREATE_INVITE
                    || k == KIND_GROUP_USER_JOIN_REQUEST
                    || k == KIND_GROUP_USER_LEAVE_REQUEST
            );

        if !supported {
            return Err("invalid: event kind not supported by this relay");
        }

        if event.tags.find(TagKind::h()).is_none() {
            return Err("invalid: group events must contain an 'h' tag");
        }

        Ok(())
    }

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
                GROUP_CONTENT_KINDS.contains(kind)
                    || matches!(
                        kind,
                        k if *k == KIND_GROUP_CREATE
                            || *k == KIND_GROUP_DELETE
                            || *k == KIND_GROUP_ADD_USER
                            || *k == KIND_GROUP_REMOVE_USER
                            || *k == KIND_GROUP_EDIT_METADATA
                            || *k == KIND_GROUP_DELETE_EVENT
                            || *k == KIND_GROUP_SET_ROLES
                            || *k == KIND_GROUP_CREATE_INVITE
                            || *k == KIND_GROUP_USER_JOIN_REQUEST
                            || *k == KIND_GROUP_USER_LEAVE_REQUEST
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
            ClientMessage::Req {
                subscription_id,
                filters,
            } => {
                debug!(
                    "[{}] Validating filters for subscription {}",
                    ctx.connection_id, subscription_id
                );

                for filter in filters {
                    if let Err(reason) =
                        self.validate_filter(filter, ctx.state.authed_pubkey.as_ref())
                    {
                        warn!(
                            "[{}] Filter validation failed for subscription {}: {}",
                            ctx.connection_id, subscription_id, reason
                        );

                        ctx.send_message(RelayMessage::notice(reason.to_string()))
                            .await?;

                        ctx.state.connection_token.cancel();
                        return Ok(());
                    }
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
    use nostr_sdk::{EventBuilder, Keys};
    use std::collections::BTreeSet;
    use std::time::Instant;

    fn create_test_keys() -> Keys {
        Keys::generate()
    }

    async fn create_test_event(keys: &Keys, kind: Kind, tags: Vec<Tag>) -> Event {
        let event = EventBuilder::new(kind, "test")
            .tags(tags)
            .build_with_ctx(&Instant::now(), keys.public_key());
        keys.sign_event(event).await.unwrap()
    }

    #[tokio::test]
    async fn test_event_validation() {
        let keys = create_test_keys();
        let relay_keys = create_test_keys();
        let middleware = ValidationMiddleware::new(relay_keys.public_key());

        // Test valid event with supported kind and h tag
        let event = create_test_event(
            &keys,
            GROUP_CONTENT_KINDS[0],
            vec![Tag::custom(TagKind::h(), ["test_group"])],
        )
        .await;
        assert!(middleware.validate_event(&event).is_ok());

        // Test event with supported kind but no h tag
        let event = create_test_event(&keys, GROUP_CONTENT_KINDS[0], vec![]).await;
        assert!(middleware.validate_event(&event).is_err());

        // Test event with unsupported kind but h tag
        let event = create_test_event(
            &keys,
            Kind::Custom(65000),
            vec![Tag::custom(TagKind::h(), ["test_group"])],
        )
        .await;
        assert!(middleware.validate_event(&event).is_err());
    }

    #[test]
    fn test_filter_validation() {
        let relay_keys = create_test_keys();
        let middleware = ValidationMiddleware::new(relay_keys.public_key());

        // Test filter with h tag
        let mut filter = Filter::new();
        let mut tag_set = BTreeSet::new();
        tag_set.insert("test".to_string());
        filter
            .generic_tags
            .insert(SingleLetterTag::lowercase(Alphabet::H), tag_set);
        assert!(middleware.validate_filter(&filter, None).is_ok());

        // Test filter with d tag
        let mut filter = Filter::new();
        let mut tag_set = BTreeSet::new();
        tag_set.insert("test".to_string());
        filter
            .generic_tags
            .insert(SingleLetterTag::lowercase(Alphabet::D), tag_set);
        assert!(middleware.validate_filter(&filter, None).is_ok());

        // Test filter with supported kinds
        let filter = Filter::new().kinds(vec![GROUP_CONTENT_KINDS[0]]);
        assert!(middleware.validate_filter(&filter, None).is_ok());

        // Test filter with unsupported kinds
        let filter = Filter::new().kinds(vec![Kind::Custom(65000)]);
        assert!(middleware.validate_filter(&filter, None).is_err());

        // Test filter with no tags and no kinds
        let filter = Filter::new();
        assert!(middleware.validate_filter(&filter, None).is_err());

        // Test filter with relay pubkey should skip validation
        let filter = Filter::new(); // Invalid filter
        assert!(middleware
            .validate_filter(&filter, Some(&relay_keys.public_key))
            .is_ok());
    }
}
