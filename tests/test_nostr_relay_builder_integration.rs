//! Integration test to verify groups_relay works with nostr_relay_builder

use groups_relay::{
    config::Keys, groups::Groups, groups_event_processor::GroupsRelayProcessor, RelayDatabase,
};
use nostr_relay_builder::{crypto_worker::CryptoWorker, AuthConfig, RelayBuilder, RelayConfig};
use std::sync::Arc;
use tempfile::TempDir;
use tokio_util::task::TaskTracker;

#[tokio::test]
async fn test_groups_relay_with_nostr_relay_builder() -> anyhow::Result<()> {
    // Create temporary directory for database
    let temp_dir = TempDir::new()?;
    let db_path = temp_dir.path().join("test.db");

    let keys = Keys::generate();
    let task_tracker = TaskTracker::new();
    let crypto_worker = CryptoWorker::spawn(Arc::new(keys.clone()), &task_tracker);

    // groups_relay's database is used for groups management
    let (groups_database, db_sender) = RelayDatabase::new(&db_path, crypto_worker)?;
    let groups_database = Arc::new(groups_database);

    let groups = Arc::new(
        Groups::load_groups(
            groups_database.clone(),
            keys.public_key(),
            "wss://test.relay.com".to_string(),
        )
        .await?,
    );

    let relay_config = RelayConfig::new(
        "wss://test.groups.relay",
        (groups_database.clone(), db_sender),
        keys.clone(),
    )
    .with_subdomains(2)
    .with_auth(AuthConfig {
        relay_url: "wss://test.groups.relay".to_string(),
        validate_subdomains: true,
    });

    let groups_processor = GroupsRelayProcessor::new(groups, keys.public_key());

    let _handler = RelayBuilder::new(relay_config)
        .with_event_processor(groups_processor)
        .build()
        .await?;

    println!("✅ Successfully created groups relay using nostr_relay_builder!");

    Ok(())
}

#[test]
fn test_store_command_compatibility() {
    use nostr_lmdb::Scope;
    use nostr_sdk::prelude::*;

    // Test that StoreCommand types are compatible
    let scope = Scope::Default;
    // Create a proper test event
    let keys = Keys::generate();
    let event = EventBuilder::text_note("test content")
        .sign_with_keys(&keys)
        .unwrap();

    // Create groups_relay StoreCommand
    let groups_cmd =
        groups_relay::StoreCommand::SaveSignedEvent(Box::new(event.clone()), scope.clone());

    // Convert to nostr_relay_builder StoreCommand
    let relay_cmd = match groups_cmd {
        groups_relay::StoreCommand::SaveSignedEvent(e, s) => {
            nostr_relay_builder::StoreCommand::SaveSignedEvent(e, s)
        }
        groups_relay::StoreCommand::SaveUnsignedEvent(e, s) => {
            nostr_relay_builder::StoreCommand::SaveUnsignedEvent(e, s)
        }
        groups_relay::StoreCommand::DeleteEvents(f, s) => {
            nostr_relay_builder::StoreCommand::DeleteEvents(f, s)
        }
    };

    // Verify the conversion worked
    match relay_cmd {
        nostr_relay_builder::StoreCommand::SaveSignedEvent(e, s) => {
            assert_eq!(e.id.to_string(), event.id.to_string());
            assert!(matches!(s, Scope::Default));
        }
        _ => panic!("Unexpected command type"),
    }

    println!("✅ StoreCommand types are compatible!");
}
