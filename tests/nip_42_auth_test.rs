use groups_relay::middlewares::nip_42_auth::Nip42Auth;
use nostr_sdk::{
    EventBuilder, Keys, Kind, NostrSigner, RelayUrl, Tag, TagStandard, Timestamp,
};
use std::time::Instant;

#[tokio::test]
async fn test_authed_pubkey() {
    let keys = Keys::generate();
    let local_url = RelayUrl::parse("wss://test.relay").unwrap();
    let auth = Nip42Auth::new(local_url.as_str().to_string());
    let challenge = "test_challenge";

    // Create valid auth event
    let unsigned_event = EventBuilder::new(Kind::Authentication, "")
        .tag(Tag::from_standardized(TagStandard::Challenge(challenge.to_string())))
        .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
        .build_with_ctx(&Instant::now(), keys.public_key());
    let event = keys.sign_event(unsigned_event).await.unwrap();

    // Verify event signature
    assert!(event.verify().is_ok());

    // Test valid auth
    let result = auth.authed_pubkey(&event, Some(challenge));
    assert_eq!(result, Some(keys.public_key()));

    // Test invalid challenge
    let result = auth.authed_pubkey(&event, Some("wrong_challenge"));
    assert_eq!(result, None);

    // Test missing challenge
    let result = auth.authed_pubkey(&event, None);
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_expired_auth() {
    let keys = Keys::generate();
    let local_url = RelayUrl::parse("wss://test.relay").unwrap();
    let auth = Nip42Auth::new(local_url.as_str().to_string());
    let challenge = "test_challenge";

    // Create expired auth event with manual timestamp
    let unsigned_event = EventBuilder::new(Kind::Authentication, "")
        .tag(Tag::from_standardized(TagStandard::Challenge(challenge.to_string())))
        .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
        .custom_created_at(Timestamp::from(0))
        .build_with_ctx(&Instant::now(), keys.public_key());
    let event = keys.sign_event(unsigned_event).await.unwrap();

    // Verify event signature
    assert!(event.verify().is_ok());

    let result = auth.authed_pubkey(&event, Some(challenge));
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_wrong_relay() {
    let keys = Keys::generate();
    let local_url = RelayUrl::parse("wss://test.relay").unwrap();
    let wrong_url = RelayUrl::parse("wss://wrong.relay").unwrap();
    let auth = Nip42Auth::new(local_url.as_str().to_string());
    let challenge = "test_challenge";

    // Create auth event with wrong relay
    let unsigned_event = EventBuilder::new(Kind::Authentication, "")
        .tag(Tag::from_standardized(TagStandard::Challenge(challenge.to_string())))
        .tag(Tag::from_standardized(TagStandard::Relay(wrong_url)))
        .build_with_ctx(&Instant::now(), keys.public_key());
    let event = keys.sign_event(unsigned_event).await.unwrap();

    // Verify event signature
    assert!(event.verify().is_ok());

    let result = auth.authed_pubkey(&event, Some(challenge));
    assert_eq!(result, None);
}

#[tokio::test]
async fn test_wrong_signature() {
    let auth_keys = Keys::generate();
    let wrong_keys = Keys::generate();
    let local_url = RelayUrl::parse("wss://test.relay").unwrap();
    let auth = Nip42Auth::new(local_url.as_str().to_string());
    let challenge = "test_challenge";

    // Create auth event with wrong_keys but claiming to be from auth_keys
    let unsigned_event = EventBuilder::new(Kind::Authentication, "")
        .tag(Tag::from_standardized(TagStandard::Challenge(challenge.to_string())))
        .tag(Tag::from_standardized(TagStandard::Relay(local_url)))
        .build_with_ctx(&Instant::now(), auth_keys.public_key()); // Claim to be auth_keys
    let event = wrong_keys.sign_event(unsigned_event).await.unwrap(); // But sign with wrong_keys

    // Verify event signature should fail
    assert!(event.verify().is_err());

    // Should fail because signature doesn't match the claimed pubkey
    let result = auth.authed_pubkey(&event, Some(challenge));
    assert_eq!(result, None);
}
