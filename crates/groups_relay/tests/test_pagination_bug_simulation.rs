/// This test simulates the pagination bug where post-query filtering
/// causes fewer events to be returned than the requested limit,
/// breaking standard pagination semantics.
///
/// Scenario:
/// - Create 1 accessible event (older)
/// - Create 5 non-accessible events (newer)
/// - Query with limit 5
/// - Database returns 5 newest events (all non-accessible)
/// - After filtering: 0 events returned
/// - Bug: Client thinks no more data exists
/// - Reality: 1 accessible event exists but wasn't in the query window
use groups_relay::nostr_database::RelayDatabase;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_pagination_bug_simulation() {
    // Setup
    let relay_keys = Keys::generate();
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let database = Arc::new(
        RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap()
    );
    
    // Create events with specific timestamps to control ordering
    let base_timestamp = Timestamp::from(1700000000); // Arbitrary base time
    
    // 1. Create OLDER accessible event
    let accessible_event = EventBuilder::new(Kind::from(9), "Accessible message")
        .tags(vec![
            Tag::custom(TagKind::h(), ["public_group"]),
            Tag::custom(TagKind::custom("-"), Vec::<String>::new()),
        ])
        .custom_created_at(base_timestamp)
        .build(relay_keys.public_key());
    let accessible_event = relay_keys.sign_event(accessible_event).await.unwrap();
    database.save_signed_event(accessible_event.clone(), Scope::Default).await.unwrap();
    
    // 2. Create NEWER non-accessible events (these will be returned by the limit query)
    for i in 0..5 {
        let timestamp = Timestamp::from(base_timestamp.as_u64() + 10 + i as u64); // Newer timestamps
        let non_accessible_event = EventBuilder::new(Kind::from(9), format!("Private message {}", i))
            .tags(vec![
                Tag::custom(TagKind::h(), ["private_group"]),
                Tag::custom(TagKind::custom("-"), Vec::<String>::new()),
            ])
            .custom_created_at(timestamp)
            .build(relay_keys.public_key());
        let non_accessible_event = relay_keys.sign_event(non_accessible_event).await.unwrap();
        database.save_signed_event(non_accessible_event, Scope::Default).await.unwrap();
    }
    
    // Add a small delay to ensure events are properly saved
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // First let's check all events in the database
    let _all_events_check = database.query(vec![Filter::new().kinds(vec![Kind::from(9)])], &Scope::Default).await.unwrap();
    
    // 3. Query with limit 5 (simulating what the database would return)
    let filter = Filter::new()
        .kinds(vec![Kind::from(9)])
        .limit(5);
    
    let events = database.query(vec![filter], &Scope::Default).await.unwrap();
    
    // 4. Simulate access control filtering (user can only see public_group)
    let filtered_events: Vec<_> = events.into_iter()
        .filter(|e| e.tags.iter().any(|t| t.clone().to_vec().contains(&"public_group".to_string())))
        .collect();
    
    // Verify our simulation
    assert_eq!(filtered_events.len(), 0, "Should have 0 events after filtering");
    
    // Now let's verify the accessible event exists in the database
    let all_events_filter = Filter::new()
        .kinds(vec![Kind::from(9)]);
    let all_events = database.query(vec![all_events_filter], &Scope::Default).await.unwrap();
    let accessible_count = all_events.iter()
        .filter(|e| e.tags.iter().any(|t| t.clone().to_vec().contains(&"public_group".to_string())))
        .count();
    
    assert_eq!(accessible_count, 1, "Should have 1 accessible event in database");
}