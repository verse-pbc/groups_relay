/// Integration tests for GroupsRelayProcessor
///
/// These tests validate the EventProcessor implementation directly
/// without going through RelayMiddleware which is now pub(crate).
#[cfg(test)]
mod integration_tests {
    use crate::groups::Groups;
    use crate::groups_event_processor::GroupsRelayProcessor;
    use crate::test_utils::*;
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
    ) -> GroupsRelayProcessor {
        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                admin_keys.public_key(),
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        GroupsRelayProcessor::new(groups, admin_keys.public_key())
    }

    /// Test that GroupsRelayProcessor can process events correctly
    #[tokio::test]
    async fn test_event_processing() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let processor = create_test_processor(database.clone(), admin_keys.clone()).await;
        let relay_pubkey = admin_keys.public_key();

        // Test group creation
        let group_id = "test_group";
        let create_event = create_test_event(
            &admin_keys,
            9007,
            vec![
                Tag::custom(TagKind::h(), [group_id]),
                Tag::custom(TagKind::d(), [group_id]),
                Tag::custom(TagKind::Custom("name".into()), ["Test Group"]),
                Tag::custom(TagKind::Custom("public".into()), [""]),
            ],
        )
        .await;

        // Process event
        let admin_pubkey = admin_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(admin_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };
        let store_commands = processor
            .handle_event(create_event, empty_state(), &context)
            .await
            .unwrap();

        // Should have multiple store commands (group creation + metadata events)
        assert!(store_commands.len() > 1);

        // Verify we have the expected state events generated
        let mut has_metadata = false;
        let mut has_admins = false;
        let mut has_members = false;

        for cmd in &store_commands {
            match cmd {
                relay_builder::StoreCommand::SaveUnsignedEvent(evt, _, _) => {
                    match evt.kind.as_u16() {
                        39000 => has_metadata = true, // Group metadata
                        39001 => has_admins = true,   // Group admins
                        39002 => has_members = true,  // Group members
                        _ => {}
                    }
                }
                relay_builder::StoreCommand::SaveSignedEvent(evt, _, _) => {
                    // The original 9007 event should be saved
                    assert_eq!(evt.kind.as_u16(), 9007);
                }
                _ => {}
            }
        }

        assert!(has_metadata, "Should generate 39000 (metadata) event");
        assert!(has_admins, "Should generate 39001 (admins) event");
        assert!(has_members, "Should generate 39002 (members) event");
    }

    /// Test filter verification and access control
    #[tokio::test]
    async fn test_filter_verification() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let processor = create_test_processor(database.clone(), admin_keys.clone()).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a private group
        let group_id = "private_group";
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

        let admin_pubkey = admin_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(admin_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };
        processor
            .handle_event(create_event, empty_state(), &context)
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
            .handle_event(add_event, empty_state(), &context)
            .await
            .unwrap();

        // Test filter verification for member
        let filters = vec![Filter::new()
            .kinds(vec![Kind::TextNote])
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), group_id)];

        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(member_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };

        // Member should be able to query
        let result = processor.verify_filters(&filters, empty_state(), &member_context);
        assert!(result.is_ok());

        // Non-member should not be able to query
        let non_member_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(non_member_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };
        let result = processor.verify_filters(&filters, empty_state(), &non_member_context);
        assert!(result.is_err());
    }

    /// Test event visibility in groups
    #[tokio::test]
    async fn test_event_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let processor = create_test_processor(database.clone(), admin_keys.clone()).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a private group
        let group_id = "visibility_test";
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

        let admin_pubkey = admin_keys.public_key();
        let admin_context = EventContext {
            authed_pubkey: Some(admin_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };
        processor
            .handle_event(create_event, empty_state(), &admin_context)
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
            .handle_event(add_event, empty_state(), &admin_context)
            .await
            .unwrap();

        // Create group content
        let content_event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [group_id])],
        )
        .await;

        // Test visibility for admin
        let can_see = processor
            .can_see_event(&content_event, empty_state(), &admin_context)
            .unwrap();
        assert!(can_see);

        // Test visibility for member
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(member_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };
        let can_see = processor
            .can_see_event(&content_event, empty_state(), &member_context)
            .unwrap();
        assert!(can_see);

        // Test visibility for non-member
        let non_member_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(non_member_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey,
        };
        let can_see = processor
            .can_see_event(&content_event, empty_state(), &non_member_context)
            .unwrap();
        assert!(!can_see);
    }
}
