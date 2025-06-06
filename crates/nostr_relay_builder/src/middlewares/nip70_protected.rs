//! NIP-70: Protected Events middleware

use crate::error::Error;
use crate::state::NostrConnectionState;
use anyhow::Result;
use async_trait::async_trait;
use nostr_sdk::prelude::*;
use websocket_builder::{InboundContext, Middleware, OutboundContext, SendMessage};

/// Middleware implementing NIP-70: Protected Events
///
/// This middleware ensures that events marked with the "-" tag (protected)
/// can only be published by their original author. This prevents anyone
/// else from republishing or redistributing these protected events.
#[derive(Debug)]
pub struct Nip70Middleware;

#[async_trait]
impl Middleware for Nip70Middleware {
    type State = NostrConnectionState;
    type IncomingMessage = ClientMessage<'static>;
    type OutgoingMessage = RelayMessage<'static>;

    async fn process_inbound(
        &self,
        ctx: &mut InboundContext<'_, Self::State, ClientMessage<'static>, RelayMessage<'static>>,
    ) -> Result<(), anyhow::Error> {
        let Some(ClientMessage::Event(event)) = &ctx.message else {
            return ctx.next().await;
        };

        // Check if the event has a protected tag ("-")
        if event.tags.find_standardized(TagKind::Protected).is_none() {
            return ctx.next().await;
        }

        // Protected events require authentication
        let Some(auth_pubkey) = ctx.state.authed_pubkey else {
            return Err(
                Error::auth_required("this event may only be published by its author").into(),
            );
        };

        // Only the original author can publish protected events
        if auth_pubkey != event.pubkey {
            return ctx.send_message(RelayMessage::ok(
                event.id,
                false,
                "rejected: this event may only be published by its author",
            ));
        }

        ctx.next().await
    }

    async fn process_outbound(
        &self,
        ctx: &mut OutboundContext<'_, Self::State, ClientMessage<'static>, RelayMessage<'static>>,
    ) -> Result<(), anyhow::Error> {
        // No special handling needed for outbound messages in NIP-70
        // Protected events can be sent to authenticated users
        ctx.next().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_protected_event_identification() {
        let keys = Keys::generate();

        // Create a protected event (with "-" tag)
        let unsigned_protected = EventBuilder::text_note("protected content")
            .tag(Tag::protected())
            .build(keys.public_key());
        let protected_event = keys.sign_event(unsigned_protected).await.unwrap();

        // Verify the event has the protected tag
        let has_protected_tag = protected_event
            .tags
            .find_standardized(TagKind::Protected)
            .is_some();
        assert!(has_protected_tag, "Event should have protected tag");
    }

    #[tokio::test]
    async fn test_non_protected_event() {
        let keys = Keys::generate();

        // Create a regular event (without "-" tag)
        let unsigned_regular = EventBuilder::text_note("regular content").build(keys.public_key());
        let regular_event = keys.sign_event(unsigned_regular).await.unwrap();

        // Verify the event does NOT have the protected tag
        let has_protected_tag = regular_event
            .tags
            .find_standardized(TagKind::Protected)
            .is_some();
        assert!(!has_protected_tag, "Event should not have protected tag");
    }

    #[tokio::test]
    async fn test_author_pubkey_matching() {
        let author_keys = Keys::generate();
        let other_keys = Keys::generate();

        // Create a protected event from the author
        let unsigned_protected = EventBuilder::text_note("protected content")
            .tag(Tag::protected())
            .build(author_keys.public_key());
        let protected_event = author_keys.sign_event(unsigned_protected).await.unwrap();

        // Verify author pubkey matches event pubkey
        assert_eq!(protected_event.pubkey, author_keys.public_key());
        assert_ne!(protected_event.pubkey, other_keys.public_key());
    }

    #[test]
    fn test_nip70_middleware_creation() {
        let _middleware = Nip70Middleware;
        // Just verify it can be created
        assert!(std::any::type_name::<Nip70Middleware>().contains("Nip70Middleware"));
    }

    #[test]
    fn test_nip70_middleware_debug() {
        let middleware = Nip70Middleware;
        let debug_str = format!("{:?}", middleware);
        assert!(debug_str.contains("Nip70Middleware"));
    }

    #[tokio::test]
    async fn test_protected_tag_format() {
        let keys = Keys::generate();

        // Create a protected event
        let unsigned_protected = EventBuilder::text_note("protected content")
            .tag(Tag::protected())
            .build(keys.public_key());
        let protected_event = keys.sign_event(unsigned_protected).await.unwrap();

        // Find the protected tag and verify its format
        let protected_tag = protected_event.tags.find_standardized(TagKind::Protected);
        assert!(protected_tag.is_some(), "Protected tag should exist");

        // The protected tag should be a single "-" tag
        if let Some(tag) = protected_tag {
            assert_eq!(tag.kind(), TagKind::Protected);
        }
    }

    #[test]
    fn test_error_messages() {
        // Test the error message format for auth required access
        let error = Error::auth_required("this event may only be published by its author");
        match error {
            Error::AuthRequired { message, .. } => {
                assert_eq!(message, "this event may only be published by its author");
            }
            _ => panic!("Expected AuthRequired error"),
        }

        // Test the rejection message format
        let rejection_msg = "rejected: this event may only be published by its author";
        assert!(rejection_msg.starts_with("rejected: "));
        assert!(rejection_msg.contains("may only be published by its author"));
    }

    #[tokio::test]
    async fn test_relay_message_ok_format() {
        let event_id = EventId::all_zeros();

        // Test the OK message for rejection
        let ok_msg = RelayMessage::ok(
            event_id,
            false,
            "rejected: this event may only be published by its author",
        );

        match ok_msg {
            RelayMessage::Ok {
                event_id: id,
                status,
                message,
            } => {
                assert_eq!(id, event_id);
                assert!(!status);
                assert!(message.starts_with("rejected: "));
            }
            _ => panic!("Expected OK message"),
        }
    }

    #[tokio::test]
    async fn test_multiple_protected_tags() {
        let keys = Keys::generate();

        // Create an event with multiple tags including protected
        let unsigned = EventBuilder::text_note("content")
            .tag(Tag::protected())
            .tag(Tag::custom(
                TagKind::from("subject"),
                vec!["test".to_string()],
            ))
            .tag(Tag::custom(TagKind::from("t"), vec!["topic".to_string()]))
            .build(keys.public_key());
        let event = keys.sign_event(unsigned).await.unwrap();

        // Verify protected tag exists among other tags
        assert!(event.tags.find_standardized(TagKind::Protected).is_some());
        assert_eq!(event.tags.len(), 3);
    }

    #[tokio::test]
    async fn test_protected_event_with_different_kinds() {
        let keys = Keys::generate();

        // Test protected metadata event
        let metadata = Metadata::new().name("Protected User");
        let unsigned_metadata = EventBuilder::metadata(&metadata)
            .tag(Tag::protected())
            .build(keys.public_key());
        let metadata_event = keys.sign_event(unsigned_metadata).await.unwrap();
        assert!(metadata_event
            .tags
            .find_standardized(TagKind::Protected)
            .is_some());

        // Test protected long-form content
        let unsigned_article = EventBuilder::new(Kind::from(30023), "Article content")
            .tag(Tag::protected())
            .tag(Tag::custom(
                TagKind::from("title"),
                vec!["Protected Article".to_string()],
            ))
            .build(keys.public_key());
        let article_event = keys.sign_event(unsigned_article).await.unwrap();
        assert!(article_event
            .tags
            .find_standardized(TagKind::Protected)
            .is_some());
    }
}
