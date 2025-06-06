//! Integration tests for EventProcessor trait and implementations

use async_trait::async_trait;
use nostr_relay_builder::{
    Error, EventContext, EventProcessor, NostrConnectionState, StoreCommand,
};
use nostr_sdk::prelude::*;

/// Basic public event processor implementation for testing
#[derive(Debug, Clone)]
struct PublicEventProcessor;

#[async_trait]
impl EventProcessor for PublicEventProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Public relay accepts all events
        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }
}

/// Auth-required event processor implementation for testing
#[derive(Debug, Clone)]
struct AuthRequiredEventProcessor;

#[async_trait]
impl EventProcessor for AuthRequiredEventProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Only accept events from authenticated users
        if context.authed_pubkey.is_some() {
            Ok(vec![StoreCommand::SaveSignedEvent(
                Box::new(event),
                context.subdomain.clone(),
            )])
        } else {
            Err(Error::auth_required(
                "Authentication required to publish events",
            ))
        }
    }

    fn can_see_event(
        &self,
        _event: &Event,
        _custom_state: &(),
        context: EventContext<'_>,
    ) -> Result<bool, Error> {
        // Only authenticated users can see events
        Ok(context.authed_pubkey.is_some())
    }

    fn verify_filters(
        &self,
        _filters: &[Filter],
        _custom_state: &(),
        context: EventContext<'_>,
    ) -> Result<(), Error> {
        // Only authenticated users can subscribe
        if context.authed_pubkey.is_some() {
            Ok(())
        } else {
            Err(Error::auth_required("Authentication required to subscribe"))
        }
    }
}

#[tokio::test]
async fn test_public_event_processor() {
    let processor = PublicEventProcessor;
    let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
    let keys = Keys::generate();
    let unsigned_event = EventBuilder::text_note("test").build(keys.public_key());
    let event = keys.sign_event(unsigned_event).await.unwrap();
    let relay_pubkey = Keys::generate().public_key();

    let context = EventContext {
        authed_pubkey: state.authed_pubkey.as_ref(),
        subdomain: state.subdomain(),
        relay_pubkey: &relay_pubkey,
    };

    // Test event handling - should accept all events
    let result = processor
        .handle_event(event.clone(), &mut (), context)
        .await;
    assert!(result.is_ok());
    let commands = result.unwrap();
    assert_eq!(commands.len(), 1);
    match &commands[0] {
        StoreCommand::SaveSignedEvent(e, _) => assert_eq!(e.id, event.id),
        _ => panic!("Expected SaveSignedEvent command"),
    }

    // Test can_see_event - should always return true
    let can_see = processor.can_see_event(&event, &(), context).unwrap();
    assert!(can_see);

    // Test verify_filters - should always succeed
    let filters = vec![Filter::new()];
    let verify_result = processor.verify_filters(&filters, &(), context);
    assert!(verify_result.is_ok());
}

#[tokio::test]
async fn test_auth_required_event_processor() {
    let processor = AuthRequiredEventProcessor;
    let mut state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
    let keys = Keys::generate();
    let unsigned_event = EventBuilder::text_note("test").build(keys.public_key());
    let event = keys.sign_event(unsigned_event).await.unwrap();
    let relay_pubkey = Keys::generate().public_key();

    // Test without authentication
    let context = EventContext {
        authed_pubkey: None,
        subdomain: state.subdomain(),
        relay_pubkey: &relay_pubkey,
    };

    let result = processor
        .handle_event(event.clone(), &mut (), context)
        .await;
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::AuthRequired { .. }));

    let can_see = processor.can_see_event(&event, &(), context).unwrap();
    assert!(!can_see);

    let filters = vec![Filter::new()];
    let verify_result = processor.verify_filters(&filters, &(), context);
    assert!(verify_result.is_err());

    // Test with authentication
    let auth_pubkey = Keys::generate().public_key();
    state.authed_pubkey = Some(auth_pubkey);

    let auth_context = EventContext {
        authed_pubkey: state.authed_pubkey.as_ref(),
        subdomain: state.subdomain(),
        relay_pubkey: &relay_pubkey,
    };

    let result = processor
        .handle_event(event.clone(), &mut (), auth_context)
        .await;
    assert!(result.is_ok());
    let commands = result.unwrap();
    assert_eq!(commands.len(), 1);

    let can_see = processor.can_see_event(&event, &(), auth_context).unwrap();
    assert!(can_see);

    let verify_result = processor.verify_filters(&filters, &(), auth_context);
    assert!(verify_result.is_ok());
}

/// Filtering processor that rejects events with certain keywords
#[derive(Debug, Clone)]
struct FilteringProcessor {
    blocked_keywords: Vec<String>,
}

#[async_trait]
impl EventProcessor for FilteringProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        // Check if event contains blocked keywords
        for keyword in &self.blocked_keywords {
            if event.content.contains(keyword) {
                return Err(Error::restricted(format!(
                    "Event contains blocked keyword: {}",
                    keyword
                )));
            }
        }

        Ok(vec![StoreCommand::SaveSignedEvent(
            Box::new(event),
            context.subdomain.clone(),
        )])
    }

    fn can_see_event(
        &self,
        event: &Event,
        _custom_state: &(),
        _context: EventContext<'_>,
    ) -> Result<bool, Error> {
        // Hide events with blocked keywords
        for keyword in &self.blocked_keywords {
            if event.content.contains(keyword) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[tokio::test]
async fn test_filtering_processor() {
    let processor = FilteringProcessor {
        blocked_keywords: vec!["spam".to_string(), "test123".to_string()],
    };
    let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
    let keys = Keys::generate();
    let relay_pubkey = Keys::generate().public_key();

    let context = EventContext {
        authed_pubkey: state.authed_pubkey.as_ref(),
        subdomain: state.subdomain(),
        relay_pubkey: &relay_pubkey,
    };

    // Test with clean content
    let clean_event = keys
        .sign_event(EventBuilder::text_note("clean content").build(keys.public_key()))
        .await
        .unwrap();
    let result = processor
        .handle_event(clean_event.clone(), &mut (), context)
        .await;
    assert!(result.is_ok());

    let can_see = processor.can_see_event(&clean_event, &(), context).unwrap();
    assert!(can_see);

    // Test with blocked content
    let spam_event = keys
        .sign_event(EventBuilder::text_note("this is spam").build(keys.public_key()))
        .await
        .unwrap();
    let result = processor
        .handle_event(spam_event.clone(), &mut (), context)
        .await;
    assert!(result.is_err());

    let can_see = processor.can_see_event(&spam_event, &(), context).unwrap();
    assert!(!can_see);
}
