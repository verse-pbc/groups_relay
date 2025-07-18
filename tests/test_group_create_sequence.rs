use nostr_lmdb::Scope;
use relay_builder::{
    CryptoHelper, MessageSender, RelayDatabase, StoreCommand, SubscriptionCoordinator,
    SubscriptionRegistry,
};
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

async fn setup_test() -> (TempDir, Arc<RelayDatabase>, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let admin_keys = Keys::generate();

    let database =
        RelayDatabase::new(tmp_dir.path().join("test.db").to_string_lossy().to_string()).unwrap();
    (tmp_dir, Arc::new(database), admin_keys)
}

#[tokio::test]
async fn test_group_create_followed_by_metadata_update_sequence() {
    let (_tmp_dir, database, admin_keys) = setup_test().await;

    // Create a groups manager and subscription manager (with buffer)
    let groups = groups_relay::groups::Groups::load_groups(
        database.clone(),
        admin_keys.public_key(),
        "wss://test.relay.com".to_string(),
    )
    .await
    .unwrap();
    // Create SubscriptionRegistry and SubscriptionCoordinator
    let registry = Arc::new(SubscriptionRegistry::new(None));
    let (tx, _rx) = flume::bounded(10);
    let message_sender = MessageSender::new(tx, 0);
    let cancellation_token = CancellationToken::new();

    let crypto_helper = CryptoHelper::new(Arc::new(admin_keys.clone()));
    let subscription_coordinator = SubscriptionCoordinator::new(
        database.clone(),
        crypto_helper,
        registry.clone(),
        "test_conn".to_string(),
        message_sender.clone(),
        Some(admin_keys.public_key()),
        Arc::new(Scope::Default),
        cancellation_token.clone(),
        None,
        5000,
    );

    // Create a group (kind 9007) - this will generate kind 39000 events
    let group_id = "test_group_123";
    let create_event = EventBuilder::new(Kind::Custom(9007), "")
        .tags(vec![Tag::custom(TagKind::h(), [group_id])])
        .build_with_ctx(&Instant::now(), admin_keys.public_key());
    let create_event = Box::new(admin_keys.sign_event(create_event).await.unwrap());

    // Handle group creation (this generates metadata events)
    let create_commands = groups
        .handle_group_create(create_event, &&Scope::Default)
        .await
        .unwrap();

    println!(
        "Group creation generated {} commands",
        create_commands.len()
    );

    // Execute the commands (save all events)
    for (i, command) in create_commands.iter().enumerate() {
        println!(
            "Command {}: {:?}",
            i,
            match command {
                StoreCommand::SaveSignedEvent(event, _, None) =>
                    format!("SaveSignedEvent(kind={}, id={})", event.kind, event.id),
                StoreCommand::SaveUnsignedEvent(event, _, None) =>
                    format!("SaveUnsignedEvent(kind={}, id={:?})", event.kind, event.id),
                _ => "Other".to_string(),
            }
        );
    }

    // Execute the commands through the subscription manager (using the buffer)
    for command in create_commands {
        subscription_coordinator
            .save_and_broadcast(command)
            .await
            .unwrap();
    }

    // NO DELAY - this is the key to reproduce the issue
    // sleep(Duration::from_millis(10)).await;

    // Update metadata (kind 9002) - this should also generate kind 39000 events
    let metadata_event = EventBuilder::new(Kind::Custom(9002), "")
        .tags(vec![
            Tag::custom(TagKind::h(), [group_id]),
            Tag::custom(TagKind::Name, ["Updated Group Name"]),
            Tag::custom(TagKind::custom("about"), ["This is the real metadata"]),
        ])
        .build_with_ctx(&Instant::now(), admin_keys.public_key());
    let metadata_event = Box::new(admin_keys.sign_event(metadata_event).await.unwrap());

    // Handle metadata update (this generates new metadata events)
    let metadata_commands = groups
        .handle_edit_metadata(metadata_event, &&Scope::Default)
        .unwrap();

    println!(
        "Metadata update generated {} commands",
        metadata_commands.len()
    );

    // Execute the commands (save all events)
    for (i, command) in metadata_commands.iter().enumerate() {
        println!(
            "Metadata Command {}: {:?}",
            i,
            match command {
                StoreCommand::SaveSignedEvent(event, _, None) =>
                    format!("SaveSignedEvent(kind={}, id={})", event.kind, event.id),
                StoreCommand::SaveUnsignedEvent(event, _, None) =>
                    format!("SaveUnsignedEvent(kind={}, id={:?})", event.kind, event.id),
                _ => "Other".to_string(),
            }
        );
    }

    // Execute the metadata commands through the subscription manager (using the buffer)
    println!("Executing metadata commands...");
    for command in metadata_commands {
        subscription_coordinator
            .save_and_broadcast(command)
            .await
            .unwrap();
    }
    println!("All metadata commands sent to buffer");

    // Allow time for the buffer to flush (>1 second)
    // In CI environments, we need more time due to potential slowness
    println!("Waiting for buffer to flush...");

    // Wait up to 5 seconds for the metadata event to appear
    let mut retries = 0;
    let max_retries = 10;
    let mut found_metadata = false;

    while retries < max_retries && !found_metadata {
        sleep(Duration::from_millis(500)).await;

        let check_filter = vec![Filter::new()
            .kinds(vec![Kind::Custom(39000)])
            .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)
            .since(Timestamp::from(0))];

        let check_events = database
            .query(check_filter, &&Scope::Default)
            .await
            .unwrap();

        if !check_events.is_empty() {
            found_metadata = true;
            println!("Metadata event found after {retries} retries");
        } else {
            retries += 1;
            println!("Retry {retries}/{max_retries}: Metadata event not found yet");
        }
    }

    println!("Buffer flush wait completed");

    // First, let's query for ALL events to see what's in the database
    let all_events_filter = vec![Filter::new().since(Timestamp::from(0))]; // Get all events

    let all_events = database
        .query(all_events_filter, &&Scope::Default)
        .await
        .unwrap();

    println!("Found {} TOTAL events in database:", all_events.len());
    for event in all_events.iter() {
        println!(
            "  Event: kind={}, id={}, tags={:?}",
            event.kind,
            event.id,
            event
                .tags
                .iter()
                .map(|t| format!("{}:{:?}", t.kind(), t.content()))
                .collect::<Vec<_>>()
        );
    }

    // Query for ALL kind 39000 events (group metadata) to see both
    let all_metadata_filter = vec![Filter::new()
        .kinds(vec![Kind::Custom(39000)])
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)
        .since(Timestamp::from(0))]; // Get all events

    let all_metadata_events = database
        .query(all_metadata_filter, &&Scope::Default)
        .await
        .unwrap();

    println!(
        "Found {} metadata events (kind 39000) TOTAL",
        all_metadata_events.len()
    );

    // Query for the latest only
    let metadata_filter = vec![Filter::new()
        .kinds(vec![Kind::Custom(39000)])
        .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)
        .limit(1)];

    let metadata_events = database
        .query(metadata_filter, &&Scope::Default)
        .await
        .unwrap();

    println!(
        "Found {} metadata events (kind 39000) LATEST",
        metadata_events.len()
    );

    for (i, event) in metadata_events.iter().enumerate() {
        println!(
            "Event {}: timestamp={}, id={}",
            i, event.created_at, event.id
        );
        println!("  Tags: {:?}", event.tags);

        // Look for the group name
        if let Some(name_tag) = event.tags.iter().find(|t| t.kind() == TagKind::Name) {
            if let Some(name) = name_tag.content() {
                println!("  Group name: {name}");
            }
        }

        // Look for the about field
        if let Some(about_tag) = event.tags.iter().find(|t| {
            if let TagKind::Custom(s) = t.kind() {
                s == "about"
            } else {
                false
            }
        }) {
            if let Some(about) = about_tag.content() {
                println!("  About: {about}");
            }
        }
    }

    // The problem: if both events have the same timestamp, the one with the larger event ID
    // should win, but our bug might cause the first one (from group creation) to win instead

    // Check which event is returned by the database as the "latest"
    let latest_metadata = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)
                .limit(1)],
            &&Scope::Default,
        )
        .await
        .unwrap();

    if let Some(latest) = latest_metadata.first() {
        println!("Latest metadata event returned by database:");
        println!("  Timestamp: {}, ID: {}", latest.created_at, latest.id);

        // Check if it has the updated metadata
        let has_updated_name = latest
            .tags
            .iter()
            .any(|t| t.kind() == TagKind::Name && t.content() == Some("Updated Group Name"));

        let has_about = latest.tags.iter().any(|t| {
            if let TagKind::Custom(s) = t.kind() {
                s == "about" && t.content() == Some("This is the real metadata")
            } else {
                false
            }
        });

        println!("  Has updated name: {has_updated_name}");
        println!("  Has about field: {has_about}");

        // The test should verify that the metadata from the 9002 event wins
        assert!(
            has_updated_name,
            "The latest metadata should have the updated name from the 9002 event"
        );
        assert!(
            has_about,
            "The latest metadata should have the about field from the 9002 event"
        );
    } else {
        panic!("No metadata events found");
    }
}

