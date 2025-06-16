use groups_relay::RelayDatabase;
use nostr_lmdb::Scope;
use nostr_relay_builder::{crypto_worker::CryptoWorker, StoreCommand, SubscriptionService};
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use websocket_builder::MessageSender;

async fn setup_test() -> (TempDir, Arc<RelayDatabase>, Keys, SubscriptionService) {
    let tmp_dir = TempDir::new().unwrap();
    let admin_keys = Keys::generate();
    let cancellation_token = CancellationToken::new();
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(admin_keys.clone()),
        cancellation_token,
    ));
    let database = Arc::new(
        RelayDatabase::new(
            tmp_dir.path().join("test.db").to_string_lossy().to_string(),
            crypto_worker,
        )
        .unwrap(),
    );

    // Create a channel for outgoing messages (we won't use it in this test)
    let (tx, _rx) = mpsc::channel(10);
    let subscription_service =
        SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

    (tmp_dir, database, admin_keys, subscription_service)
}

#[tokio::test]
async fn test_replaceable_events_buffer_deduplicates_same_second_events() {
    let (_tmp_dir, database, admin_keys, subscription_service) = setup_test().await;

    let group_id = "test_group_buffer";
    let fixed_timestamp = Timestamp::now();

    // Create two replaceable events with the same timestamp and same (pubkey, kind, scope)
    let event1 = UnsignedEvent::new(
        admin_keys.public_key(),
        fixed_timestamp,
        Kind::Custom(39000), // Replaceable event
        vec![
            Tag::identifier(group_id.to_string()),
            Tag::custom(TagKind::Name, ["First Event - Should Be Overwritten"]),
        ],
        "".to_string(),
    );

    let event2 = UnsignedEvent::new(
        admin_keys.public_key(),
        fixed_timestamp,
        Kind::Custom(39000), // Same kind
        vec![
            Tag::identifier(group_id.to_string()),
            Tag::custom(TagKind::Name, ["Second Event - Should Win"]),
            Tag::custom(
                TagKind::custom("about"),
                ["This should be the final metadata"],
            ),
        ],
        "".to_string(),
    );

    // Send both events to the subscription manager (they will be buffered)
    subscription_service
        .save_and_broadcast(StoreCommand::SaveUnsignedEvent(event1, Scope::Default))
        .await
        .unwrap();

    subscription_service
        .save_and_broadcast(StoreCommand::SaveUnsignedEvent(event2, Scope::Default))
        .await
        .unwrap();

    // Wait for buffer to flush with retry logic
    let mut retries = 0;
    let max_retries = 10;
    let mut metadata_events = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)],
            &Scope::Default,
        )
        .await
        .unwrap();

    while metadata_events.is_empty() && retries < max_retries {
        println!(
            "Retry {}/{}: No events found yet, waiting...",
            retries + 1,
            max_retries
        );
        sleep(Duration::from_millis(500)).await;

        metadata_events = database
            .query(
                vec![Filter::new()
                    .kinds(vec![Kind::Custom(39000)])
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)],
                &Scope::Default,
            )
            .await
            .unwrap();

        retries += 1;
    }

    println!(
        "Found {} metadata events after buffering",
        metadata_events.len()
    );

    // Should only have 1 event (the second one should have overwritten the first)
    assert_eq!(
        metadata_events.len(),
        1,
        "Buffer should deduplicate events with same (pubkey, kind, scope)"
    );

    let events_vec: Vec<_> = metadata_events.into_iter().collect();
    let event = &events_vec[0];

    // Verify it's the second event that won
    let name_tag = event
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::Name)
        .and_then(|t| t.content())
        .unwrap();

    assert_eq!(
        name_tag, "Second Event - Should Win",
        "The last event sent to buffer should win"
    );

    // Verify it has the about field from the second event
    let has_about = event.tags.iter().any(|t| {
        if let TagKind::Custom(s) = t.kind() {
            s == "about" && t.content() == Some("This should be the final metadata")
        } else {
            false
        }
    });

    assert!(
        has_about,
        "The second event's about field should be present"
    );
}

#[tokio::test]
async fn test_non_replaceable_events_bypass_buffer() {
    let (_tmp_dir, database, admin_keys, subscription_service) = setup_test().await;

    // Create a non-replaceable event (kind 1 - text note)
    let text_note = UnsignedEvent::new(
        admin_keys.public_key(),
        Timestamp::now(),
        Kind::TextNote, // Non-replaceable
        vec![],
        "This should bypass the buffer".to_string(),
    );

    // Send the event
    subscription_service
        .save_and_broadcast(StoreCommand::SaveUnsignedEvent(text_note, Scope::Default))
        .await
        .unwrap();

    // Wait a short time (no need to wait for buffer flush)
    sleep(Duration::from_millis(100)).await;

    // Query for the event
    let events = database
        .query(
            vec![Filter::new().kinds(vec![Kind::TextNote])],
            &Scope::Default,
        )
        .await
        .unwrap();

    // Should find the event immediately (not buffered)
    assert_eq!(
        events.len(),
        1,
        "Non-replaceable events should bypass buffer"
    );
    let events_vec: Vec<_> = events.into_iter().collect();
    assert_eq!(events_vec[0].content, "This should bypass the buffer");
}

