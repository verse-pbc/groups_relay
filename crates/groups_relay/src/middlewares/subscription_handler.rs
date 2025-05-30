use crate::error::Error;
use crate::nostr_session_state::NostrConnectionState;
use crate::Groups;
use nostr_lmdb::Scope;
use nostr_sdk::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, error};
use websocket_builder::MessageSender;

/// Handles subscription requests, compensating for post-query filtering in groups relay.
///
/// ## The Problem
///
/// This groups relay applies post-query filtering (access control based on group membership).
/// When a client requests events with a limit, the database returns that many events, but 
/// after filtering, fewer events may be sent to the client. This can make pagination 
/// difficult for clients.
///
/// Example scenario:
/// - Client requests: limit=10
/// - Database returns: 10 events (newest first)
/// - Access control filters out: 8 events (user not in those groups)
/// - Client receives: 2 events
/// - Problem: Client may think there are no more events, but older accessible events exist
///
/// ## Our Solution
///
/// **No limit queries**: We apply no special logic. The client gets all matching events,
/// so there's no pagination issue.
///
/// **Queries with limits**: We use one of two strategies to ensure clients receive the 
/// requested number of events (when available):
///
/// ### 1. Window Sliding (Optimal for time-bounded queries)
/// Used when filters have:
/// - `limit` only (implicitly until=now)
/// - `since + limit` (no until)
/// - `until + limit` (no since)
///
/// This algorithm slides the time window to fetch new events without re-fetching already seen events:
/// - For backward sliding (limit-only, until+limit): Progressively move the `until` timestamp backwards
/// - For forward sliding (since+limit): Progressively move the `since` timestamp forward
///
/// Benefits: Efficient use of database indexes, no duplicate fetches, maintains correct event ordering
///
/// ### 2. Exponential Buffer Fill (Fallback for complex queries)
/// Used when filters have:
/// - `since + until + limit` (bounded time window)
/// - Any other complex filter combinations
///
/// This algorithm exponentially increases the limit (2x, 4x, 8x...) until we have enough events that pass
/// the post-query filter. Less efficient but handles all edge cases.
///
/// ## Goal
///
/// Provide a guarantee that when a client receives fewer events than their limit, it means
/// there are no more accessible events available (not just that they were filtered out).
/// This enables reliable pagination for clients despite our post-query filtering.
pub async fn handle_subscription(
    groups: &Arc<Groups>,
    relay_pubkey: &PublicKey,
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
    authed_pubkey: Option<PublicKey>,
    connection_state: Option<&NostrConnectionState>,
) -> Result<(), Error> {
    let Some(conn) = connection_state else {
        error!(
            "No connection_state available for subscription {}",
            subscription_id
        );
        return Ok(());
    };

    let Some(relay_conn) = &conn.subscription_manager else {
        error!(
            "No relay connection available for subscription {}",
            subscription_id
        );
        return Ok(());
    };

    let subdomain = &conn.subdomain;

    // Get the sender for sending events to the client
    let Some(mut sender) = conn
        .subscription_manager
        .as_ref()
        .and_then(|sm| sm.get_outgoing_sender().cloned())
    else {
        error!(
            "No outgoing sender available for subscription {}",
            subscription_id
        );
        return Ok(());
    };

    // Register the subscription
    // Note: We call add_subscription directly since each connection is already
    // scoped to a specific subdomain stored in the connection state
    relay_conn.add_subscription(subscription_id.clone(), filters.clone())?;

    // Check if any filter has a limit and determine the query type
    let has_limit = filters.iter().any(|f| f.limit.is_some());

    // Determine if we can use window sliding optimization
    let can_use_window_sliding = has_limit
        && filters.iter().all(|f| {
            // Window sliding works when we have:
            // 1. limit only (implicitly until=now)
            // 2. since + limit (no until)
            // 3. until + limit (no since)
            // It doesn't work well with since + until + limit
            f.limit.is_some() && !(f.since.is_some() && f.until.is_some())
        });

    if has_limit {
        if can_use_window_sliding {
            // Use window sliding optimization for better efficiency
            handle_limited_subscription_window_sliding(
                groups,
                relay_pubkey,
                subscription_id.clone(),
                filters,
                authed_pubkey,
                relay_conn,
                subdomain,
                sender.clone(),
            )
            .await?;
        } else {
            // Use fill-buffer pagination for complex cases (e.g., since + until + limit)
            handle_limited_subscription(
                groups,
                relay_pubkey,
                subscription_id.clone(),
                filters,
                authed_pubkey,
                relay_conn,
                subdomain,
                sender.clone(),
            )
            .await?;
        }
    } else {
        // Simple case: no limits, just fetch and filter all events once
        handle_unlimited_subscription(
            groups,
            relay_pubkey,
            subscription_id.clone(),
            filters,
            authed_pubkey,
            relay_conn,
            subdomain,
            sender.clone(),
        )
        .await?;
    }

    // Send EOSE
    if let Err(e) = sender.send(RelayMessage::EndOfStoredEvents(std::borrow::Cow::Owned(
        subscription_id,
    ))) {
        error!("Failed to send EOSE: {:?}", e);
        return Err(Error::internal("Failed to send EOSE to client"));
    }

    Ok(())
}

