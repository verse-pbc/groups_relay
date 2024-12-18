use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use tracing::debug;

use crate::error::Error;
use crate::groups::{
    Group, Groups, GROUP_CONTENT_KINDS, KIND_GROUP_ADD_USER, KIND_GROUP_CREATE,
    KIND_GROUP_CREATE_INVITE, KIND_GROUP_DELETE, KIND_GROUP_DELETE_EVENT, KIND_GROUP_EDIT_METADATA,
    KIND_GROUP_REMOVE_USER, KIND_GROUP_SET_ROLES, KIND_GROUP_USER_JOIN_REQUEST,
    KIND_GROUP_USER_LEAVE_REQUEST, METADATA_EVENT_KINDS,
};
use crate::nostr_session_state::NostrConnectionState;
use crate::EventToSave;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

#[derive(Debug)]
pub struct Nip29Middleware {
    groups: Arc<Groups>,
    relay_pubkey: PublicKey,
}

impl Nip29Middleware {
    pub fn new(groups: Arc<Groups>, relay_pubkey: PublicKey) -> Self {
        Self {
            groups,
            relay_pubkey,
        }
    }

    /// Handles the creation of a group (`kind:9007`)
    async fn handle_group_create(&self, event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling group creation event");

        if let Some(group) = self.groups.find_group_from_event(event) {
            debug!("Group already exists: {:?}", group);
            return Err(Error::notice("Group already exists"));
        }

        let group = Group::new(event)?;

        let metadata_event = group.generate_metadata_event();
        let put_user_event = group.generate_put_user_event(&event.pubkey);
        let admins_event = group.generate_admins_event();
        let members_event = group.generate_members_event();
        let roles_event = group.generate_roles_event();

        debug!("New group: {:?}", group);
        let _ = self.groups.insert(group.id.to_string(), group);

        Ok(vec![
            EventToSave::Event(event.clone()),
            EventToSave::UnsignedEvent(metadata_event.build(self.relay_pubkey)),
            EventToSave::UnsignedEvent(put_user_event.build(self.relay_pubkey)),
            EventToSave::UnsignedEvent(admins_event.build(self.relay_pubkey)),
            EventToSave::UnsignedEvent(members_event.build(self.relay_pubkey)),
            EventToSave::UnsignedEvent(roles_event.build(self.relay_pubkey)),
        ])
    }

    async fn handle_set_roles<'a>(&'a self, event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling set-roles event");
        let mut group = self
            .groups
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.set_roles(event)?;

