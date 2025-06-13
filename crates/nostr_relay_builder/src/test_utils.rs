use crate::crypto_worker::CryptoWorker;
use crate::database::RelayDatabase;
use crate::state::NostrConnectionState;
use crate::subscription_service::SubscriptionService;
use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use websocket_builder::MessageSender;

pub async fn setup_test() -> (TempDir, Arc<RelayDatabase>, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let keys = Keys::generate();
    let cancellation_token = CancellationToken::new();
    let crypto_worker = Arc::new(CryptoWorker::new(
        Arc::new(keys.clone()),
        cancellation_token,
    ));
    let database = Arc::new(RelayDatabase::new(db_path.to_str().unwrap(), crypto_worker).unwrap());
    (tmp_dir, database, keys)
}

pub async fn create_test_keys() -> (Keys, Keys, Keys) {
    (Keys::generate(), Keys::generate(), Keys::generate())
}

pub async fn create_test_event(keys: &Keys, kind: u16, tags: Vec<Tag>) -> nostr_sdk::Event {
    let created_at = Timestamp::now_with_supplier(&Instant::now());

    let mut unsigned = UnsignedEvent::new(
        keys.public_key(),
        created_at,
        Kind::Custom(kind),
        tags.clone(),
        "",
    );

    unsigned.ensure_id();

    unsigned.sign_with_keys(keys).unwrap()
}

pub fn create_test_state(pubkey: Option<nostr_sdk::PublicKey>) -> NostrConnectionState {
    let mut state = NostrConnectionState::new("ws://test.relay".to_string())
        .expect("Failed to create test state");
    state.authed_pubkey = pubkey;
    state
}

pub async fn create_test_state_with_subscription_service(
    pubkey: Option<nostr_sdk::PublicKey>,
    database: Arc<RelayDatabase>,
) -> (
    NostrConnectionState,
    mpsc::Receiver<(RelayMessage<'static>, usize)>,
) {
    let (tx, rx) = mpsc::channel(10);
    let sender = MessageSender::new(tx, 0);

    let subscription_service = SubscriptionService::new(database, sender)
        .await
        .expect("Failed to create subscription service");

    let mut state = NostrConnectionState::new("ws://test.relay".to_string())
        .expect("Failed to create test state");
    state.authed_pubkey = pubkey;

    // Use the test method to add the subscription service
    state.set_subscription_service(subscription_service);

    (state, rx)
}