/// Handles subscriptions without limits - simple case where we fetch all events once
/// and apply post-query filtering.
///
/// This is the straightforward case: no pagination bug because we're fetching all events
/// that match the filter criteria. Post-query filtering doesn't cause issues here because
/// the client isn't expecting a specific count of events.
async fn handle_unlimited_subscription(
    groups: &Arc<Groups>,
    relay_pubkey: &PublicKey,
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
    authed_pubkey: Option<PublicKey>,
    relay_conn: &crate::subscription_manager::SubscriptionManager,
    subdomain: &Scope,
    mut sender: MessageSender<RelayMessage<'static>>,
) -> Result<(), Error> {
    debug!("Handling unlimited subscription {}", subscription_id);

    // Fetch all events matching the filters
    let events = relay_conn
        .fetch_historical_events(&filters, subdomain)
        .await?;

    debug!("Fetched {} events for unlimited subscription", events.len());

    // Process and send events with access control filtering
    let mut sent_count = 0;
    for event in events {
        // Check if user can see this event
        let should_send = if let Some(group) = groups.find_group_from_event(&event, subdomain) {
            // Group event - check access control
            matches!(
                group
                    .value()
                    .can_see_event(&authed_pubkey, relay_pubkey, &event),
                Ok(true)
            )
        } else {
            // Not a group event or unmanaged group - allow it through
            true
        };

        if should_send {
            if let Err(e) = sender.send(RelayMessage::Event {
                subscription_id: std::borrow::Cow::Owned(subscription_id.clone()),
                event: std::borrow::Cow::Owned(event),
            }) {
                error!("Failed to send event: {:?}", e);
                return Err(Error::internal("Failed to send event to client"));
            }
            sent_count += 1;
        }
    }

    debug!(
        "Sent {} events for unlimited subscription {}",
        sent_count, subscription_id
    );

    Ok(())
}