        Ok(vec![])
    }

    /// Handles the deletion of a group (`kind:9008`)
    async fn handle_group_delete<'a>(&'a self, _event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling group deletion event");
        Err(Error::notice("Group deletion is not supported yet"))
    }

    /// Handles adding a user to the group (`kind:9000`)
    async fn handle_put_user<'a>(&'a self, event: &Event) -> Result<Vec<EventToSave>, Error> {
        let mut group = self
            .groups
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        let added_admins = group.add_members(event)?;

        let mut events = vec![EventToSave::Event(event.clone())];
        if added_admins {
            let admins_event = group.generate_admins_event();
            events.push(EventToSave::UnsignedEvent(
                admins_event.build(self.relay_pubkey),
            ));
        }

        let members_event = group.generate_members_event();
        events.push(EventToSave::UnsignedEvent(
            members_event.build(self.relay_pubkey),
        ));

        Ok(events)
    }

    /// Handles removing a user from the group (`kind:9001`)
    async fn handle_remove_user<'a>(&'a self, event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling remove-user event");
        let mut group = self
            .groups
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        let removed_admins = group.remove_members(event)?;

        let mut events = vec![EventToSave::Event(event.clone())];
        if removed_admins {
            let admins_event = group.generate_admins_event();
            events.push(EventToSave::UnsignedEvent(
                admins_event.build(self.relay_pubkey),
            ));
        }

        let members_event = group.generate_members_event();
        events.push(EventToSave::UnsignedEvent(
            members_event.build(self.relay_pubkey),
        ));

        Ok(events)
    }

    /// Handles editing group metadata (`kind:9002`)
    async fn handle_edit_metadata<'a>(
        &'a self,
        edit_metadata_event: &Event,
    ) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling edit-metadata event");
        let mut group = self
            .groups
            .find_group_from_event_mut(edit_metadata_event)?
            .ok_or(Error::notice("Group not found"))?;

        group.set_metadata(edit_metadata_event)?;

        let metadata_event = group.generate_metadata_event();

        Ok(vec![
            EventToSave::Event(edit_metadata_event.clone()),
            EventToSave::UnsignedEvent(metadata_event.build(self.relay_pubkey)),
        ])
    }

    /// Handles deleting an event within the group (`kind:9005`)
    async fn handle_delete_event<'a>(&'a self, _event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling delete-event");
        Err(Error::notice("Event deletion is not supported yet"))
    }

    /// Handles creating an invite (`kind:9009`)
    async fn handle_create_invite<'a>(&'a self, event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling create-invite event");
        let mut group = self
            .groups
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.create_invite(event)?;

        Ok(vec![EventToSave::Event(event.clone())])
    }

    /// Handles a join request (`kind:9021`)
    async fn handle_join_request<'a>(
        &'a self,
        join_request_event: &Event,
    ) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling join-request event");
        let mut group = self
            .groups
            .find_group_from_event_mut(join_request_event)?
            .ok_or(Error::notice("Group not found"))?;

        // May auto-join the user if the invite code matches or the group is public
        let auto_joined = group.join_request(join_request_event)?;

        if auto_joined {
            let put_user_event = group.generate_put_user_event(&join_request_event.pubkey);
            let members_event = group.generate_members_event();
            return Ok(vec![
                EventToSave::Event(join_request_event.clone()),
                EventToSave::UnsignedEvent(put_user_event.build(self.relay_pubkey)),
                EventToSave::UnsignedEvent(members_event.build(self.relay_pubkey)),
            ]);
        }

        Ok(vec![EventToSave::Event(join_request_event.clone())])
    }

    /// Handles a leave request (`kind:9022`)
    async fn handle_leave_request<'a>(&'a self, event: &Event) -> Result<Vec<EventToSave>, Error> {
        debug!("Handling leave-request event");

        let mut group = self
            .groups
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        if group.leave_request(event)? {
            let members_event = group.generate_members_event();
            return Ok(vec![
                EventToSave::Event(event.clone()),
                EventToSave::UnsignedEvent(members_event.build(self.relay_pubkey)),
            ]);
        }

        Ok(vec![])
    }

    fn verify_filters(
        &self,
        authed_pubkey: Option<PublicKey>,
        filters: &[Filter],
    ) -> Result<(), Error> {
        filters
            .iter()
            .try_for_each(|f| self.verify_filter(authed_pubkey, f))
    }

    fn verify_filter(
        &self,
        authed_pubkey: Option<PublicKey>,
        filter: &Filter,
    ) -> Result<(), Error> {
        let mut is_meta: bool = false;
        let mut is_normal: bool = false;
        let mut is_reference: bool = false;

        if let Some(kinds) = &filter.kinds {
            for k in kinds {
                if METADATA_EVENT_KINDS.contains(k) {
                    is_meta = true;
                } else if is_meta {
                    return Err(Error::notice(
                        "Invalid query, cannot mix metadata and normal event kinds",
                    ));
                }
            }
        }

        if !is_meta {
            // we assume the caller wants normal events if the 'h' tag is specified
            // or metadata events if the 'd' tag is specified
            if filter
                .generic_tags
                .contains_key(&SingleLetterTag::lowercase(Alphabet::H))
            {
                is_normal = true;
            } else if !filter
                .generic_tags
                .contains_key(&SingleLetterTag::lowercase(Alphabet::D))
            {
                // this may be a request for "#e", "authors" or just "ids"
                is_reference = true;
            }
        }

        if is_normal {
            for tag in filter
                .generic_tags
                .iter()
                .filter(|(k, _)| k == &&SingleLetterTag::lowercase(Alphabet::H))
                .flat_map(|(_, tag_set)| tag_set.iter())
            {
                let group = self.groups.get_group(tag)?;

                debug!(
                    "checking filters for normal request for group: {:?}",
                    group.value()
                );
                if !group.metadata.private {
                    return Ok(());
                }

                match authed_pubkey {
                    Some(authed_pubkey) => {
                        if !group.is_member(&authed_pubkey) {
                            return Err(Error::restricted(
                                "authed user is not a member of this group",
                            ));
                        }
                    }
                    None => {
                        return Err(Error::auth_required("trying to read from a private group"));
                    }
                }
            }

            return Ok(());
        }

        // reference queries will be filtered on each individual event
        if is_reference {
            if filter
                .generic_tags
                .iter()
                .any(|(k, _)| k == &SingleLetterTag::lowercase(Alphabet::E))
            {
                return Ok(());
            }

            if filter.authors.is_some() && filter.ids.is_some() {
                return Err(Error::notice(
                    "invalid query, must have 'e', 'authors' or 'ids' tag",
                ));
            }
        }

        Ok(())
    }

    async fn handle_event<'a>(&'a self, event: &Event) -> Result<Option<Vec<EventToSave>>, Error> {
        if event.kind == KIND_GROUP_CREATE {
            debug!("Admin -> Relay: Creating group");
            let events_to_save = self.handle_group_create(event).await?;
            return Ok(Some(events_to_save));
        }

        let events_to_save = match event.kind {
            k if k == KIND_GROUP_EDIT_METADATA => {
                debug!("Admin -> Relay: Editing group metadata");
                self.handle_edit_metadata(event).await?
            }

            k if k == KIND_GROUP_USER_JOIN_REQUEST => {
                debug!("User -> Relay: Requesting to join group");
                self.handle_join_request(event).await?
            }

            k if k == KIND_GROUP_USER_LEAVE_REQUEST => {
                debug!("User -> Relay: Requesting to leave group");
                self.handle_leave_request(event).await?
            }

            k if k == KIND_GROUP_SET_ROLES => {
                debug!("Admin/Relay -> Relay: Setting roles");
                self.handle_set_roles(event).await?
            }

            k if k == KIND_GROUP_ADD_USER => {
                debug!("Admin/Relay -> Relay: Adding user to group");
                self.handle_put_user(event).await?
            }

            k if k == KIND_GROUP_REMOVE_USER => {
                debug!("Admin/Relay -> Relay: Removing user from group");
                self.handle_remove_user(event).await?
            }

            k if k == KIND_GROUP_DELETE => {
                debug!("Admin -> Relay: Deleting group");
                self.handle_group_delete(event).await?
            }

            k if k == KIND_GROUP_DELETE_EVENT => {
                debug!("Admin -> Relay: Deleting event");
                self.handle_delete_event(event).await?
            }

            k if k == KIND_GROUP_CREATE_INVITE => {
                debug!("Admin -> Relay: Creating invite");
                self.handle_create_invite(event).await?
            }

            // Group content events
            k => {
                debug!("User -> Relay: Group content event");
                let group = match self.groups.find_group_from_event(event) {
                    None => return Ok(None),
                    Some(group) => group,
                };

                if GROUP_CONTENT_KINDS.contains(&k) {
                    if !group.is_member(&event.pubkey) {
                        return Err(Error::notice("User is not a member of this group"));
                    }
                    vec![EventToSave::Event(event.clone())]
                } else {
                    return Err(Error::notice("Event kind not supported by this group"));
                }
            }
        };

        Ok(Some(events_to_save))
    }
}

