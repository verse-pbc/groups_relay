use crate::nostr_session_state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::{debug, warn};
use websocket_builder::{InboundContext, Middleware, SendMessage};

use crate::groups::{
    GROUP_CONTENT_KINDS, KIND_GROUP_ADD_USER, KIND_GROUP_CREATE, KIND_GROUP_CREATE_INVITE,
    KIND_GROUP_DELETE, KIND_GROUP_DELETE_EVENT, KIND_GROUP_EDIT_METADATA, KIND_GROUP_REMOVE_USER,
    KIND_GROUP_SET_ROLES, KIND_GROUP_USER_JOIN_REQUEST, KIND_GROUP_USER_LEAVE_REQUEST,
    METADATA_EVENT_KINDS,
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

        if !has_h_tag && !has_d_tag {
            return Err("invalid: filter must contain either 'h' or 'd' tag");
        }

        // Check if kinds are supported
        if let Some(kinds) = &filter.kinds {
            for kind in kinds {
                let supported = GROUP_CONTENT_KINDS.contains(kind)
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
                            || METADATA_EVENT_KINDS.contains(k)
                    );

                if !supported {
                    return Err("invalid: filter contains unsupported event kind");
                }
            }
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
