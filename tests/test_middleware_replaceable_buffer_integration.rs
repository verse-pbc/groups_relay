use groups_relay::Groups;
use groups_relay::RelayDatabase;
use nostr_lmdb::Scope;
use nostr_relay_builder::{crypto_worker::CryptoWorker, StoreCommand, SubscriptionService};
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{sleep, Duration};
use tokio_util::task::TaskTracker;
use websocket_builder::MessageSender;

async fn setup_test() -> (TempDir, Arc<RelayDatabase>, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let admin_keys = Keys::generate();
    let task_tracker = TaskTracker::new();
    let crypto_worker = CryptoWorker::spawn(Arc::new(admin_keys.clone()), &task_tracker);
    let database = Arc::new(
        RelayDatabase::new(
            tmp_dir.path().join("test.db").to_string_lossy().to_string(),
            crypto_worker,
        )
        .unwrap(),
    );

    (tmp_dir, database, admin_keys)
}

#[tokio::test]
async fn test_rapid_metadata_edits_through_subscription_service() {
    let (_tmp_dir, database, admin_keys) = setup_test().await;

    // Create a subscription manager
    let (tx, _rx) = flume::bounded(10);
    let subscription_service =
        SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

    // Load groups
    let groups = Arc::new(
        Groups::load_groups(database.clone(), admin_keys.public_key())
            .await
            .unwrap(),
    );

    // Create a group first
    let group_id = "test_dedup_group";
    let create_event = EventBuilder::new(
        Kind::Custom(9007), // GROUP_CREATE
        "",
    )
    .tags(vec![
        Tag::custom(TagKind::h(), [group_id.to_string()]),
        Tag::custom(TagKind::Name, ["Initial Name"]),
        Tag::custom(TagKind::custom("private"), Vec::<String>::new()),
        Tag::custom(TagKind::custom("closed"), Vec::<String>::new()),
    ])
    .build_with_ctx(&Instant::now(), admin_keys.public_key());
    let create_event = admin_keys.sign_event(create_event).await.unwrap();

    // Process group creation
    let create_commands = groups
        .handle_group_create(Box::new(create_event), &Scope::Default)
        .await
        .unwrap();

    // Send create commands through subscription manager to establish the group
    for command in create_commands {
        subscription_service
            .save_and_broadcast(command, None)
            .await
            .unwrap();
    }

    // Now simulate rapid metadata edits
    let edit_event1 = EventBuilder::new(
        Kind::Custom(9002), // GROUP_EDIT_METADATA
        "",
    )
    .tags(vec![
        Tag::custom(TagKind::h(), [group_id.to_string()]),
        Tag::custom(TagKind::Name, ["First Edit Name"]),
        Tag::custom(TagKind::custom("about"), ["First about"]),
    ])
    .build_with_ctx(&Instant::now(), admin_keys.public_key());
    let edit_event1 = admin_keys.sign_event(edit_event1).await.unwrap();

    let edit_event2 = EventBuilder::new(
        Kind::Custom(9002), // GROUP_EDIT_METADATA
        "",
    )
    .tags(vec![
        Tag::custom(TagKind::h(), [group_id.to_string()]),
        Tag::custom(TagKind::Name, ["Final Edit Name"]),
        Tag::custom(TagKind::custom("about"), ["Final about"]),
    ])
    .build_with_ctx(&Instant::now(), admin_keys.public_key());
    let edit_event2 = admin_keys.sign_event(edit_event2).await.unwrap();

    // Process both edits and get their commands
    let edit1_commands = groups
        .handle_edit_metadata(Box::new(edit_event1), &Scope::Default)
        .unwrap();
    let edit2_commands = groups
        .handle_edit_metadata(Box::new(edit_event2), &Scope::Default)
        .unwrap();

    // Send both sets of commands through subscription manager rapidly
    // This should trigger the buffer for the 39000 events
    for command in edit1_commands {
        subscription_service
            .save_and_broadcast(command, None)
            .await
            .unwrap();
    }
    for command in edit2_commands {
        subscription_service
            .save_and_broadcast(command, None)
            .await
            .unwrap();
    }

    // Wait less than buffer flush time
    sleep(Duration::from_millis(800)).await;

    // Query for 39000 events - at this point edits should still be buffered
    let metadata_events_before_flush = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)],
            &Scope::Default,
        )
        .await
        .unwrap();

    println!(
        "Before flush: found {} metadata events",
        metadata_events_before_flush.len()
    );

    // The initial metadata from group creation might be saved, but edits should be buffered
    assert!(
        metadata_events_before_flush.len() <= 1,
        "Edit metadata events should still be in buffer, found {} events",
        metadata_events_before_flush.len()
    );

    // Wait for buffer to flush
    sleep(Duration::from_millis(1000)).await; // Total 1.8 seconds, ensuring flush happens

    // Query again after flush
    let metadata_events_after_flush = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)],
            &Scope::Default,
        )
        .await
        .unwrap();

    println!(
        "After flush: found {} metadata events",
        metadata_events_after_flush.len()
    );

    // Should have exactly one 39000 event with the final metadata
    assert_eq!(
        metadata_events_after_flush.len(),
        1,
        "Should have exactly one metadata event after buffer flush"
    );

    let final_event = &metadata_events_after_flush.into_iter().next().unwrap();

    // Verify it has the final name
    let name_tag = final_event
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::Name)
        .and_then(|t| t.content())
        .unwrap();

    assert_eq!(
        name_tag, "Final Edit Name",
        "Should have the final edited name, not the intermediate one"
    );

    // Verify it has the final about
    let has_final_about = final_event.tags.iter().any(|t| {
        if let TagKind::Custom(s) = t.kind() {
            s == "about" && t.content() == Some("Final about")
        } else {
            false
        }
    });

    assert!(has_final_about, "Should have the final about text");
}

#[tokio::test]
async fn test_direct_database_save_bypasses_buffer() {
    let (_tmp_dir, database, admin_keys) = setup_test().await;

    // Create two 39000 events
    let event1 = UnsignedEvent::new(
        admin_keys.public_key(),
        Timestamp::now_with_supplier(&Instant::now()),
        Kind::Custom(39000),
        vec![
            Tag::identifier("bypass_test".to_string()),
            Tag::custom(TagKind::Name, ["First Name"]),
        ],
        "".to_string(),
    );

    let event2 = UnsignedEvent::new(
        admin_keys.public_key(),
        Timestamp::now_with_supplier(&Instant::now()),
        Kind::Custom(39000),
        vec![
            Tag::identifier("bypass_test".to_string()),
            Tag::custom(TagKind::Name, ["Second Name"]),
        ],
        "".to_string(),
    );

    // Save directly to database (simulating the old broken behavior)
    database
        .save_store_command(
            StoreCommand::SaveUnsignedEvent(event1, Scope::Default),
            None,
        )
        .await
        .unwrap();
    database
        .save_store_command(
            StoreCommand::SaveUnsignedEvent(event2, Scope::Default),
            None,
        )
        .await
        .unwrap();

    // Wait a bit for processing
    sleep(Duration::from_millis(100)).await;

    // Query immediately - both should be saved (no buffer deduplication)
    let events = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "bypass_test")],
            &Scope::Default,
        )
        .await
        .unwrap();

    // Without the buffer, we might get 2 events initially (before database deduplication)
    // or 1 event (after database deduplication based on replaceable logic)
    // The key point is that this bypasses the 1-second buffer window
    println!(
        "Direct save resulted in {} events in database",
        events.len()
    );

    // This test demonstrates why we need to use save_and_broadcast instead of save_store_command
}