#[async_trait]
impl Middleware for Nip29Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage;
    type OutgoingMessage = RelayMessage;

    async fn process_inbound<'a>(
        &'a self,
        ctx: &mut InboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        let message = match ctx.message {
            ClientMessage::Event(ref event) => match self.handle_event(event).await {
                Ok(Some(events_to_save)) => {
                    let event_id = event.id;
                    if let Err(e) = ctx.state.save_events(events_to_save).await {
                        e.handle_inbound_error(ctx).await;
                        return Ok(());
                    }
                    Some(RelayMessage::ok(event_id, true, ""))
                }
                Ok(None) => None,
                Err(e) => {
                    e.handle_inbound_error(ctx).await;
                    return Ok(());
                }
            },
            ClientMessage::Req { ref filters, .. } => {
                if let Err(e) = self.verify_filters(ctx.state.authed_pubkey, filters) {
                    e.handle_inbound_error(ctx).await;
                    return Ok(());
                }
                None
            }
            _ => None,
        };

        match message {
            Some(msg) => ctx.send_message(msg).await,
            None => ctx.next().await,
        }
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        if let Some(RelayMessage::Event { event, .. }) = &ctx.message {
            if let Some(group) = self.groups.find_group_from_event(event) {
                if !group.can_see_event(&ctx.state.authed_pubkey, event.kind) {
                    debug!(
                        "Not authorized to see event {}, kind {}",
                        event.id, event.kind
                    );

                    ctx.message = None;
                } else {
                    debug!("Authorized to see event {}, kind {}", event.id, event.kind);
                }
            }
        }

        ctx.next().await
    }
}
