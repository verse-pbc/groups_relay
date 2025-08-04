/// Unit tests for GroupsRelayProcessor
///
/// These tests directly test the GroupsRelayProcessor implementation
/// without going through RelayMiddleware, which is now pub(crate).
#[cfg(test)]
mod tests {
    use crate::groups_event_processor::GroupsRelayProcessor;
    use crate::test_utils::{create_test_event, create_test_keys, setup_test_with_sender};
    use crate::{Groups, StoreCommand};
    use nostr_lmdb::Scope;
    use nostr_sdk::prelude::*;
    use parking_lot::RwLock;
    use relay_builder::{EventContext, EventProcessor};
    use std::sync::Arc;

    fn empty_state() -> Arc<RwLock<()>> {
        Arc::new(RwLock::new(()))
    }

    async fn create_test_processor(
        database: Arc<crate::RelayDatabase>,
        admin_keys: Keys,
    ) -> (GroupsRelayProcessor, Arc<crate::groups::Groups>) {
        let groups_arc = Arc::new(
            Groups::load_groups(
                database.clone(),
                admin_keys.public_key(),
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let groups_processor =
            GroupsRelayProcessor::new(groups_arc.clone(), admin_keys.public_key());

        (groups_processor, groups_arc)
    }
    #[tokio::test]
    async fn test_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create group content event without existing group (unmanaged group)
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(TagKind::h(), ["test_group"])],
        )
        .await;

        let member_keys_pubkey = member_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        let commands = processor
            .handle_event(event.clone(), empty_state(), context)
            .await
            .unwrap();

