use nostr_sdk::{EventBuilder, Keys, Kind, NostrSigner, Tag};
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::nostr_database::NostrDatabase;
use crate::nostr_session_state::NostrConnectionState;

pub async fn setup_test() -> (TempDir, Arc<NostrDatabase>, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let keys = Keys::generate();
    let database =
        Arc::new(NostrDatabase::new(db_path.to_str().unwrap().to_string(), keys.clone()).unwrap());
    (tmp_dir, database, keys)
}

pub async fn create_test_keys() -> (Keys, Keys, Keys) {
    (Keys::generate(), Keys::generate(), Keys::generate())
}

pub async fn create_test_event(keys: &Keys, kind: u16, tags: Vec<Tag>) -> nostr_sdk::Event {
    let event = EventBuilder::new(Kind::Custom(kind), "test")
        .tags(tags)
        .build_with_ctx(&Instant::now(), keys.public_key());
    keys.sign_event(event).await.unwrap()
}

pub fn create_test_state(pubkey: Option<nostr_sdk::PublicKey>) -> NostrConnectionState {
    NostrConnectionState {
        relay_url: "wss://test.relay".to_string(),
        challenge: None,
        authed_pubkey: pubkey,
        relay_connection: None,
        connection_token: CancellationToken::new(),
    }
}