#[tokio::test]
async fn test_same_timestamp_event_id_ordering() {
    let (_tmp_dir, database, admin_keys) = setup_test().await;

    let group_id = "test_group_456";
    let fixed_timestamp = Timestamp::now();

    // Create two events with the SAME timestamp but different content
    let event1 = EventBuilder::new(Kind::Custom(39000), "")
        .tags(vec![
            Tag::identifier(group_id.to_string()),
            Tag::custom(TagKind::Name, ["First Event"]),
            Tag::custom(TagKind::custom("h"), [group_id]), // Add h tag
        ])
        .custom_created_at(fixed_timestamp)
        .build(admin_keys.public_key());
    let event1 = admin_keys.sign_event(event1).await.unwrap();

    let event2 = EventBuilder::new(Kind::Custom(39000), "")
        .tags(vec![
            Tag::identifier(group_id.to_string()),
            Tag::custom(TagKind::Name, ["Second Event"]),
            Tag::custom(TagKind::custom("about"), ["This should win"]),
            Tag::custom(TagKind::custom("h"), [group_id]), // Add h tag
        ])
        .custom_created_at(fixed_timestamp)
        .build(admin_keys.public_key());
    let event2 = admin_keys.sign_event(event2).await.unwrap();

    // Save events directly to database
    // Note: The send_with_sender API has been removed in the latest version
    // Using direct database save instead
    database
        .save_event(&Box::new(event1.clone()), &Scope::Default)
        .await
        .unwrap();

    database
        .save_event(&Box::new(event2.clone()), &Scope::Default)
        .await
        .unwrap();

    // Give time for database to process the events
    sleep(Duration::from_millis(100)).await;

    // Query for the latest event
    let latest_events = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)
                .limit(1)],
            &&Scope::Default,
        )
        .await
        .unwrap();

    if let Some(latest) = latest_events.first() {
        // Determine which event should win based on ID comparison
        let expected_winner_id = if event1.id > event2.id {
            event1.id
        } else {
            event2.id
        };
        let expected_winner_name = if event1.id > event2.id {
            "First Event"
        } else {
            "Second Event"
        };

        // Verify the correct event won
        assert_eq!(
            latest.id, expected_winner_id,
            "The event with the larger ID should win when timestamps are equal"
        );

        let actual_name = latest
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::Name)
            .and_then(|t| t.content())
            .unwrap_or("NO NAME");

        assert_eq!(
            actual_name, expected_winner_name,
            "The returned event should have the correct name"
        );
    } else {
        panic!("No events found");
    }
}