        // Should allow unmanaged group events
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(e, _, _) => assert_eq!(e.id, event.id),
            _ => panic!("Expected SaveSignedEvent"),
        }
    }

    #[tokio::test]
    async fn test_can_see_event_unmanaged_group() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create an unmanaged group event
        let event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), ["unmanaged_group"])],
        )
        .await;

        // Non-member should be able to see unmanaged group events
        let non_member_keys_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&event, empty_state(), non_member_context)
            .unwrap();
        assert!(can_see);

        // Anonymous should also see unmanaged group events
        let anon_context = EventContext {
            authed_pubkey: None,
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&event, empty_state(), anon_context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_non_group_event() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create non-group event
        let event = create_test_event(&member_keys, Kind::TextNote.as_u16(), vec![]).await;

        let member_keys_pubkey = member_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        let commands = processor
            .handle_event(event.clone(), empty_state(), context)
            .await
            .unwrap();

        // Should save non-group events normally
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(e, scope, _) => {
                assert_eq!(e.id, event.id);
                assert_eq!(*scope, Scope::Default);
            }
            _ => panic!("Expected SaveSignedEvent"),
        }
    }

    #[tokio::test]
    async fn test_can_see_non_group_event() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create non-group event
        let event = create_test_event(&member_keys, Kind::TextNote.as_u16(), vec![]).await;

        // Everyone should see non-group events
        let context = EventContext {
            authed_pubkey: None,
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&event, empty_state(), context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_group_create_by_non_admin() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Try to create group as non-admin
        let create_event = create_test_event(
            &member_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), ["test_group"]),
                Tag::custom(TagKind::d(), ["test_group"]),
            ],
        )
        .await;

        let member_keys_pubkey = member_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        let result = processor
            .handle_event(create_event, empty_state(), context)
            .await;

        // Should succeed - anyone can create new groups
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_private_group_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create private group
        let group_id = "private_test";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("private".into()), [""]),
            ],
        )
        .await;

        let admin_keys_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Add member
        let add_event = create_test_event(
            &admin_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        processor
            .handle_event(add_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create group content
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Non-member should NOT see private group events
        let non_member_keys_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&content_event, empty_state(), non_member_context)
            .unwrap();
        assert!(!can_see);

        // Member should see private group events
        let member_keys_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&content_event, empty_state(), member_context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_verify_filters_private_group() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create private group
        let group_id = "private_filters_test";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("private".into()), [""]),
            ],
        )
        .await;

        let admin_keys_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Add member
        let add_event = create_test_event(
            &admin_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        processor
            .handle_event(add_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create filter for private group
        let filters = vec![Filter::new()
            .kinds(vec![Kind::TextNote])
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), group_id)];

        // Non-member should NOT be able to query private group
        let non_member_keys_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let result = processor
            .verify_filters(&filters, empty_state(), non_member_context);
        assert!(result.is_err());

        // Member should be able to query
        let member_keys_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let result = processor
            .verify_filters(&filters, empty_state(), member_context);
        assert!(result.is_ok());

        // Admin should be able to query
        let result = processor
            .verify_filters(&filters, empty_state(), admin_context);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_public_group_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create public open group (public = readable by all, open = auto-join on posting)
        let group_id = "public_test";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("public".into()), [""]),
                Tag::custom(TagKind::Custom("open".into()), [""]),
            ],
        )
        .await;

        let admin_keys_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create group content - this should auto-join the member in a public group
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Store the event first
        processor
            .handle_event(content_event.clone(), empty_state(), admin_context)
            .await
            .unwrap();

        // Non-member SHOULD see public group events
        let non_member_keys_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&content_event, empty_state(), non_member_context)
            .unwrap();
        assert!(can_see);

        // Anonymous should also see public group events
        let anon_context = EventContext {
            authed_pubkey: None,
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let can_see = processor
            .can_see_event(&content_event, empty_state(), anon_context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_member_management() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create group
        let group_id = "member_test";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
            ],
        )
        .await;

        let admin_keys_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Add member as admin should succeed
        let add_event = create_test_event(
            &admin_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        let result = processor
            .handle_event(add_event.clone(), empty_state(), admin_context)
            .await;
        assert!(result.is_ok());

        // Try to add another member as non-admin should fail
        let member_keys_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let add_event_by_member = create_test_event(
            &member_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(admin_keys.public_key()),
            ],
        )
        .await;

        let result = processor
            .handle_event(add_event_by_member, empty_state(), member_context)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_event_deletion() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create group and add member
        let group_id = "deletion_test";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
            ],
        )
        .await;

        let admin_keys_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Add member
        let add_event = create_test_event(
            &admin_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        processor
            .handle_event(add_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        processor
            .handle_event(content_event.clone(), empty_state(), admin_context)
            .await
            .unwrap();

        // Admin can delete any event
        let delete_event = create_test_event(
            &admin_keys,
            9005,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::event(content_event.id),
            ],
        )
        .await;

        let result = processor
            .handle_event(delete_event, empty_state(), admin_context)
            .await;
        assert!(result.is_ok());

        // Member can delete own event
        let member_keys_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };
        let member_content = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        processor
            .handle_event(member_content.clone(), empty_state(), member_context)
            .await
            .unwrap();

        let member_delete = create_test_event(
            &member_keys,
            9005,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::event(member_content.id),
            ],
        )
        .await;

        let result = processor
            .handle_event(member_delete, empty_state(), member_context)
            .await;
        // Current implementation: only admins can delete events (including group deletion events)
        // TODO: Should allow event authors to delete their own events
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_visibility_member_can_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create a group first
        let group_id = "test_group";
        let create_group_event = create_test_event(
            &admin_keys,
            9007, // Group creation
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("name".into()), ["Test Group"]),
            ],
        )
        .await;

        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Create the group
        processor
            .handle_event(create_group_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Add member to the group
        let add_member_event = create_test_event(
            &admin_keys,
            9000, // Add user
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        processor
            .handle_event(add_member_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create group content event
        let group_event = create_test_event(
            &member_keys,
            11, // Group content
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Member should be able to see the event
        let can_see = processor
            .can_see_event(&group_event, empty_state(), member_context)
            .unwrap();
        assert!(can_see, "Member should be able to see group events");
    }

    #[tokio::test]
    async fn test_visibility_non_member_cannot_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create a private group
        let group_id = "private_test_group";
        let create_group_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("private".into()), [""]),
                Tag::custom(TagKind::Custom("closed".into()), [""]),
            ],
        )
        .await;

        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_group_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Add only one member (not the non_member)
        let add_member_event = create_test_event(
            &admin_keys,
            9000,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        processor
            .handle_event(add_member_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create group content event
        let group_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        let non_member_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Non-member should NOT be able to see private group events
        let can_see = processor
            .can_see_event(&group_event, empty_state(), non_member_context)
            .unwrap();
        assert!(!can_see, "Non-member should not see private group events");
    }

    #[tokio::test]
    async fn test_visibility_relay_can_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create a private group
        let group_id = "relay_test_group";
        let create_group_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("private".into()), [""]),
            ],
        )
        .await;

        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        processor
            .handle_event(create_group_event, empty_state(), admin_context)
            .await
            .unwrap();

        // Create group content event
        let group_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Relay (admin in this case) should be able to see all events
        let can_see = processor
            .can_see_event(&group_event, empty_state(), admin_context)
            .unwrap();
        assert!(can_see, "Relay should be able to see all events");
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_d_tag() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Create metadata filter with d tag (group creation events)
        let meta_filter = Filter::new()
            .kind(Kind::Custom(9007)) // KIND_GROUP_CREATE_9007
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group");

        // Test filter verification - should pass for metadata queries
        let result = processor
            .verify_filters(&[meta_filter], empty_state(), member_context);

        assert!(
            result.is_ok(),
            "Filter verification should pass for metadata queries with d-tag"
        );
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_addressable_kind() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Create filter for addressable events (30000-39999 range)
        let addressable_filter = Filter::new()
            .kind(Kind::Custom(39000)) // KIND_GROUP_METADATA_39000
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group");

        // Test filter verification - should pass for addressable queries
        let result = processor.verify_filters(
            &[addressable_filter],
            empty_state(),
            member_context,
        );

        assert!(
            result.is_ok(),
            "Filter verification should pass for addressable events"
        );
    }

    #[tokio::test]
    async fn test_filter_verification_non_existing_group() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Create filter for non-existing group
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote]).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            "non_existing_group",
        )];

        // Should pass because unmanaged groups are allowed
        let result = processor
            .verify_filters(&filters, empty_state(), member_context);
        assert!(
            result.is_ok(),
            "Non-existing groups should be allowed (unmanaged groups)"
        );
    }

    #[tokio::test]
    async fn test_filter_verification_non_group_query() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Create filter without group tags (non-group query)
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote])];

        // Should always pass for non-group queries
        let result = processor
            .verify_filters(&filters, empty_state(), member_context);
        assert!(result.is_ok(), "Non-group queries should always be allowed");
    }

    #[tokio::test]
    async fn test_allowed_non_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database, admin_keys.clone()).await;

        // Create non-group content event (kind 1 without h-tag)
        let event = create_test_event(&member_keys, 1, vec![]).await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        let commands = processor
            .handle_event(event.clone(), empty_state(), member_context)
            .await
            .unwrap();

        // Should accept non-group events
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(e, _, _) => assert_eq!(e.id, event.id),
            _ => panic!("Expected SaveSignedEvent"),
        }
    }

    #[tokio::test]
    async fn test_group_create_with_existing_events_requires_relay_admin() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, _) = create_test_keys().await;
        let (processor, _) = create_test_processor(database.clone(), admin_keys.clone()).await;

        let group_id = "existing_events_group";

        // First, create some unmanaged group content
        let unmanaged_event = create_test_event(
            &member_keys,
            11, // Group content
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &Scope::Default,
            relay_pubkey: &admin_keys.public_key(),
        };

        // Save the unmanaged event directly to database (like the old test)
        database
            .save_event(&unmanaged_event, &Scope::Default)
            .await
            .unwrap();

        // Add a small delay to ensure the save completes
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;

        // Now try to create a managed group with the same ID as non-admin
        let create_event = create_test_event(
            &member_keys,
            9007, // Group creation
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
            ],
        )
        .await;

        let result = processor
            .handle_event(create_event, empty_state(), member_context)
            .await;

        // Should fail because only relay admin can convert unmanaged to managed
        assert!(
            result.is_err(),
            "Only relay admin should be able to convert unmanaged groups to managed"
        );
    }
}
