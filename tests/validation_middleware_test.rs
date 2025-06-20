//! Tests for the ValidationMiddleware

use groups_relay::groups::{
    ADDRESSABLE_EVENT_KINDS, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022, NON_GROUP_ALLOWED_KINDS,
};
use groups_relay::validation_middleware::ValidationMiddleware;
use nostr_relay_builder::NostrConnectionState;
use nostr_sdk::prelude::*;
use std::borrow::Cow;
use std::sync::Arc;
use tokio::sync::RwLock;
use websocket_builder::{InboundContext, Middleware};
extern crate flume;

fn create_test_context(
    message: ClientMessage<'static>,
    state: NostrConnectionState,
    middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    >,
    sender: Option<flume::Sender<(RelayMessage<'static>, usize)>>,
) -> InboundContext<NostrConnectionState, ClientMessage<'static>, RelayMessage<'static>> {
    let state_arc = Arc::new(RwLock::new(state));
    let middlewares_arc = Arc::new(middlewares);

    InboundContext::new(
        "test_conn".to_string(),
        Some(message),
        sender,
        state_arc,
        middlewares_arc,
        0,
    )
}

fn create_test_event(keys: &Keys, kind: Kind, tags: Vec<Tag>) -> Event {
    let mut builder = EventBuilder::new(kind, "test content");
    for tag in tags {
        builder = builder.tag(tag);
    }
    builder.sign_with_keys(keys).unwrap()
}

#[tokio::test]
async fn test_validation_middleware_creation() {
    let relay_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());

    // Test Debug implementation
    let debug_str = format!("{:?}", middleware);
    assert!(debug_str.contains("ValidationMiddleware"));
}

#[tokio::test]
async fn test_event_from_relay_pubkey_with_d_tag_allowed() {
    let relay_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    // Create event from relay pubkey with d tag
    let event = create_test_event(
        &relay_keys,
        Kind::Custom(9007),
        vec![Tag::identifier("test_group")],
    );

    let message = ClientMessage::Event(Cow::Owned(event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), None);

    // Should pass validation
    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_event_without_h_tag_non_group_kind_allowed() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    // Test each non-group allowed kind
    for kind in NON_GROUP_ALLOWED_KINDS.iter() {
        let event = create_test_event(&user_keys, *kind, vec![]);
        let message = ClientMessage::Event(Cow::Owned(event));
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_context(message, state, middlewares.clone(), None);

        let result = middlewares[0].process_inbound(&mut ctx).await;
        assert!(
            result.is_ok(),
            "Kind {:?} should be allowed without h tag",
            kind
        );
    }
}

#[tokio::test]
async fn test_event_with_h_tag_allowed() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    // Create event with h tag
    let event = create_test_event(
        &user_keys,
        Kind::Custom(11), // Group chat event
        vec![Tag::custom(TagKind::h(), vec!["test_group"])],
    );

    let message = ClientMessage::Event(Cow::Owned(event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), None);

    // Should pass validation
    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_event_without_h_tag_group_kind_rejected() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    let (tx, rx) = flume::bounded(100);

    // Create group event without h tag
    let event = create_test_event(
        &user_keys,
        Kind::Custom(11), // Group chat event
        vec![],           // No h tag
    );

    let event_id = event.id;
    let message = ClientMessage::Event(Cow::Owned(event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), Some(tx));

    // Should fail validation but return Ok (error handled)
    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());

    // Check that an error message was sent
    let (sent_msg, _) = rx.try_recv().unwrap();
    match sent_msg {
        RelayMessage::Ok {
            event_id: id,
            status,
            message,
        } => {
            assert_eq!(id, event_id);
            assert!(!status);
            assert_eq!(
                message.as_ref(),
                "invalid: group events must contain an 'h' tag"
            );
        }
        _ => panic!("Expected OK message with error"),
    }
}

