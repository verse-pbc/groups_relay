/// Comprehensive tests demonstrating the pagination bug in all three query cases:
/// 1. Default/until + limit
/// 2. Since + limit  
/// 3. Since + until + limit
use groups_relay::nostr_database::RelayDatabase;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create test events
async fn create_test_event(
    keys: &Keys,
    timestamp: Timestamp,
    group: &str,
    content: &str,
) -> Event {
    let event = EventBuilder::new(Kind::from(9), content)
        .tags(vec![
            Tag::custom(TagKind::h(), [group]),
            Tag::custom(TagKind::custom("-"), Vec::<String>::new()),
        ])
        .custom_created_at(timestamp)
        .build(keys.public_key());
    keys.sign_event(event).await.unwrap()
}

#[tokio::test]
async fn test_pagination_bug_case1_until_limit() {
    let relay_keys = Keys::generate();
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let database = Arc::new(
        RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap()
    );
    
    let base_timestamp = Timestamp::from(1700000000);
    
    // Create 1 accessible event (older)
    let accessible = create_test_event(
        &relay_keys,
        base_timestamp,
        "public_group",
        "Accessible message"
    ).await;
    database.save_signed_event(accessible, Scope::Default).await.unwrap();
    
    // Create 5 non-accessible events (newer)
    for i in 0..5 {
        let event = create_test_event(
            &relay_keys,
            Timestamp::from(base_timestamp.as_u64() + 10 + i),
            "private_group",
            &format!("Private message {}", i)
        ).await;
        database.save_signed_event(event, Scope::Default).await.unwrap();
    }
    
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // Query with limit 5 (default includes implicit "until now")
    let filter = Filter::new()
        .kinds(vec![Kind::from(9)])
        .limit(5);
    
    let events = database.query(vec![filter], &Scope::Default).await.unwrap();
    
    // Simulate filtering (user can only see public_group)
    let filtered: Vec<_> = events.into_iter()
        .filter(|e| e.tags.iter().any(|t| t.clone().to_vec().contains(&"public_group".to_string())))
        .collect();
    
    assert_eq!(filtered.len(), 0, "Bug: 0 events returned but 1 accessible exists");
}

#[tokio::test]
async fn test_pagination_bug_case2_since_limit() {
    let relay_keys = Keys::generate();
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let database = Arc::new(
        RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap()
    );
    
    let base_timestamp = Timestamp::from(1700000000);
    
    // Create events: A(accessible), N(non-accessible), N, N, N, N, A
    // Timeline:      0             10            20  30  40  50  60
    
    // First accessible event
    let event1 = create_test_event(
        &relay_keys,
        base_timestamp,
        "public_group",
        "First accessible"
    ).await;
    database.save_signed_event(event1, Scope::Default).await.unwrap();
    
    // 5 non-accessible events
    for i in 0..5 {
        let event = create_test_event(
            &relay_keys,
            Timestamp::from(base_timestamp.as_u64() + 10 + i * 10),
            "private_group",
            &format!("Private {}", i)
        ).await;
        database.save_signed_event(event, Scope::Default).await.unwrap();
    }
    
    // Last accessible event
    let event2 = create_test_event(
        &relay_keys,
        Timestamp::from(base_timestamp.as_u64() + 60),
        "public_group",
        "Last accessible"
    ).await;
    database.save_signed_event(event2, Scope::Default).await.unwrap();
    
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // Query: Get 5 events after timestamp 0
    let filter = Filter::new()
        .kinds(vec![Kind::from(9)])
        .since(base_timestamp)
        .limit(5);
    
    let events = database.query(vec![filter], &Scope::Default).await.unwrap();
    
    // Simulate filtering
    let filtered: Vec<_> = events.into_iter()
        .filter(|e| e.tags.iter().any(|t| t.clone().to_vec().contains(&"public_group".to_string())))
        .collect();
    
    // With limit 5 and forward pagination, we expect to get the 5 oldest events
    // But the exact behavior depends on whether the database returns in forward or reverse order
    // The key is that with only 5 events returned, we might miss accessible events
    
    let has_first = filtered.iter().any(|e| e.content == "First accessible");
    let has_last = filtered.iter().any(|e| e.content == "Last accessible");
    
    // The bug is that we can't get both accessible events with limit 5
    // because there are 5 non-accessible events between them
    assert!(!(has_first && has_last), "Bug: Should not get both accessible events with limit 5");
    assert!(filtered.len() < 2, "Bug: Not all accessible events returned");
}

#[tokio::test]
async fn test_pagination_bug_case3_since_until_limit() {
    let relay_keys = Keys::generate();
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let database = Arc::new(
        RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap()
    );
    
    let base_timestamp = Timestamp::from(1700000000);
    
    // Create 10 events across 100 seconds, alternating accessible/non-accessible
    // Timeline: A N N A N N A N N A (at timestamps 0, 10, 20, ..., 90)
    for i in 0..10 {
        let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
        let group = if i % 3 == 0 { "public_group" } else { "private_group" };
        let content = format!("{} message at {}", 
            if i % 3 == 0 { "Accessible" } else { "Private" }, i * 10);
        
        let event = create_test_event(&relay_keys, timestamp, group, &content).await;
        database.save_signed_event(event, Scope::Default).await.unwrap();
    }
    
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // Query: Get 5 events in window [20, 80]
    // Should have: N(20), A(30), N(40), N(50), A(60), N(70), N(80)
    // Accessible: A(30), A(60)
    let filter = Filter::new()
        .kinds(vec![Kind::from(9)])
        .since(Timestamp::from(base_timestamp.as_u64() + 20))
        .until(Timestamp::from(base_timestamp.as_u64() + 80))
        .limit(5);
    
    let events = database.query(vec![filter], &Scope::Default).await.unwrap();
    
    // Simulate filtering
    let filtered: Vec<_> = events.into_iter()
        .filter(|e| e.tags.iter().any(|t| t.clone().to_vec().contains(&"public_group".to_string())))
        .collect();
    
    // The query returns the 5 newest in the window: N(80), N(70), A(60), N(50), N(40)
    // After filtering: only A(60) remains
    // But A(30) is never returned despite being in the time window!
    
    assert!(filtered.len() < 2, "Bug: Not all accessible events in window returned");
}

#[tokio::test]
async fn test_all_cases_summary() {
    // This test serves as documentation for the pagination bug patterns
    // The bug occurs in all three query patterns:
    // 1. DEFAULT/UNTIL + LIMIT: Query returns newest N events, post-filter reduces count, older accessible events are missed
    // 2. SINCE + LIMIT: Query returns oldest N events after timestamp, post-filter reduces count, newer accessible events are missed  
    // 3. SINCE + UNTIL + LIMIT: Query returns N events in time window, post-filter reduces count, other accessible events in window are missed
    // ROOT CAUSE: Database limit is applied before access control filtering
    // SOLUTION: Middleware must implement fill-buffer with exponential limits
}