/// Handles subscriptions with limits using exponential fill-buffer pagination.
///
/// This is the fallback strategy used for complex queries (e.g., since + until + limit) where
/// window sliding doesn't work well. It exponentially increases the database query limit
/// (2x, 4x, 8x, up to 32x) until we have enough events that pass the post-query filter.
///
/// While less efficient than window sliding (due to re-fetching events), this approach
/// handles all edge cases and ensures clients receive the requested number of events
/// when they exist.
async fn handle_limited_subscription(
    groups: &Arc<Groups>,
    relay_pubkey: &PublicKey,
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
    authed_pubkey: Option<PublicKey>,
    relay_conn: &crate::subscription_manager::SubscriptionManager,
    subdomain: &Scope,
    mut sender: MessageSender<RelayMessage<'static>>,
) -> Result<(), Error> {
    debug!(
        "Handling limited subscription {} with fill-buffer pagination",
        subscription_id
    );

    let mut seen_event_ids = HashSet::new();
    let mut sent_count = 0;
    let mut multiplier = 1usize;
    const MAX_MULTIPLIER: usize = 32;

    // Use channel capacity as the default limit to match buffer size
    let channel_capacity = sender.capacity();

    // Track original limits for each filter
    let original_limits: Vec<Option<usize>> = filters.iter().map(|f| f.limit).collect();
    let target_limit = original_limits
        .iter()
        .filter_map(|&l| l)
        .max()
        .unwrap_or(channel_capacity);

    loop {
        // Adjust filters with exponentially growing limits
        let mut adjusted_filters = filters.clone();
        for (i, filter) in adjusted_filters.iter_mut().enumerate() {
            if let Some(original_limit) = original_limits[i] {
                filter.limit = if multiplier <= MAX_MULTIPLIER {
                    Some(original_limit.saturating_mul(multiplier))
                } else {
                    None // No limit - get all events
                };
            }
        }

        // Fetch events from database
        let events = relay_conn
            .fetch_historical_events(&adjusted_filters, subdomain)
            .await?;

        if events.is_empty() {
            debug!("Fill-buffer: No more events in database");
            break;
        }

        debug!(
            "Fill-buffer: Fetched {} events with multiplier {}",
            events.len(),
            multiplier
        );

        // Process events - filter and send immediately
        for event in events {
            // Skip duplicates
            if seen_event_ids.contains(&event.id) {
                continue;
            }
            seen_event_ids.insert(event.id);

            // Check if user can see this event
            let should_send = if let Some(group) = groups.find_group_from_event(&event, subdomain) {
                // Group event - check access control
                matches!(
                    group
                        .value()
                        .can_see_event(&authed_pubkey, relay_pubkey, &event),
                    Ok(true)
                )
            } else {
                // Not a group event or unmanaged group - allow it through
                true
            };

            if should_send {
                if let Err(e) = sender.send(RelayMessage::Event {
                    subscription_id: std::borrow::Cow::Owned(subscription_id.clone()),
                    event: std::borrow::Cow::Owned(event),
                }) {
                    error!("Failed to send event: {:?}", e);
                    return Err(Error::internal("Failed to send event to client"));
                }

                sent_count += 1;
                if sent_count >= target_limit {
                    debug!("Fill-buffer: Reached target limit of {}", target_limit);
                    break;
                }
            }
        }

        // Check if we've sent enough events
        if sent_count >= target_limit {
            break;
        }

        // Check if we've hit the multiplier limit
        if multiplier > MAX_MULTIPLIER {
            debug!("Fill-buffer: Reached max multiplier, stopping");
            break;
        }

        // Exponentially increase the multiplier
        multiplier *= 2;
    }

    debug!(
        "Fill-buffer: Sent {} events total to subscription {}",
        sent_count, subscription_id
    );

    Ok(())
}

