#[cfg(test)]
mod integration_tests {
    use crate::groups::Groups;
    use crate::groups_event_processor::GroupsRelayProcessor;
    use crate::test_utils::*;
    use nostr_lmdb::Scope;
    use relay_builder::{
        CryptoHelper, EventContext, EventProcessor, NostrConnectionState, RelayMiddleware,
        SubscriptionRegistry,
    };
    use nostr_sdk::prelude::*;
    use parking_lot::RwLock;
    use std::sync::Arc;

    fn empty_state() -> Arc<RwLock<()>> {
        Arc::new(RwLock::new(()))
    }

    /// Test that RelayMiddleware can process events correctly
    #[tokio::test]
    async fn test_relay_middleware_event_processing() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let relay_pubkey = admin_keys.public_key();

        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                relay_pubkey,
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let groups_processor = GroupsRelayProcessor::new(groups.clone(), relay_pubkey);
        let registry = Arc::new(SubscriptionRegistry::new(None));
        let relay_url = RelayUrl::parse("wss://test.relay.com").unwrap();
        let crypto_helper = CryptoHelper::new(Arc::new(admin_keys.clone()));
        let relay_middleware = RelayMiddleware::new(
            groups_processor,
            relay_pubkey,
            database.clone(),
            registry.clone(),
            5000,
            relay_url,
            crypto_helper,
            None, // max_subscriptions
        );

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

        let mut state =
            NostrConnectionState::<()>::new(RelayUrl::parse("ws://test").unwrap()).unwrap();
        state.authed_pubkey = Some(admin_keys.public_key());

        // Process event
        let context = EventContext {
            authed_pubkey: state.authed_pubkey.as_ref(),
            subdomain: &state.subdomain,
            relay_pubkey: &relay_pubkey,
        };
        let store_commands = relay_middleware
            .processor()
            .handle_event(create_event, empty_state(), context)
            .await
            .unwrap();

        // Should have multiple store commands (group creation + metadata events)
        assert!(store_commands.len() > 1);

        // Verify group exists and is public
        let group = groups.get_group(&Scope::Default, group_id);
        assert!(group.is_some());
        assert!(!group.unwrap().value().metadata.private); // Should be public
    }

    /// Test filter verification and access control
    #[tokio::test]
    async fn test_relay_middleware_filter_verification() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let relay_pubkey = admin_keys.public_key();

        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                relay_pubkey,
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let groups_processor = GroupsRelayProcessor::new(groups.clone(), relay_pubkey);
        let registry = Arc::new(SubscriptionRegistry::new(None));
        let relay_url = RelayUrl::parse("wss://test.relay.com").unwrap();
        let crypto_helper = CryptoHelper::new(Arc::new(admin_keys.clone()));
        let relay_middleware = RelayMiddleware::new(
            groups_processor,
            relay_pubkey,
            database.clone(),
            registry.clone(),
            5000,
            relay_url,
            crypto_helper,
            None, // max_subscriptions
        );

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

        let mut state =
            NostrConnectionState::<()>::new(RelayUrl::parse("ws://test").unwrap()).unwrap();
        state.authed_pubkey = Some(admin_keys.public_key());

        let context = EventContext {
            authed_pubkey: state.authed_pubkey.as_ref(),
            subdomain: &state.subdomain,
            relay_pubkey: &relay_pubkey,
        };
        relay_middleware
            .processor()
            .handle_event(create_event, empty_state(), context)
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

        relay_middleware
            .processor()
            .handle_event(add_event, empty_state(), context)
            .await
            .unwrap();

        // Test filter verification for member
        let filters = vec![Filter::new()
            .kinds(vec![Kind::TextNote])
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), group_id)];

        let member_state =
            NostrConnectionState::<()>::new(RelayUrl::parse("ws://test").unwrap()).unwrap();
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &member_state.subdomain,
            relay_pubkey: &relay_pubkey,
        };

        // Member should be able to query
        let result =
            relay_middleware
                .processor()
                .verify_filters(&filters, empty_state(), member_context);
        assert!(result.is_ok());

        // Non-member should not be able to query
        let non_member_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_pubkey),
            subdomain: &member_state.subdomain,
            relay_pubkey: &relay_pubkey,
        };
        let result = relay_middleware.processor().verify_filters(
            &filters,
            empty_state(),
            non_member_context,
        );
        assert!(result.is_err());
    }

    /// Test event visibility in groups
    #[tokio::test]
    async fn test_relay_middleware_event_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test_with_sender().await;
        let (_, member_keys, non_member_keys) = create_test_keys().await;
        let relay_pubkey = admin_keys.public_key();

        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                relay_pubkey,
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let groups_processor = GroupsRelayProcessor::new(groups.clone(), relay_pubkey);
        let registry = Arc::new(SubscriptionRegistry::new(None));
        let relay_url = RelayUrl::parse("wss://test.relay.com").unwrap();
        let crypto_helper = CryptoHelper::new(Arc::new(admin_keys.clone()));
        let relay_middleware = RelayMiddleware::new(
            groups_processor,
            relay_pubkey,
            database.clone(),
            registry.clone(),
            5000,
            relay_url,
            crypto_helper,
            None, // max_subscriptions
        );

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

        let mut admin_state =
            NostrConnectionState::<()>::new(RelayUrl::parse("ws://test").unwrap()).unwrap();
        admin_state.authed_pubkey = Some(admin_keys.public_key());

        let admin_context = EventContext {
            authed_pubkey: admin_state.authed_pubkey.as_ref(),
            subdomain: &admin_state.subdomain,
            relay_pubkey: &relay_pubkey,
        };
        relay_middleware
            .processor()
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

        relay_middleware
            .processor()
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

        // Test visibility for admin
        let can_see = relay_middleware
            .processor()
            .can_see_event(&content_event, empty_state(), admin_context)
            .unwrap();
        assert!(can_see);

        // Test visibility for member
        let member_pubkey = member_keys.public_key();
        let member_context = EventContext {
            authed_pubkey: Some(&member_pubkey),
            subdomain: &admin_state.subdomain,
            relay_pubkey: &relay_pubkey,
        };
        let can_see = relay_middleware
            .processor()
            .can_see_event(&content_event, empty_state(), member_context)
            .unwrap();
        assert!(can_see);

        // Test visibility for non-member
        let non_member_pubkey = non_member_keys.public_key();
        let non_member_context = EventContext {
            authed_pubkey: Some(&non_member_pubkey),
            subdomain: &admin_state.subdomain,
            relay_pubkey: &relay_pubkey,
        };
        let can_see = relay_middleware
            .processor()
            .can_see_event(&content_event, empty_state(), non_member_context)
            .unwrap();
        assert!(!can_see);
    }
}
