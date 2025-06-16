/// Integration tests for RelayMiddleware with GroupsRelayProcessor
///
/// These tests mirror the existing Nip29Middleware tests to ensure identical behavior
/// during the hot-swap migration. Each test validates that RelayMiddleware + GroupsRelayProcessor
/// produces the same results as the original Nip29Middleware.
#[cfg(test)]
mod tests {
    use crate::groups_event_processor::GroupsRelayProcessor;
    use crate::test_utils::{create_test_event, create_test_keys, setup_test};
    use crate::{Groups, StoreCommand};
    use nostr_lmdb::Scope;
    use nostr_relay_builder::{
        EventContext, EventProcessor, NostrConnectionState, RelayDatabase, RelayMiddleware,
    };
    use nostr_sdk::prelude::*;
    use std::sync::Arc;

    /// Helper function to create a RelayMiddleware with GroupsRelayProcessor for testing
    async fn create_test_relay_middleware(
        database: Arc<RelayDatabase>,
        admin_pubkey: PublicKey,
    ) -> RelayMiddleware<GroupsRelayProcessor, ()> {
        let groups = Arc::new(
            Groups::load_groups(database.clone(), admin_pubkey)
                .await
                .unwrap(),
        );

        let groups_processor = GroupsRelayProcessor::new(groups, admin_pubkey);
        RelayMiddleware::new(groups_processor, admin_pubkey, database)
    }

    #[tokio::test]
    async fn test_relay_middleware_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware = create_test_relay_middleware(database, admin_keys.public_key()).await;