#[tokio::test]
async fn test_signed_events_bypass_buffer() {
    let (_tmp_dir, database, admin_keys, subscription_service) = setup_test().await;

    // Create a signed replaceable event
    let event = EventBuilder::new(Kind::Custom(39000), "")
        .tags(vec![
            Tag::identifier("test_group_signed".to_string()),
            Tag::custom(TagKind::Name, ["Signed Event"]),
        ])
        .build_with_ctx(&Instant::now(), admin_keys.public_key());
    let signed_event = admin_keys.sign_event(event).await.unwrap();

    // Subscribe to broadcasts to wait for the event to be processed
    let mut broadcast_receiver = database.subscribe();

    // Send the signed event (should bypass buffer even though it's replaceable)
    subscription_service
        .save_and_broadcast(StoreCommand::SaveSignedEvent(
            Box::new(signed_event.clone()),
            Scope::Default,
        ))
        .await
        .unwrap();

    // Wait for the event to be broadcast (indicating it's been saved)
    let timeout = Duration::from_secs(2);
    let broadcast_result = tokio::time::timeout(timeout, broadcast_receiver.recv()).await;
    assert!(
        broadcast_result.is_ok(),
        "Event should be broadcast within timeout"
    );
    let received_event = broadcast_result.unwrap().unwrap();
    assert_eq!(
        received_event.id, signed_event.id,
        "Broadcast should contain our event"
    );

    // Query for the event
    let events = database
        .query(
            vec![Filter::new()
                .kinds(vec![Kind::Custom(39000)])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::D), "test_group_signed")],
            &Scope::Default,
        )
        .await
        .unwrap();

    // Should find the event immediately (not buffered)
    assert_eq!(events.len(), 1, "Signed events should bypass buffer");
    let events_vec: Vec<_> = events.into_iter().collect();
    assert_eq!(events_vec[0].id, signed_event.id);
}

#[tokio::test]
async fn test_different_scopes_are_separate_in_buffer() {
    let (_tmp_dir, database, admin_keys, subscription_service) = setup_test().await;

    let group_id = "test_group_scopes";
    let timestamp = Timestamp::now();

    // Create two events with same (pubkey, kind) but different scopes
    let event_scope1 = UnsignedEvent::new(
        admin_keys.public_key(),
        timestamp,
        Kind::Custom(39000),
        vec![
            Tag::identifier(group_id.to_string()),
            Tag::custom(TagKind::Name, ["Event in Default Scope"]),
        ],
        "".to_string(),
    );

    let event_scope2 = UnsignedEvent::new(
        admin_keys.public_key(),
        timestamp,
        Kind::Custom(39000),
        vec![
            Tag::identifier(group_id.to_string()),
            Tag::custom(TagKind::Name, ["Event in Named Scope"]),
        ],
        "".to_string(),
    );

    // Send to different scopes
    subscription_service
        .save_and_broadcast(StoreCommand::SaveUnsignedEvent(
            event_scope1,
            Scope::Default,
        ))
        .await
        .unwrap();

    let named_scope = Scope::named("testscope").unwrap();
    subscription_service
        .save_and_broadcast(StoreCommand::SaveUnsignedEvent(
            event_scope2,
            named_scope.clone(),
        ))
        .await
        .unwrap();

    // Wait for buffer flush with retry logic
    let mut retries = 0;
    let max_retries = 10;
    let mut default_events;
    let mut named_events;

    loop {
        default_events = database
            .query(
                vec![Filter::new()
                    .kinds(vec![Kind::Custom(39000)])
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)],
                &Scope::Default,
            )
            .await
            .unwrap();

        named_events = database
            .query(
                vec![Filter::new()
                    .kinds(vec![Kind::Custom(39000)])
                    .custom_tag(SingleLetterTag::lowercase(Alphabet::D), group_id)],
                &named_scope,
            )
            .await
            .unwrap();

        if (!default_events.is_empty() && !named_events.is_empty()) || retries >= max_retries {
            break;
        }

        println!(
            "Retry {}/{}: Default: {}, Named: {} events found",
            retries + 1,
            max_retries,
            default_events.len(),
            named_events.len()
        );
        sleep(Duration::from_millis(500)).await;
        retries += 1;
    }

    // Should have one event in each scope
    assert_eq!(
        default_events.len(),
        1,
        "Should have event in default scope"
    );
    assert_eq!(named_events.len(), 1, "Should have event in named scope");

    // Verify the content is different
    let default_events_vec: Vec<_> = default_events.into_iter().collect();
    let named_events_vec: Vec<_> = named_events.into_iter().collect();

    let default_name = default_events_vec[0]
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::Name)
        .and_then(|t| t.content())
        .unwrap();

    let named_name = named_events_vec[0]
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::Name)
        .and_then(|t| t.content())
        .unwrap();

    assert_eq!(default_name, "Event in Default Scope");
    assert_eq!(named_name, "Event in Named Scope");
}
