use crate::error::Error;
use crate::groups::group::{
    ADDRESSABLE_EVENT_KINDS, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022, NON_GROUP_ALLOWED_KINDS,
};
use crate::nostr_session_state::NostrConnectionState;
use crate::Groups;
use crate::StoreCommand;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use std::sync::Arc;
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

    async fn handle_event(
        &self,
        event: &Event,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Option<Vec<StoreCommand>>, Error> {
        if event.kind == KIND_GROUP_CREATE_9007 {
            let group = self.groups.handle_group_create(event).await?;

            let metadata_event = group.generate_metadata_event();
            let put_user_event = group.generate_put_user_event(&event.pubkey);
            let admins_event = group.generate_admins_event();
            let members_event = group.generate_members_event();
            let roles_event = group.generate_roles_event();

            return Ok(Some(vec![
                StoreCommand::SaveSignedEvent(event.clone()),
                StoreCommand::SaveUnsignedEvent(metadata_event.build(self.relay_pubkey)),
                StoreCommand::SaveUnsignedEvent(put_user_event.build(self.relay_pubkey)),
                StoreCommand::SaveUnsignedEvent(admins_event.build(self.relay_pubkey)),
                StoreCommand::SaveUnsignedEvent(members_event.build(self.relay_pubkey)),
                StoreCommand::SaveUnsignedEvent(roles_event.build(self.relay_pubkey)),
            ]));
        }

        let events_to_save = match event.kind {
            k if k == KIND_GROUP_EDIT_METADATA_9002 => {
                self.groups.handle_edit_metadata(event)?;
                let Some(group) = self.groups.find_group_from_event(event) else {
                    return Ok(None);
                };
                let metadata_event = group.generate_metadata_event();
                vec![
                    StoreCommand::SaveSignedEvent(event.clone()),
                    StoreCommand::SaveUnsignedEvent(metadata_event.build(self.relay_pubkey)),
                ]
            }

            k if k == KIND_GROUP_USER_JOIN_REQUEST_9021 => {
                let auto_joined = self.groups.handle_join_request(event)?;
                if auto_joined {
                    let Some(group) = self.groups.find_group_from_event(event) else {
                        return Err(Error::notice("Group not found"));
                    };
                    let put_user_event = group.generate_put_user_event(&event.pubkey);
                    let members_event = group.generate_members_event();
                    vec![
                        StoreCommand::SaveSignedEvent(event.clone()),
                        StoreCommand::SaveUnsignedEvent(put_user_event.build(self.relay_pubkey)),
                        StoreCommand::SaveUnsignedEvent(members_event.build(self.relay_pubkey)),
                    ]
                } else {
                    vec![StoreCommand::SaveSignedEvent(event.clone())]
                }
            }

            k if k == KIND_GROUP_USER_LEAVE_REQUEST_9022 => {
                if self.groups.handle_leave_request(event)? {
                    let Some(group) = self.groups.find_group_from_event(event) else {
                        return Err(Error::notice("Group not found"));
                    };
                    let members_event = group.generate_members_event();
                    vec![
                        StoreCommand::SaveSignedEvent(event.clone()),
                        StoreCommand::SaveUnsignedEvent(members_event.build(self.relay_pubkey)),
                    ]
                } else {
                    vec![]
                }
            }

            k if k == KIND_GROUP_SET_ROLES_9006 => {
                self.groups.handle_set_roles(event)?;
                vec![]
            }

            k if k == KIND_GROUP_ADD_USER_9000 => {
                self.groups.handle_put_user(event)?;
                let Some(group) = self.groups.find_group_from_event(event) else {
                    return Err(Error::notice("Group not found"));
                };
                let mut events = vec![StoreCommand::SaveSignedEvent(event.clone())];

                let admins_event = group.generate_admins_event();
                events.push(StoreCommand::SaveUnsignedEvent(
                    admins_event.build(self.relay_pubkey),
                ));

                let members_event = group.generate_members_event();
                events.push(StoreCommand::SaveUnsignedEvent(
                    members_event.build(self.relay_pubkey),
                ));
                events
            }

            k if k == KIND_GROUP_REMOVE_USER_9001 => {
                let removed_admins = self.groups.handle_remove_user(event)?;
                let Some(group) = self.groups.find_group_from_event(event) else {
                    return Err(Error::notice("Group not found"));
                };
                let mut events = vec![StoreCommand::SaveSignedEvent(event.clone())];
                if removed_admins {
                    let admins_event = group.generate_admins_event();
                    events.push(StoreCommand::SaveUnsignedEvent(
                        admins_event.build(self.relay_pubkey),
                    ));
                }
                let members_event = group.generate_members_event();
                events.push(StoreCommand::SaveUnsignedEvent(
                    members_event.build(self.relay_pubkey),
                ));
                events
            }

            k if k == KIND_GROUP_DELETE_9008 => {
                let Some(group) = self.groups.find_group_from_event(event) else {
                    return Err(Error::notice("Group not found"));
                };

                match group.delete_group_request(event, &self.relay_pubkey, authed_pubkey) {
                    Ok(commands) => commands,
                    Err(e) => return Err(e),
                }
            }

            k if k == KIND_GROUP_DELETE_EVENT_9005 => {
                let Some(group) = self.groups.find_group_from_event(event) else {
                    return Err(Error::notice("Group not found"));
                };

                match group.delete_event_request(event, &self.relay_pubkey, authed_pubkey) {
                    Ok(commands) => commands,
                    Err(e) => return Err(e),
                }
            }

            k if k == KIND_GROUP_CREATE_INVITE_9009 => {
                self.groups.handle_create_invite(event)?;
                vec![StoreCommand::SaveSignedEvent(event.clone())]
            }

            k if !NON_GROUP_ALLOWED_KINDS.contains(&k) => {
                let mut group = self
                    .groups
                    .find_group_from_event_mut(event)?
                    .ok_or(Error::notice("Group not found for this group content"))?;

                let is_member = group.is_member(&event.pubkey);
                let mut events_to_save = vec![StoreCommand::SaveSignedEvent(event.clone())];

                // For private and closed groups, only members can post
                if group.metadata.private && group.metadata.closed && !is_member {
                    return Err(Error::notice("User is not a member of this group"));
                }

                // Open groups auto-join the author when posting
                if !group.metadata.closed {
                    // For open groups, non-members are automatically added
                    if !is_member {
                        group.add_pubkey(event.pubkey);

                        let put_user_event = group.generate_put_user_event(&event.pubkey);
                        let members_event = group.generate_members_event();

                        events_to_save.push(StoreCommand::SaveUnsignedEvent(
                            put_user_event.build(self.relay_pubkey),
                        ));
                        events_to_save.push(StoreCommand::SaveUnsignedEvent(
                            members_event.build(self.relay_pubkey),
                        ));
                    }
                } else if !is_member {
                    // For closed groups, non-members can't post
                    return Err(Error::notice("User is not a member of this group"));
                }

                events_to_save
            }

            _ => {
                // We always save allowed non-group events
                vec![StoreCommand::SaveSignedEvent(event.clone())]
            }
        };

        Ok(Some(events_to_save))
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
            // Check kinds in reverse order to catch addressable kinds first
            for k in kinds.iter().rev() {
                if ADDRESSABLE_EVENT_KINDS.contains(k) {
                    is_meta = true;
                } else if is_meta {
                    // This was taken from relay29. I still unsure why this was done so I'm commenting until I know why we don't let a mixed query
                    // return Err(Error::notice(
                    //     "Invalid query, cannot mix metadata and normal event kinds",
                    // ));
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
                let Some(group) = self.groups.get_group(tag) else {
                    return Ok(());
                };

                if !group.metadata.private {
                    return Ok(());
                }

                match authed_pubkey {
                    Some(authed_pubkey) => {
                        // relay pubkey can always read private groups
                        if authed_pubkey == self.relay_pubkey {
                            return Ok(());
                        }

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
        let response_message = match &ctx.message {
            ClientMessage::Event(ref event) => {
                match self.handle_event(event, &ctx.state.authed_pubkey).await {
                    Ok(Some(events_to_save)) => {
                        let event_id = event.id;
                        if let Err(e) = ctx.state.save_events(events_to_save).await {
                            e.handle_inbound_error(ctx).await;
                            return Ok(());
                        }
                        Some(RelayMessage::ok(event_id, true, ""))
                    }
                    Ok(None) => return Ok(()),
                    Err(e) => {
                        e.handle_inbound_error(ctx).await;
                        return Ok(());
                    }
                }
            }
            ClientMessage::Req {
                ref filters,
                subscription_id: _,
            } => {
                if let Err(e) = self.verify_filters(ctx.state.authed_pubkey, filters) {
                    e.handle_inbound_error(ctx).await;
                    return Ok(());
                }
                None
            }
            _ => None,
        };

        match response_message {
            Some(relay_message) => ctx.send_message(relay_message).await,
            None => ctx.next().await,
        }
    }

    async fn process_outbound<'a>(
        &'a self,
        ctx: &mut OutboundContext<'a, Self::State, Self::IncomingMessage, Self::OutgoingMessage>,
    ) -> Result<()> {
        if let Some(RelayMessage::Event { event, .. }) = &ctx.message {
            if let Some(group) = self.groups.find_group_from_event(event) {
                match group.can_see_event(&ctx.state.authed_pubkey, &self.relay_pubkey, event) {
                    Ok(false) => ctx.message = None,
                    Err(_e) => ctx.message = None,
                    _ => (),
                }
            }
        }

        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, create_test_keys, create_test_state, setup_test};
    use websocket_builder::OutboundContext;

    #[tokio::test]
    async fn test_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database, admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups, admin_keys.public_key());

        // Create a content event for a non-existent group
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(TagKind::h(), ["non_existent_group"])],
        )
        .await;

        // Should return an error because group doesn't exist
        let result = middleware.handle_event(&event, &None).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Group not found for this group content"
        );
    }

    #[tokio::test]
    async fn test_allowed_non_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database, admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups, admin_keys.public_key());

        // Create a content event for a non-existent group
        let event = create_test_event(
            &member_keys,
            10009, // This event doesn't need an 'h' tag
            vec![],
        )
        .await;

        // Should not return an error because group is not needed here
        let result = middleware.handle_event(&event, &None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_process_outbound_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database, admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key());

        // Create a group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007, // KIND_GROUP_CREATE
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Add member to group
        let add_member_event = create_test_event(
            &admin_keys,
            9008, // KIND_GROUP_ADD_USER
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(&add_member_event).unwrap();

        // Create a group content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Test member can see event
        let mut state = create_test_state(Some(member_keys.public_key()));
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: SubscriptionId::new("test"),
                event: Box::new(content_event.clone()),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_some());

        // Test non-member cannot see event
        let mut state = create_test_state(Some(non_member_keys.public_key()));
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: SubscriptionId::new("test"),
                event: Box::new(content_event.clone()),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_none());

        // Test relay pubkey can see event
        let mut state = create_test_state(Some(admin_keys.public_key()));
        let mut ctx = create_test_context(
            &mut state,
            RelayMessage::Event {
                subscription_id: SubscriptionId::new("test"),
                event: Box::new(content_event),
            },
        );
        middleware.process_outbound(&mut ctx).await.unwrap();
        assert!(ctx.message.is_some());
    }

    #[tokio::test]
    async fn test_filter_verification() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database, admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key());

        // Create a test group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Add member to group
        let add_member_event = create_test_event(
            &admin_keys,
            9008,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(&add_member_event).unwrap();

        // Normal filter with 'h' tag
        let normal_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), vec![group_id]);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &normal_filter)
            .is_ok());

        let non_existing_group_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            vec!["non_existing_group"],
        );
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &non_existing_group_filter)
            .is_ok());

        // Metadata filter with 'd' tag
        let meta_filter = Filter::new()
            .kind(Kind::Custom(9007))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec![group_id]);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &meta_filter)
            .is_ok());

        // Reference filter with 'e' tag
        let ref_filter = Filter::new()
            .kind(Kind::Custom(11))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::E), vec!["test_id"]);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &ref_filter)
            .is_ok());

        // Reference filter with authors
        let author_filter = Filter::new()
            .kind(Kind::Custom(11))
            .authors(vec![member_keys.public_key()]);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &author_filter)
            .is_ok());

        // Metadata filter with addressable kind
        let meta_filter = Filter::new()
            .kinds(vec![Kind::Custom(39000)]) // Just the addressable kind
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), vec![group_id]);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &meta_filter)
            .is_ok());

        // Normal filter with non-addressable kind
        let normal_filter = Filter::new()
            .kinds(vec![Kind::Custom(11)]) // Just the normal kind
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), vec![group_id]);
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &normal_filter)
            .is_ok());

        let private_group_id = "private_group";
        let private_create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::custom(TagKind::p(), ["true"]),
            ],
        )
        .await;
        groups
            .handle_group_create(&private_create_event)
            .await
            .unwrap();

        let add_to_private_event = create_test_event(
            &admin_keys,
            9008,
            vec![
                Tag::custom(TagKind::h(), [private_group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(&add_to_private_event).unwrap();

        let private_filter = Filter::new().kind(Kind::Custom(11)).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            vec![private_group_id],
        );
        assert!(middleware
            .verify_filter(Some(member_keys.public_key()), &private_filter)
            .is_ok());

        // Private group access - non-member
        assert!(middleware
            .verify_filter(Some(non_member_keys.public_key()), &private_filter)
            .is_err());

        // Private group access - no auth
        assert!(middleware.verify_filter(None, &private_filter).is_err());

        // Private group access - relay pubkey
        assert!(middleware
            .verify_filter(Some(admin_keys.public_key()), &private_filter)
            .is_ok());
    }

    #[tokio::test]
    async fn test_auto_add_member_on_post() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database, admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key());

        // Create a group
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Set group as not closed and public
        let metadata_event = create_test_event(
            &admin_keys,
            9002,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::custom("open"), [""]),
                Tag::custom(TagKind::custom("public"), [""]),
            ],
        )
        .await;
        groups.handle_edit_metadata(&metadata_event).unwrap();

        // Verify group settings
        let group = groups.get_group(group_id).unwrap();
        assert!(!group.metadata.closed, "Group should not be closed");
        assert!(!group.metadata.private, "Group should not be private");
        assert!(!group.is_member(&member_keys.public_key()));
        drop(group); // Release the lock

        // Post content to the group as non-member
        let content_event =
            create_test_event(&member_keys, 7, vec![Tag::custom(TagKind::h(), [group_id])]).await;

        // Handle the event
        let result = middleware.handle_event(&content_event, &None).await;
        assert!(result.is_ok());

        // Verify the events to save contain both the content event and member update
        let events_to_save = result.unwrap().unwrap();
        assert_eq!(events_to_save.len(), 3);
        match &events_to_save[0] {
            StoreCommand::SaveSignedEvent(event) => assert_eq!(event.id, content_event.id),
            _ => panic!("Expected SaveSignedEvent"),
        }
        match &events_to_save[1] {
            StoreCommand::SaveUnsignedEvent(_) => (), // This is the put user event
            _ => panic!("Expected SaveUnsignedEvent for put user event"),
        }
        match &events_to_save[2] {
            StoreCommand::SaveUnsignedEvent(_) => (), // This is the members event
            _ => panic!("Expected SaveUnsignedEvent for members update"),
        }

        // Verify member was added to the group
        let group = groups.get_group(group_id).unwrap();
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_no_auto_add_member_on_post_to_closed_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;
        let groups = Arc::new(
            Groups::load_groups(database, admin_keys.public_key())
                .await
                .unwrap(),
        );
        let middleware = Nip29Middleware::new(groups.clone(), admin_keys.public_key());

        // Create a group (closed by default)
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;
        groups.handle_group_create(&create_event).await.unwrap();

        // Set group as public but keep it closed
        let metadata_event = create_test_event(
            &admin_keys,
            9002,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::custom("closed"), [""]),
                Tag::custom(TagKind::custom("public"), [""]),
            ],
        )
        .await;
        groups.handle_edit_metadata(&metadata_event).unwrap();

        // Verify group settings
        let group = groups.get_group(group_id).unwrap();
        assert!(group.metadata.closed, "Group should be closed");
        assert!(!group.metadata.private, "Group should not be private");
        assert!(!group.is_member(&member_keys.public_key()));
        drop(group);

        // Try to post content to the group as non-member
        let content_event =
            create_test_event(&member_keys, 7, vec![Tag::custom(TagKind::h(), [group_id])]).await;

        // Handle the event - should fail because user is not a member
        let result = middleware.handle_event(&content_event, &None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "Restricted: User {} is not a member of this group",
                member_keys.public_key
            )
        );

        // Verify member was not added to the group
        let group = groups.get_group(group_id).unwrap();
        assert!(!group.is_member(&member_keys.public_key()));
    }

    fn create_test_context<'a>(
        state: &'a mut NostrConnectionState,
        message: RelayMessage,
    ) -> OutboundContext<'a, NostrConnectionState, ClientMessage, RelayMessage> {
        OutboundContext::new("test_conn".to_string(), message, None, state, &[], 0)
    }
}