#[tokio::test]
async fn test_all_group_event_kinds() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    let group_kinds = vec![
        KIND_GROUP_CREATE_9007,
        KIND_GROUP_DELETE_9008,
        KIND_GROUP_ADD_USER_9000,
        KIND_GROUP_REMOVE_USER_9001,
        KIND_GROUP_EDIT_METADATA_9002,
        KIND_GROUP_DELETE_EVENT_9005,
        KIND_GROUP_SET_ROLES_9006,
        KIND_GROUP_CREATE_INVITE_9009,
        KIND_GROUP_USER_JOIN_REQUEST_9021,
        KIND_GROUP_USER_LEAVE_REQUEST_9022,
    ];

    for kind in group_kinds {
        let (tx, rx) = flume::bounded(100);

        // Test without h tag - should fail
        let event = create_test_event(&user_keys, kind, vec![]);
        let event_id = event.id;
        let message = ClientMessage::Event(Cow::Owned(event));
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_context(message, state, middlewares.clone(), Some(tx.clone()));

        let result = middlewares[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());

        // Should have sent error message
        let (sent_msg, _) = rx.try_recv().unwrap();
        match sent_msg {
            RelayMessage::Ok {
                event_id: id,
                status,
                ..
            } => {
                assert_eq!(id, event_id);
                assert!(!status);
            }
            _ => panic!("Expected OK message with error for kind {:?}", kind),
        }

        // Test with h tag - should pass
        let event = create_test_event(
            &user_keys,
            kind,
            vec![Tag::custom(TagKind::h(), vec!["test_group"])],
        );
        let message = ClientMessage::Event(Cow::Owned(event));
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_context(message, state, middlewares.clone(), None);

        let result = middlewares[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_addressable_event_kinds() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    // Test some addressable event kinds
    for kind in ADDRESSABLE_EVENT_KINDS.iter().take(3) {
        let (tx, rx) = flume::bounded(100);

        // Without h tag - should fail
        let event = create_test_event(&user_keys, *kind, vec![]);
        let event_id = event.id;
        let message = ClientMessage::Event(Cow::Owned(event));
        let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
        let mut ctx = create_test_context(message, state, middlewares.clone(), Some(tx.clone()));

        let result = middlewares[0].process_inbound(&mut ctx).await;
        assert!(result.is_ok());

        // Should have sent error
        let (sent_msg, _) = rx.try_recv().unwrap();
        match sent_msg {
            RelayMessage::Ok {
                event_id: id,
                status,
                ..
            } => {
                assert_eq!(id, event_id);
                assert!(!status);
            }
            _ => panic!("Expected OK message with error"),
        }
    }
}

#[tokio::test]
async fn test_non_event_messages_pass_through() {
    let relay_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    // Test REQ message
    let filter = Filter::new().kind(Kind::Custom(11));
    let message = ClientMessage::Req {
        subscription_id: Cow::Owned(SubscriptionId::new("test")),
        filter: Cow::Owned(filter),
    };
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), None);

    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());

    // Test CLOSE message
    let message = ClientMessage::Close(Cow::Owned(SubscriptionId::new("test")));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), None);

    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());

    // Test AUTH message
    let auth_event = create_test_event(&relay_keys, Kind::Authentication, vec![]);
    let message = ClientMessage::Auth(Cow::Owned(auth_event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), None);

    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_middleware_clone() {
    let relay_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());

    // Test that Clone works
    let cloned = middleware.clone();
    let debug_str = format!("{:?}", cloned);
    assert!(debug_str.contains("ValidationMiddleware"));
}

#[tokio::test]
async fn test_relay_pubkey_without_d_tag() {
    let relay_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    let (tx, rx) = flume::bounded(100);

    // Event from relay pubkey but without d tag and not a non-group kind
    let event = create_test_event(
        &relay_keys,
        Kind::Custom(11), // Group chat event
        vec![],           // No d tag
    );

    let event_id = event.id;
    let message = ClientMessage::Event(Cow::Owned(event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), Some(tx));

    // Should fail validation
    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());

    // Check error message was sent
    let (sent_msg, _) = rx.try_recv().unwrap();
    match sent_msg {
        RelayMessage::Ok {
            event_id: id,
            status,
            message,
        } => {
            assert_eq!(id, event_id);
            assert!(!status);
            assert_eq!(
                message.as_ref(),
                "invalid: group events must contain an 'h' tag"
            );
        }
        _ => panic!("Expected OK message with error"),
    }
}

#[tokio::test]
async fn test_event_with_multiple_tags() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![Arc::new(middleware)];

    // Event with h tag and other tags
    let event = create_test_event(
        &user_keys,
        Kind::Custom(11),
        vec![
            Tag::custom(TagKind::h(), vec!["test_group"]),
            Tag::custom(TagKind::from("p"), vec!["some_pubkey"]),
            Tag::custom(TagKind::from("e"), vec!["some_event_id"]),
        ],
    );

    let message = ClientMessage::Event(Cow::Owned(event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");
    let mut ctx = create_test_context(message, state, middlewares.clone(), None);

    // Should pass validation
    let result = middlewares[0].process_inbound(&mut ctx).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_middleware_as_trait_object() {
    let relay_keys = Keys::generate();
    let middleware = ValidationMiddleware::new(relay_keys.public_key());

    // Test that it can be used as a trait object
    let _trait_obj: Arc<
        dyn Middleware<
            State = NostrConnectionState,
            IncomingMessage = ClientMessage<'static>,
            OutgoingMessage = RelayMessage<'static>,
        >,
    > = Arc::new(middleware);
}

#[tokio::test]
async fn test_empty_middleware_chain() {
    let relay_keys = Keys::generate();
    let user_keys = Keys::generate();
    let _middleware = ValidationMiddleware::new(relay_keys.public_key());
    let middlewares: Vec<
        Arc<
            dyn Middleware<
                State = NostrConnectionState,
                IncomingMessage = ClientMessage<'static>,
                OutgoingMessage = RelayMessage<'static>,
            >,
        >,
    > = vec![];

    // Create event without h tag
    let event = create_test_event(&user_keys, Kind::Custom(11), vec![]);

    let message = ClientMessage::Event(Cow::Owned(event));
    let state = NostrConnectionState::new("wss://test.relay".to_string()).expect("Valid URL");

    // This should create a context but with empty middleware chain
    let ctx = create_test_context(message, state, middlewares.clone(), None);
    assert_eq!(ctx.connection_id, "test_conn");
}