        // Create group content event without existing group (unmanaged group)
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(TagKind::h(), ["test_group"])],
        )
        .await;

        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_keys_pubkey = member_keys.public_key();

        let context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        let commands = middleware
            .processor()
            .handle_event(event.clone(), &mut (), context)
            .await
            .unwrap();

        // Should allow unmanaged group events
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(e, _) => assert_eq!(e.id, event.id),
            _ => panic!("Expected SaveSignedEvent"),
        }
    }

    #[tokio::test]
    async fn test_relay_middleware_can_see_event_unmanaged_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;

        let middleware = create_test_relay_middleware(database, admin_keys.public_key()).await;

        // Create an unmanaged group event
        let event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), ["unmanaged_group"])],
        )
        .await;

        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();

        // Non-member should be able to see unmanaged group events
        let non_member_keys_pubkey = non_member_keys.public_key();

        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_keys_pubkey),
            subdomain: state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&event, &(), non_member_context)
            .unwrap();
        assert!(can_see);

        // Anonymous should also see unmanaged group events
        let anon_context = EventContext {
            authed_pubkey: None,
            subdomain: state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&event, &(), anon_context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_relay_middleware_non_group_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

        // Create non-group event
        let event = create_test_event(&member_keys, Kind::TextNote.as_u16(), vec![]).await;

        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_keys_pubkey = member_keys.public_key();

        let context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        let commands = middleware
            .processor()
            .handle_event(event.clone(), &mut (), context)
            .await
            .unwrap();

        // Should save non-group events normally
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(e, scope) => {
                assert_eq!(e.id, event.id);
                assert_eq!(*scope, Scope::Default);
            }
            _ => panic!("Expected SaveSignedEvent"),
        }
    }

    #[tokio::test]
    async fn test_relay_middleware_can_see_non_group_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware = create_test_relay_middleware(database, admin_keys.public_key()).await;

        // Create non-group event
        let event = create_test_event(&member_keys, Kind::TextNote.as_u16(), vec![]).await;

        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();

        // Everyone should see non-group events
        let context = EventContext {
            authed_pubkey: None,
            subdomain: state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&event, &(), context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_relay_middleware_group_create_by_non_admin() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware = create_test_relay_middleware(database, admin_keys.public_key()).await;

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

        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_keys_pubkey = member_keys.public_key();

        let context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        let result = middleware
            .processor()
            .handle_event(create_event, &mut (), context)
            .await;

        // Should succeed - anyone can create new groups
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_relay_middleware_private_group_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_keys_pubkey = admin_keys.public_key();

        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_event, &mut (), admin_context)
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

        middleware
            .processor()
            .handle_event(add_event, &mut (), admin_context)
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
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&content_event, &(), non_member_context)
            .unwrap();
        assert!(!can_see);

        // Member should see private group events
        let member_keys_pubkey = member_keys.public_key();

        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&content_event, &(), member_context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_relay_middleware_verify_filters_private_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_keys_pubkey = admin_keys.public_key();

        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_event, &mut (), admin_context)
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

        middleware
            .processor()
            .handle_event(add_event, &mut (), admin_context)
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
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let result = middleware
            .processor()
            .verify_filters(&filters, &(), non_member_context);
        assert!(result.is_err());

        // Member should be able to query
        let member_keys_pubkey = member_keys.public_key();

        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let result = middleware
            .processor()
            .verify_filters(&filters, &(), member_context);
        assert!(result.is_ok());

        // Admin should be able to query
        let result = middleware
            .processor()
            .verify_filters(&filters, &(), admin_context);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_relay_middleware_public_group_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_keys_pubkey = admin_keys.public_key();

        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_event, &mut (), admin_context)
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
        middleware
            .processor()
            .handle_event(content_event.clone(), &mut (), admin_context)
            .await
            .unwrap();

        // Non-member SHOULD see public group events
        let non_member_keys_pubkey = non_member_keys.public_key();

        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&content_event, &(), non_member_context)
            .unwrap();
        assert!(can_see);

        // Anonymous should also see public group events
        let anon_context = EventContext {
            authed_pubkey: None,
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let can_see = middleware
            .processor()
            .can_see_event(&content_event, &(), anon_context)
            .unwrap();
        assert!(can_see);
    }

    #[tokio::test]
    async fn test_relay_middleware_member_management() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_keys_pubkey = admin_keys.public_key();

        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_event, &mut (), admin_context)
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

        let result = middleware
            .processor()
            .handle_event(add_event.clone(), &mut (), admin_context)
            .await;
        assert!(result.is_ok());

        // Try to add another member as non-admin should fail
        let member_keys_pubkey = member_keys.public_key();

        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
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

        let result = middleware
            .processor()
            .handle_event(add_event_by_member, &mut (), member_context)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_relay_middleware_event_deletion() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_keys_pubkey = admin_keys.public_key();

        let admin_context = EventContext {
            authed_pubkey: Some(&admin_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_event, &mut (), admin_context)
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

        middleware
            .processor()
            .handle_event(add_event, &mut (), admin_context)
            .await
            .unwrap();

        // Create content event
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        middleware
            .processor()
            .handle_event(content_event.clone(), &mut (), admin_context)
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

        let result = middleware
            .processor()
            .handle_event(delete_event, &mut (), admin_context)
            .await;
        assert!(result.is_ok());

        // Member can delete own event
        let member_keys_pubkey = member_keys.public_key();

        let member_context = EventContext {
            authed_pubkey: Some(&member_keys_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };
        let member_content = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        middleware
            .processor()
            .handle_event(member_content.clone(), &mut (), member_context)
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

        let result = middleware
            .processor()
            .handle_event(member_delete, &mut (), member_context)
            .await;
        // Current implementation: only admins can delete events (including group deletion events)
        // TODO: Should allow event authors to delete their own events
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_relay_middleware_visibility_member_can_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Create the group
        middleware
            .processor()
            .handle_event(create_group_event, &mut (), admin_context)
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

        middleware
            .processor()
            .handle_event(add_member_event, &mut (), admin_context)
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
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Member should be able to see the event
        let can_see = middleware
            .processor()
            .can_see_event(&group_event, &(), member_context)
            .unwrap();
        assert!(can_see, "Member should be able to see group events");
    }

    #[tokio::test]
    async fn test_relay_middleware_visibility_non_member_cannot_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_group_event, &mut (), admin_context)
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

        middleware
            .processor()
            .handle_event(add_member_event, &mut (), admin_context)
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
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Non-member should NOT be able to see private group events
        let can_see = middleware
            .processor()
            .can_see_event(&group_event, &(), non_member_context)
            .unwrap();
        assert!(!can_see, "Non-member should not see private group events");
    }

    #[tokio::test]
    async fn test_relay_middleware_visibility_relay_can_see_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

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

        let admin_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(&admin_pubkey),
            subdomain: admin_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        middleware
            .processor()
            .handle_event(create_group_event, &mut (), admin_context)
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
        let can_see = middleware
            .processor()
            .can_see_event(&group_event, &(), admin_context)
            .unwrap();
        assert!(can_see, "Relay should be able to see all events");
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_d_tag() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

        let member_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: member_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Create metadata filter with d tag (group creation events)
        let meta_filter = Filter::new()
            .kind(Kind::Custom(9007)) // KIND_GROUP_CREATE_9007
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group");

        // Test filter verification - should pass for metadata queries
        let result = middleware
            .processor()
            .verify_filters(&[meta_filter], &(), member_context);

        assert!(
            result.is_ok(),
            "Filter verification should pass for metadata queries with d-tag"
        );
    }

    #[tokio::test]
    async fn test_filter_verification_metadata_filter_with_addressable_kind() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

        let member_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: member_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Create filter for addressable events (30000-39999 range)
        let addressable_filter = Filter::new()
            .kind(Kind::Custom(39000)) // KIND_GROUP_METADATA_39000
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group");

        // Test filter verification - should pass for addressable queries
        let result =
            middleware
                .processor()
                .verify_filters(&[addressable_filter], &(), member_context);

        assert!(
            result.is_ok(),
            "Filter verification should pass for addressable events"
        );
    }

    #[tokio::test]
    async fn test_relay_middleware_filter_verification_non_existing_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

        let member_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: member_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Create filter for non-existing group
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote]).custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            "non_existing_group",
        )];

        // Should pass because unmanaged groups are allowed
        let result = middleware
            .processor()
            .verify_filters(&filters, &(), member_context);
        assert!(
            result.is_ok(),
            "Non-existing groups should be allowed (unmanaged groups)"
        );
    }

    #[tokio::test]
    async fn test_relay_middleware_filter_verification_non_group_query() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

        let member_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: member_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Create filter without group tags (non-group query)
        let filters = vec![Filter::new().kinds(vec![Kind::TextNote])];

        // Should always pass for non-group queries
        let result = middleware
            .processor()
            .verify_filters(&filters, &(), member_context);
        assert!(result.is_ok(), "Non-group queries should always be allowed");
    }

    #[tokio::test]
    async fn test_relay_middleware_allowed_non_group_content_event_without_group() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware = create_test_relay_middleware(database, admin_keys.public_key()).await;

        // Create non-group content event (kind 1 without h-tag)
        let event = create_test_event(&member_keys, 1, vec![]).await;

        let member_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: member_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        let commands = middleware
            .processor()
            .handle_event(event.clone(), &mut (), member_context)
            .await
            .unwrap();

        // Should accept non-group events
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(e, _) => assert_eq!(e.id, event.id),
            _ => panic!("Expected SaveSignedEvent"),
        }
    }

    #[tokio::test]
    async fn test_group_create_with_existing_events_requires_relay_admin() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let (_, member_keys, _) = create_test_keys().await;

        let middleware =
            create_test_relay_middleware(database.clone(), admin_keys.public_key()).await;

        let group_id = "existing_events_group";

        // First, create some unmanaged group content
        let unmanaged_event = create_test_event(
            &member_keys,
            11, // Group content
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        let member_state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: member_state.subdomain(),
            relay_pubkey: middleware.processor().relay_pubkey(),
        };

        // Save the unmanaged event directly to database (like the old test)
        database
            .save_signed_event(unmanaged_event.clone(), member_state.subdomain().clone())
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

        let result = middleware
            .processor()
            .handle_event(create_event, &mut (), member_context)
            .await;

        // Should fail because only relay admin can convert unmanaged to managed
        assert!(
            result.is_err(),
            "Only relay admin should be able to convert unmanaged groups to managed"
        );
    }
}
