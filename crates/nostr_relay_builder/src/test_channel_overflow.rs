#[cfg(test)]
mod test_channel_overflow {
    use super::*;
    use crate::test_utils::setup_test;
    use std::time::Instant;
    use tokio::sync::mpsc;
    use tokio::time::sleep;
    use websocket_builder::MessageSender;
    use nostr_sdk::prelude::*;
    use crate::subscription_service::*;

    /// Test that reproduces the channel overflow bug when requesting many events
    #[tokio::test]
    async fn test_channel_overflow_with_large_limit() {
        let (_tmp_dir, database, keys) = setup_test().await;
        
        // Create a VERY SMALL channel to easily reproduce the bug
        let (tx, mut rx) = mpsc::channel(2); // Only 2 slots!
        
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create many events (more than channel capacity)
        let num_events = 10;
        for i in 0..num_events {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        // Request all events with a large limit
        let filter = Filter::new()
            .kinds(vec![Kind::TextNote])
            .limit(num_events);
        
        let sub_id = SubscriptionId::new("test_overflow");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        // This should fail with channel full error when using send_bypass with try_send
        let result = service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await;

        // The error should occur because send_bypass uses try_send
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to send event"));
    }

    /// Test simulating slow WebSocket consumer
    #[tokio::test] 
    async fn test_slow_consumer_scenario() {
        let (_tmp_dir, database, keys) = setup_test().await;
        
        // Small channel
        let (tx, mut rx) = mpsc::channel(5);
        
        let service = SubscriptionService::new(database.clone(), MessageSender::new(tx, 0))
            .await
            .unwrap();

        // Create many events
        for i in 0..20 {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        sleep(Duration::from_millis(100)).await;

        let filter = Filter::new()
            .kinds(vec![Kind::TextNote])
            .limit(20);
        
        let sub_id = SubscriptionId::new("slow_consumer");
        let filter_fn = |_: &Event, _: &Scope, _: Option<&PublicKey>| true;

        // Spawn task that slowly consumes messages (simulating slow WebSocket)
        let slow_consumer = tokio::spawn(async move {
            let mut count = 0;
            while let Ok(msg) = rx.try_recv() {
                count += 1;
                println!("Slow consumer received message {}", count);
                // Simulate slow processing
                sleep(Duration::from_millis(50)).await;
            }
            count
        });

        // Try to send many events quickly
        let result = service
            .handle_req(
                sub_id.clone(),
                vec![filter],
                None,
                &Scope::Default,
                filter_fn,
            )
            .await;

        // Should fail because consumer is too slow
        assert!(result.is_err());
        
        // Clean up
        drop(service);
        let _ = slow_consumer.await;
    }

    /// Test that async send would provide backpressure
    #[tokio::test]
    async fn test_async_send_would_work() {
        let (_tmp_dir, database, keys) = setup_test().await;
        
        // Very small channel
        let (tx, mut rx) = mpsc::channel(2);
        
        // Create many events
        for i in 0..10 {
            let event = EventBuilder::text_note(format!("Event {}", i))
                .build_with_ctx(&Instant::now(), keys.public_key())
                .sign_with_keys(&keys)
                .unwrap();
            database
                .save_signed_event(event, Scope::Default)
                .await
                .unwrap();
        }

        // Simulate what would happen with async send
        let sender_task = tokio::spawn(async move {
            for i in 0..10 {
                let msg = RelayMessage::Event {
                    subscription_id: std::borrow::Cow::Borrowed("test"),
                    event: std::borrow::Cow::Owned(
                        EventBuilder::text_note(format!("Event {}", i))
                            .build_with_ctx(&Instant::now(), Keys::generate().public_key())
                            .sign_with_keys(&Keys::generate())
                            .unwrap()
                    ),
                };
                
                // This would wait if channel is full (natural backpressure)
                if let Err(e) = tx.send((msg, 0)).await {
                    eprintln!("Send failed: {}", e);
                    break;
                }
                println!("Sent event {}", i);
            }
        });

        // Slow consumer
        let consumer_task = tokio::spawn(async move {
            let mut count = 0;
            while let Some(_msg) = rx.recv().await {
                count += 1;
                println!("Received event {}", count);
                // Simulate slow processing
                sleep(Duration::from_millis(100)).await;
                if count >= 10 {
                    break;
                }
            }
            count
        });

        // Both should complete successfully with backpressure
        let (send_result, recv_count) = tokio::join!(sender_task, consumer_task);
        assert!(send_result.is_ok());
        assert_eq!(recv_count.unwrap(), 10);
    }
}