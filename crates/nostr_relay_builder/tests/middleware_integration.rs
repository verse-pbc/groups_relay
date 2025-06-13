//! Integration tests for RelayMiddleware

use async_trait::async_trait;
use nostr_relay_builder::{
    CryptoWorker, Error, EventContext, EventProcessor, NostrConnectionState, RelayDatabase,
    RelayMiddleware, StoreCommand,
};
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Test implementation of EventProcessor
#[derive(Debug, Clone)]
struct TestEventProcessor {
    allow_all: bool,
}

impl TestEventProcessor {
    fn new(allow_all: bool) -> Self {
        Self { allow_all }
    }
}

#[async_trait]
impl EventProcessor for TestEventProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
        if self.allow_all {
            Ok(vec![StoreCommand::SaveSignedEvent(
                Box::new(event),
                context.subdomain.clone(),
            )])
        } else {
            Err(Error::restricted("Events not allowed"))
        }
    }

    async fn handle_message(
        &self,
        _message: ClientMessage<'static>,
        _connection_state: &mut NostrConnectionState,
    ) -> Result<Vec<RelayMessage<'static>>, Error> {
        // Default implementation - return empty to use standard handlers
        Ok(vec![])
    }

    fn can_see_event(
        &self,
        _event: &Event,
        _custom_state: &(),
        _context: EventContext<'_>,
    ) -> Result<bool, Error> {
        Ok(self.allow_all)
    }
}

/// Restrictive processor that only allows authenticated users
#[derive(Debug, Clone)]
struct RestrictiveEventProcessor;

#[async_trait]
impl EventProcessor for RestrictiveEventProcessor {
    async fn handle_event(
        &self,
        event: Event,
        _custom_state: &mut (),
        context: EventContext<'_>,
    ) -> Result<Vec<StoreCommand>, Error> {
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
        Ok(context.authed_pubkey.is_some())
    }

    fn verify_filters(
        &self,
        _filters: &[Filter],
        _custom_state: &(),
        context: EventContext<'_>,
    ) -> Result<(), Error> {
        if context.authed_pubkey.is_some() {
            Ok(())
        } else {
            Err(Error::auth_required("Authentication required to subscribe"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_test() -> (Arc<RelayDatabase>, Keys, TempDir) {
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path();
        let keys = Keys::generate();
        let cancellation_token = CancellationToken::new();
        let crypto_worker = Arc::new(CryptoWorker::new(
            Arc::new(keys.clone()),
            cancellation_token,
        ));
        let database = Arc::new(RelayDatabase::new(db_path, crypto_worker).unwrap());
        (database, keys, tmp_dir)
    }

    #[tokio::test]
    async fn test_relay_middleware_creation() {
        let (database, keys, _tmp_dir) = setup_test().await;
        let processor = TestEventProcessor::new(true);
        let middleware = RelayMiddleware::new(processor, keys.public_key(), database);
        assert!(middleware.processor().allow_all);
    }

    #[tokio::test]
    async fn test_verify_filters() {
        let restrict_processor = RestrictiveEventProcessor;
        let relay_pubkey = Keys::generate().public_key();
        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let filters = vec![Filter::new()];

        let context = EventContext {
            authed_pubkey: None,
            subdomain: state.subdomain(),
            relay_pubkey: &relay_pubkey,
        };

        // Test without authentication
        let verify_result = restrict_processor.verify_filters(&filters, &(), context);
        assert!(verify_result.is_err());

        // Test with authentication
        let auth_pubkey = Keys::generate().public_key();
        let auth_context = EventContext {
            authed_pubkey: Some(&auth_pubkey),
            subdomain: state.subdomain(),
            relay_pubkey: &relay_pubkey,
        };

        let verify_result = restrict_processor.verify_filters(&filters, &(), auth_context);
        assert!(verify_result.is_ok());
    }

    #[tokio::test]
    async fn test_event_visibility() {
        let allow_processor = TestEventProcessor::new(true);
        let deny_processor = TestEventProcessor::new(false);
        let restrict_processor = RestrictiveEventProcessor;

        let relay_pubkey = Keys::generate().public_key();
        let state = NostrConnectionState::<()>::new("ws://test".to_string()).unwrap();
        let keys = Keys::generate();
        let event = keys
            .sign_event(EventBuilder::text_note("test").build(keys.public_key()))
            .await
            .unwrap();

        let context = EventContext {
            authed_pubkey: None,
            subdomain: state.subdomain(),
            relay_pubkey: &relay_pubkey,
        };

        // Test allow all processor
        assert!(allow_processor.can_see_event(&event, &(), context).unwrap());

        // Test deny all processor
        assert!(!deny_processor.can_see_event(&event, &(), context).unwrap());

        // Test restrictive processor without auth
        assert!(!restrict_processor
            .can_see_event(&event, &(), context)
            .unwrap());

        // Test restrictive processor with auth
        let auth_pubkey = keys.public_key();
        let auth_context = EventContext {
            authed_pubkey: Some(&auth_pubkey),
            subdomain: state.subdomain(),
            relay_pubkey: &relay_pubkey,
        };
        assert!(restrict_processor
            .can_see_event(&event, &(), auth_context)
            .unwrap());
    }
}
