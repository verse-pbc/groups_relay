use groups_relay::groups::{
    Groups, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007, KIND_GROUP_CREATE_INVITE_9009,
    KIND_GROUP_MEMBERS_39002, KIND_GROUP_USER_JOIN_REQUEST_9021,
};
use nostr_lmdb::Scope;
use nostr_relay_builder::{RelayDatabase, StoreCommand};
use nostr_sdk::prelude::*;
use std::sync::Arc;
use tempfile::TempDir;
use tokio_util::task::TaskTracker;

#[tokio::test]
async fn test_join_request_generates_correct_events() {
    // Setup
    let temp_dir = TempDir::new().unwrap();
    let admin_keys = Keys::generate();
    let user_keys = Keys::generate();

    let task_tracker = TaskTracker::new();
    
    let (db, _db_sender) = RelayDatabase::with_task_tracker(
        temp_dir
            .path()
            .join("test.db")
            .to_string_lossy()
            .to_string(),
        Arc::new(admin_keys.clone()),
        task_tracker,
    )
    .unwrap();
    let db = Arc::new(db);

    let groups = Arc::new(
        Groups::load_groups(
            db.clone(),
            admin_keys.public_key(),
            "wss://test.relay.com".to_string(),
        )
        .await
        .unwrap(),
    );

    let scope = Scope::Default;
    let group_id = "test_group_123";

    // Create group
    let create_event = EventBuilder::new(KIND_GROUP_CREATE_9007, "")
        .tags(vec![Tag::custom(TagKind::h(), [group_id])])
        .sign_with_keys(&admin_keys)
        .unwrap();

    println!("Creating group...");
    let commands = groups
        .handle_group_create(Box::new(create_event), &scope)
        .await
        .unwrap();
    println!("Group creation returned {} commands", commands.len());

    // Create invite
    let invite_code = "TESTINVITE123";
    let invite_event = EventBuilder::new(KIND_GROUP_CREATE_INVITE_9009, "")
        .tags(vec![
            Tag::custom(TagKind::h(), [group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ])
        .sign_with_keys(&admin_keys)
        .unwrap();

    println!("\nCreating invite...");
    let commands = groups
        .handle_create_invite(Box::new(invite_event), &scope)
        .unwrap();
    println!("Invite creation returned {} commands", commands.len());

    // User joins with invite
    let join_event = EventBuilder::new(KIND_GROUP_USER_JOIN_REQUEST_9021, "")
        .tags(vec![
            Tag::custom(TagKind::h(), [group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ])
        .sign_with_keys(&user_keys)
        .unwrap();

    println!(
        "\nUser {} joining with invite code...",
        user_keys.public_key()
    );
    let commands = groups
        .handle_join_request(Box::new(join_event), &scope)
        .unwrap();
    println!("Join request returned {} commands", commands.len());

    // Verify we got the expected commands
    assert!(
        commands.len() >= 3,
        "Expected at least 3 commands (join event + membership events)"
    );

    let mut found_members_event = false;
    let mut user_in_members = false;

    // Analyze the commands
    for (i, cmd) in commands.iter().enumerate() {
        match cmd {
            StoreCommand::SaveSignedEvent(event, scope, None) => {
                println!("\nCommand {i}: SaveSignedEvent");
                println!("  Kind: {}", event.kind);
                println!("  Author: {}", event.pubkey);
                println!("  Scope: {scope:?}");
                assert_eq!(event.kind, KIND_GROUP_USER_JOIN_REQUEST_9021);
            }
            StoreCommand::SaveUnsignedEvent(event, scope, None) => {
                println!("\nCommand {i}: SaveUnsignedEvent");
                println!("  Kind: {}", event.kind);
                println!("  Pubkey: {}", event.pubkey);
                println!("  Scope: {scope:?}");

                if event.kind == KIND_GROUP_MEMBERS_39002 {
                    found_members_event = true;
                    println!("  Members list:");
                    for tag in event.tags.iter() {
                        if tag.kind() == TagKind::p() {
                            if let Some(pubkey) = tag.content() {
                                println!("    - {pubkey}");
                                if pubkey == user_keys.public_key().to_string() {
                                    user_in_members = true;
                                }
                            }
                        }
                    }
                } else if event.kind == KIND_GROUP_ADD_USER_9000 {
                    println!("  Add user event details:");
                    for tag in event.tags.iter() {
                        println!("    Tag: {tag:?}");
                    }
                }
            }
            _ => println!("\nCommand {i}: Other"),
        }
    }

    // Assertions
    assert!(
        found_members_event,
        "Should have generated a 39002 members event"
    );
    assert!(user_in_members, "User should be in the 39002 members list");

    // Check group state
    let group = groups.get_group(&scope, group_id).unwrap();
    println!("\nFinal group state:");
    println!("  Members count: {}", group.value().members.len());
    println!(
        "  Is user {} a member? {}",
        user_keys.public_key(),
        group.value().is_member(&user_keys.public_key())
    );

    assert_eq!(group.value().members.len(), 2); // Admin + new user
    assert!(group.value().is_member(&user_keys.public_key()));
}