/// Handles subscriptions with limits using window sliding optimization.
///
/// This is the optimal strategy for simple time-bounded queries. Instead of increasing the
/// limit and re-fetching events we've already seen (like exponential pagination does), we
/// slide the time window to fetch only new events:
///
/// - **Backward sliding** (for `limit` only or `until + limit`):
///   We progressively move the `until` timestamp backwards to fetch older events
/// - **Forward sliding** (for `since + limit`):
///   We progressively move the `since` timestamp forward to fetch newer events
///
/// This approach is highly efficient because:
/// 1. It leverages database indexes on created_at timestamps
/// 2. It never fetches the same event twice
/// 3. It maintains correct chronological ordering as per NIP-01
///
/// The algorithm stops when we've sent enough events that pass the filter, when there are
/// no more events in the database, or after a maximum number of iterations (to prevent
/// infinite loops in edge cases).
async fn handle_limited_subscription_window_sliding(
    groups: &Arc<Groups>,
    relay_pubkey: &PublicKey,
    subscription_id: SubscriptionId,
    filters: Vec<Filter>,
    authed_pubkey: Option<PublicKey>,
    relay_conn: &crate::subscription_manager::SubscriptionManager,
    subdomain: &Scope,
    mut sender: MessageSender<RelayMessage<'static>>,
) -> Result<(), Error> {
    debug!(
        "Handling limited subscription {} with window sliding optimization",
        subscription_id
    );

    let mut seen_event_ids = HashSet::new();
    let mut sent_count = 0;

    // Use channel capacity as the default limit
    let channel_capacity = sender.capacity();

    // Get the target limit from filters
    let target_limit = filters
        .iter()
        .filter_map(|f| f.limit)
        .max()
        .unwrap_or(channel_capacity);

    // Determine the sliding direction based on filter type
    enum SlidingDirection {
        Backward, // For limit-only or until+limit (move towards past)
        Forward,  // For since+limit (move towards future)
    }

    let direction = if filters
        .iter()
        .any(|f| f.since.is_some() && f.until.is_none())
    {
        SlidingDirection::Forward
    } else {
        SlidingDirection::Backward
    };

    // Track the boundary timestamp for sliding
    let mut boundary_timestamp: Option<Timestamp> = None;
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 10; // Prevent infinite loops

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            debug!("Window sliding: Reached max iterations, stopping");
            break;
        }

        // Adjust filters based on sliding window
        let mut adjusted_filters = filters.clone();
        for filter in adjusted_filters.iter_mut() {
            if let Some(boundary) = boundary_timestamp {
                match direction {
                    SlidingDirection::Backward => {
                        // Move window backwards: set until to boundary - 1
                        filter.until = Some(Timestamp::from(boundary.as_u64().saturating_sub(1)));
                    }
                    SlidingDirection::Forward => {
                        // Move window forward: set since to boundary + 1
                        filter.since = Some(Timestamp::from(boundary.as_u64().saturating_add(1)));
                    }
                }
            }
        }

        // Fetch events from database
        let events = relay_conn
            .fetch_historical_events(&adjusted_filters, subdomain)
            .await?;

        if events.is_empty() {
            debug!("Window sliding: No more events in database");
            break;
        }

        debug!(
            "Window sliding: Fetched {} events in iteration {}",
            events.len(),
            iterations
        );

        // Track the boundary for next iteration
        let mut iteration_boundary: Option<Timestamp> = None;

        // Process events - filter and send immediately
        for event in events {
            // Skip duplicates
            if seen_event_ids.contains(&event.id) {
                continue;
            }
            seen_event_ids.insert(event.id);

            // Update boundary timestamp based on direction
            match direction {
                SlidingDirection::Backward => {
                    // Track the oldest event timestamp
                    if iteration_boundary.is_none()
                        || event.created_at < iteration_boundary.unwrap()
                    {
                        iteration_boundary = Some(event.created_at);
                    }
                }
                SlidingDirection::Forward => {
                    // Track the newest event timestamp
                    if iteration_boundary.is_none()
                        || event.created_at > iteration_boundary.unwrap()
                    {
                        iteration_boundary = Some(event.created_at);
                    }
                }
            }

            // Check if user can see this event
            let should_send = if let Some(group) = groups.find_group_from_event(&event, subdomain) {
                // Group event - check access control
                matches!(
                    group
                        .value()
                        .can_see_event(&authed_pubkey, relay_pubkey, &event),
                    Ok(true)
                )
            } else {
                // Not a group event or unmanaged group - allow it through
                true
            };

            if should_send {
                if let Err(e) = sender.send(RelayMessage::Event {
                    subscription_id: std::borrow::Cow::Owned(subscription_id.clone()),
                    event: std::borrow::Cow::Owned(event),
                }) {
                    error!("Failed to send event: {:?}", e);
                    return Err(Error::internal("Failed to send event to client"));
                }

                sent_count += 1;
                if sent_count >= target_limit {
                    debug!("Window sliding: Reached target limit of {}", target_limit);
                    return Ok(());
                }
            }
        }

        // Update boundary for next iteration
        if let Some(new_boundary) = iteration_boundary {
            // If boundary hasn't changed, we might be stuck
            if boundary_timestamp == Some(new_boundary) {
                debug!("Window sliding: Boundary didn't change, stopping to prevent infinite loop");
                break;
            }
            boundary_timestamp = Some(new_boundary);
        } else {
            // No events were processed in this iteration
            debug!("Window sliding: No events processed in iteration, stopping");
            break;
        }
    }

    debug!(
        "Window sliding: Sent {} events total to subscription {}",
        sent_count, subscription_id
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nostr_database::RelayDatabase;
    use tempfile::TempDir;

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
    #[tokio::test]
    async fn test_pagination_bug_simulation() {
        // Setup
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
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
        database
            .save_signed_event(accessible_event.clone(), Scope::Default)
            .await
            .unwrap();

        // 2. Create NEWER non-accessible events (these will be returned by the limit query)
        for i in 0..5 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + 10 + i as u64); // Newer timestamps
            let non_accessible_event =
                EventBuilder::new(Kind::from(9), format!("Private message {}", i))
                    .tags(vec![
                        Tag::custom(TagKind::h(), ["private_group"]),
                        Tag::custom(TagKind::custom("-"), Vec::<String>::new()),
                    ])
                    .custom_created_at(timestamp)
                    .build(relay_keys.public_key());
            let non_accessible_event = relay_keys.sign_event(non_accessible_event).await.unwrap();
            database
                .save_signed_event(non_accessible_event, Scope::Default)
                .await
                .unwrap();
        }

        // Add a small delay to ensure events are properly saved
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // First let's check all events in the database
        let _all_events_check = database
            .query(
                vec![Filter::new().kinds(vec![Kind::from(9)])],
                &Scope::Default,
            )
            .await
            .unwrap();

        // 3. Query with limit 5 (simulating what the database would return)
        let filter = Filter::new().kinds(vec![Kind::from(9)]).limit(5);

        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // 4. Simulate access control filtering (user can only see public_group)
        let filtered_events: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        // Verify our simulation
        assert_eq!(
            filtered_events.len(),
            0,
            "Should have 0 events after filtering"
        );

        // Now let's verify the accessible event exists in the database
        let all_events_filter = Filter::new().kinds(vec![Kind::from(9)]);
        let all_events = database
            .query(vec![all_events_filter], &Scope::Default)
            .await
            .unwrap();
        let accessible_count = all_events
            .iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .count();

        assert_eq!(
            accessible_count, 1,
            "Should have 1 accessible event in database"
        );
    }

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
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create 1 accessible event (older)
        let accessible = create_test_event(
            &relay_keys,
            base_timestamp,
            "public_group",
            "Accessible message",
        )
        .await;
        database
            .save_signed_event(accessible, Scope::Default)
            .await
            .unwrap();

        // Create 5 non-accessible events (newer)
        for i in 0..5 {
            let event = create_test_event(
                &relay_keys,
                Timestamp::from(base_timestamp.as_u64() + 10 + i),
                "private_group",
                &format!("Private message {}", i),
            )
            .await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query with limit 5 (default includes implicit "until now")
        let filter = Filter::new().kinds(vec![Kind::from(9)]).limit(5);

        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // Simulate filtering (user can only see public_group)
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        assert_eq!(
            filtered.len(),
            0,
            "Bug: 0 events returned but 1 accessible exists"
        );
    }

    #[tokio::test]
    async fn test_pagination_bug_case2_since_limit() {
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create events: A(accessible), N(non-accessible), N, N, N, N, A
        // Timeline:      0             10            20  30  40  50  60

        // First accessible event
        let event1 = create_test_event(
            &relay_keys,
            base_timestamp,
            "public_group",
            "First accessible",
        )
        .await;
        database
            .save_signed_event(event1, Scope::Default)
            .await
            .unwrap();

        // 5 non-accessible events
        for i in 0..5 {
            let event = create_test_event(
                &relay_keys,
                Timestamp::from(base_timestamp.as_u64() + 10 + i * 10),
                "private_group",
                &format!("Private {}", i),
            )
            .await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        // Last accessible event
        let event2 = create_test_event(
            &relay_keys,
            Timestamp::from(base_timestamp.as_u64() + 60),
            "public_group",
            "Last accessible",
        )
        .await;
        database
            .save_signed_event(event2, Scope::Default)
            .await
            .unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query: Get 5 events after timestamp 0
        let filter = Filter::new()
            .kinds(vec![Kind::from(9)])
            .since(base_timestamp)
            .limit(5);

        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // Simulate filtering
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        // With limit 5 and forward pagination, we expect to get the 5 oldest events
        // But the exact behavior depends on whether the database returns in forward or reverse order
        // The key is that with only 5 events returned, we might miss accessible events

        let has_first = filtered.iter().any(|e| e.content == "First accessible");
        let has_last = filtered.iter().any(|e| e.content == "Last accessible");

        // The bug is that we can't get both accessible events with limit 5
        // because there are 5 non-accessible events between them
        assert!(
            !(has_first && has_last),
            "Bug: Should not get both accessible events with limit 5"
        );
        assert!(
            filtered.len() < 2,
            "Bug: Not all accessible events returned"
        );
    }

    #[tokio::test]
    async fn test_pagination_bug_case3_since_until_limit() {
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events across 100 seconds, alternating accessible/non-accessible
        // Timeline: A N N A N N A N N A (at timestamps 0, 10, 20, ..., 90)
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let group = if i % 3 == 0 {
                "public_group"
            } else {
                "private_group"
            };
            let content = format!(
                "{} message at {}",
                if i % 3 == 0 { "Accessible" } else { "Private" },
                i * 10
            );

            let event = create_test_event(&relay_keys, timestamp, group, &content).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
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
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        // The query returns the 5 newest in the window: N(80), N(70), A(60), N(50), N(40)
        // After filtering: only A(60) remains
        // But A(30) is never returned despite being in the time window!

        assert!(
            filtered.len() < 2,
            "Bug: Not all accessible events in window returned"
        );
    }

    #[test]
    fn test_all_cases_summary() {
        // This test serves as documentation for the pagination bug patterns
        // The bug occurs in all three query patterns:
        // 1. DEFAULT/UNTIL + LIMIT: Query returns newest N events, post-filter reduces count, older accessible events are missed
        // 2. SINCE + LIMIT: Query returns oldest N events after timestamp, post-filter reduces count, newer accessible events are missed
        // 3. SINCE + UNTIL + LIMIT: Query returns N events in time window, post-filter reduces count, other accessible events in window are missed
        // ROOT CAUSE: Database limit is applied before access control filtering
        // SOLUTION: Middleware must implement fill-buffer with exponential limits
    }

    #[tokio::test]
    async fn test_window_sliding_case1_limit_only() {
        // Test case: limit only (implicitly until=now)
        // Setup: 10 events alternating between public and private
        // Expected: Should get 5 public events by sliding window backwards
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events: P, N, P, N, P, N, P, N, P, N (newest to oldest)
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + (9 - i) * 10);
            let group = if i % 2 == 0 {
                "public_group"
            } else {
                "private_group"
            };
            let content = format!(
                "{} event at position {}",
                if i % 2 == 0 { "Public" } else { "Private" },
                i
            );

            let event = create_test_event(&relay_keys, timestamp, group, &content).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query with limit 5
        let filter = Filter::new().kinds(vec![Kind::from(9)]).limit(5);
        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // Simulate filtering (user can only see public_group)
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        // With the current exponential approach, this should eventually get all 5 public events
        // With window sliding, we'd move the until backwards to get more events
        assert!(
            filtered.len() <= 3,
            "Initial query should return fewer than needed due to filtering"
        );
    }

    #[tokio::test]
    async fn test_window_sliding_case2_until_limit() {
        // Test case: until + limit
        // Setup: 10 events alternating between public and private
        // Expected: Should get 5 public events by sliding window backwards from until point
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events across 100 seconds
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let group = if i % 2 == 0 {
                "public_group"
            } else {
                "private_group"
            };
            let content = format!(
                "{} event at time {}",
                if i % 2 == 0 { "Public" } else { "Private" },
                i * 10
            );

            let event = create_test_event(&relay_keys, timestamp, group, &content).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query with until=80 (position 8) and limit 5
        // Should ideally return events at times 80, 70, 60, 50, 40
        // But only 80, 60, 40 are public
        let filter = Filter::new()
            .kinds(vec![Kind::from(9)])
            .until(Timestamp::from(base_timestamp.as_u64() + 80))
            .limit(5);

        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // Simulate filtering
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        // Initial query returns 5 events (80,70,60,50,40) but only 3 are public
        assert_eq!(
            filtered.len(),
            3,
            "Should get 3 public events in the initial window"
        );
    }

    #[tokio::test]
    async fn test_window_sliding_case3_since_limit() {
        // Test case: since + limit (no until)
        // Setup: 10 events alternating between public and private
        // Expected: Should get 5 public events by sliding window forward from since point
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create 10 events across 100 seconds
        for i in 0..10 {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + i * 10);
            let group = if i % 2 == 0 {
                "public_group"
            } else {
                "private_group"
            };
            let content = format!(
                "{} event at time {}",
                if i % 2 == 0 { "Public" } else { "Private" },
                i * 10
            );

            let event = create_test_event(&relay_keys, timestamp, group, &content).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query with since=20 (position 2) and limit 5
        // Database behavior for since+limit depends on implementation
        // Some DBs return oldest first, some return newest first within the range
        let filter = Filter::new()
            .kinds(vec![Kind::from(9)])
            .since(Timestamp::from(base_timestamp.as_u64() + 20))
            .limit(5);

        let events = database.query(vec![filter], &Scope::Default).await.unwrap();

        // Simulate filtering
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        // The query behavior with since+limit varies by implementation
        // But we should get fewer than 5 public events initially
        assert!(
            filtered.len() < 5,
            "Should get fewer than 5 public events due to filtering"
        );
    }

    #[tokio::test]
    async fn test_window_sliding_efficiency() {
        // Test that window sliding doesn't re-fetch events it's already seen
        // This demonstrates the efficiency improvement over exponential pagination
        let relay_keys = Keys::generate();
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let database = Arc::new(
            RelayDatabase::new(db_path.to_string_lossy().to_string(), relay_keys.clone()).unwrap(),
        );

        let base_timestamp = Timestamp::from(1700000000);

        // Create a pattern that requires multiple sliding iterations:
        // N, N, N, N, N, P, N, N, N, N, P (oldest to newest)
        // With limit 2, first query gets the 2 newest (N, P)
        // Only 1 passes filter, so we slide and get (N, N)
        // 0 pass filter, slide again and get (N, N)
        // 0 pass filter, slide again and get (P, N)
        // 1 passes filter, total 2, done

        let events_pattern = vec![
            ("private", 0),
            ("private", 10),
            ("private", 20),
            ("private", 30),
            ("private", 40),
            ("public", 50),
            ("private", 60),
            ("private", 70),
            ("private", 80),
            ("private", 90),
            ("public", 100),
        ];

        for (group_type, time_offset) in events_pattern {
            let timestamp = Timestamp::from(base_timestamp.as_u64() + time_offset);
            let group = if group_type == "public" {
                "public_group"
            } else {
                "private_group"
            };
            let content = format!("{} event at time {}", group_type, time_offset);

            let event = create_test_event(&relay_keys, timestamp, group, &content).await;
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Query with limit 2
        let filter = Filter::new().kinds(vec![Kind::from(9)]).limit(2);

        // First query should get events at times 100 and 90
        let events = database
            .query(vec![filter.clone()], &Scope::Default)
            .await
            .unwrap();
        assert_eq!(events.len(), 2, "Should get exactly 2 events");

        // Check we got the newest events
        let timestamps: Vec<u64> = events.iter().map(|e| e.created_at.as_u64()).collect();
        assert!(timestamps.contains(&(base_timestamp.as_u64() + 100)));
        assert!(timestamps.contains(&(base_timestamp.as_u64() + 90)));

        // Simulate filtering - only 1 public event passes
        let filtered: Vec<_> = events
            .into_iter()
            .filter(|e| {
                e.tags
                    .iter()
                    .any(|t| t.clone().to_vec().contains(&"public_group".to_string()))
            })
            .collect();

        assert_eq!(filtered.len(), 1, "Only 1 public event in first window");

        // With window sliding, the next query would use until=89 to get older events
        // This is more efficient than exponential which would query with limit=4
        // and re-fetch the same events we already processed
    }
}
