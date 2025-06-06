//! Message conversion utilities

use anyhow::Result;
use nostr_sdk::prelude::*;
use websocket_builder::MessageConverter;

/// Message converter for Nostr protocol messages
#[derive(Clone, Debug)]
pub struct NostrMessageConverter;

impl<'a> MessageConverter<ClientMessage<'a>, RelayMessage<'a>> for NostrMessageConverter {
    fn outbound_to_string(&self, message: RelayMessage<'a>) -> Result<String> {
        Ok(message.as_json())
    }

    fn inbound_from_string(&self, message: String) -> Result<Option<ClientMessage<'a>>> {
        match ClientMessage::from_json(&message) {
            Ok(sdk_msg) => Ok(Some(sdk_msg)),
            Err(e) => {
                if message.trim().is_empty() {
                    Ok(None)
                } else {
                    tracing::warn!("Failed to parse client message: {}, error: {}", message, e);
                    Err(anyhow::anyhow!("Failed to parse client message: {}", e))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, Kind, RelayUrl, SubscriptionId};

    #[test]
    fn test_outbound_to_string() {
        let converter = NostrMessageConverter;

        // Test with EVENT message
        let keys = Keys::generate();
        let event = EventBuilder::text_note("Hello")
            .sign_with_keys(&keys)
            .unwrap();
        let message = RelayMessage::event(SubscriptionId::new("test"), event);

        let result = converter.outbound_to_string(message).unwrap();
        assert!(result.contains("EVENT"));
        assert!(result.contains("test"));

        // Test with NOTICE message
        let notice = RelayMessage::notice("Test notice");
        let result = converter.outbound_to_string(notice).unwrap();
        assert!(result.contains("NOTICE"));
        assert!(result.contains("Test notice"));

        // Test with EOSE message
        let eose = RelayMessage::eose(SubscriptionId::new("sub1"));
        let result = converter.outbound_to_string(eose).unwrap();
        assert!(result.contains("EOSE"));
        assert!(result.contains("sub1"));

        // Test with OK message
        let ok = RelayMessage::ok(EventId::all_zeros(), true, "saved");
        let result = converter.outbound_to_string(ok).unwrap();
        assert!(result.contains("OK"));
        assert!(result.contains("true"));
        assert!(result.contains("saved"));
    }

    #[test]
    fn test_inbound_from_string_valid_messages() {
        let converter = NostrMessageConverter;

        // Test EVENT message
        let keys = Keys::generate();
        let event = EventBuilder::text_note("Test")
            .sign_with_keys(&keys)
            .unwrap();
        let event_json = format!(r#"["EVENT", {}]"#, event.as_json());

        let result = converter.inbound_from_string(event_json).unwrap();
        assert!(result.is_some());
        if let Some(ClientMessage::Event(parsed_event)) = result {
            assert_eq!(parsed_event.id, event.id);
        } else {
            panic!("Expected EVENT message");
        }

        // Test REQ message
        let req_json = r#"["REQ", "sub1", {"kinds": [1], "limit": 10}]"#.to_string();
        let result = converter.inbound_from_string(req_json).unwrap();
        assert!(result.is_some());
        if let Some(ClientMessage::Req {
            subscription_id,
            filter,
        }) = result
        {
            assert_eq!(subscription_id.as_str(), "sub1");
            assert!(filter.kinds.as_ref().unwrap().contains(&Kind::TextNote));
            assert_eq!(filter.limit, Some(10));
        } else {
            panic!("Expected REQ message");
        }

        // Test CLOSE message
        let close_json = r#"["CLOSE", "sub1"]"#.to_string();
        let result = converter.inbound_from_string(close_json).unwrap();
        assert!(result.is_some());
        if let Some(ClientMessage::Close(sub_id)) = result {
            assert_eq!(sub_id.as_str(), "sub1");
        } else {
            panic!("Expected CLOSE message");
        }
    }

    #[test]
    fn test_inbound_from_string_empty_message() {
        let converter = NostrMessageConverter;

        // Test empty string
        let result = converter.inbound_from_string("".to_string()).unwrap();
        assert!(result.is_none());

        // Test whitespace only
        let result = converter
            .inbound_from_string("   \n\t  ".to_string())
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_inbound_from_string_invalid_json() {
        let converter = NostrMessageConverter;

        // Test invalid JSON
        let result = converter.inbound_from_string("not json".to_string());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to parse client message"));

        // Test invalid message format
        let result = converter.inbound_from_string(r#"{"invalid": "format"}"#.to_string());
        assert!(result.is_err());

        // Test unknown message type
        let result = converter.inbound_from_string(r#"["UNKNOWN", "data"]"#.to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_auth_message() {
        let converter = NostrMessageConverter;

        // Test AUTH message
        let keys = Keys::generate();
        let relay_url = RelayUrl::parse("wss://relay.example.com").unwrap();
        let auth_event = EventBuilder::auth("challenge", relay_url)
            .sign_with_keys(&keys)
            .unwrap();
        let auth_json = format!(r#"["AUTH", {}]"#, auth_event.as_json());

        let result = converter.inbound_from_string(auth_json).unwrap();
        assert!(result.is_some());
        if let Some(ClientMessage::Auth(event)) = result {
            assert_eq!(event.kind, Kind::Authentication);
        } else {
            panic!("Expected AUTH message");
        }
    }
}
