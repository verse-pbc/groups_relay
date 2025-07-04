use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;

use crate::group::Group;
use nostr_relay_builder::{DatabaseSender, NostrConnectionState, RelayDatabase};
use tokio_util::task::TaskTracker;

pub async fn setup_test() -> (TempDir, Arc<RelayDatabase>, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let keys = Keys::generate();
    let task_tracker = TaskTracker::new();
    let (database, _db_sender) = RelayDatabase::with_task_tracker(
        db_path.to_str().unwrap(),
        Arc::new(keys.clone()),
        task_tracker,
    )
    .unwrap();
    let database = Arc::new(database);
    (tmp_dir, database, keys)
}

pub async fn setup_test_with_sender() -> (TempDir, Arc<RelayDatabase>, DatabaseSender, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let keys = Keys::generate();
    let task_tracker = TaskTracker::new();
    let (database, db_sender) = RelayDatabase::with_task_tracker(
        db_path.to_str().unwrap(),
        Arc::new(keys.clone()),
        task_tracker,
    )
    .unwrap();
    let database = Arc::new(database);
    (tmp_dir, database, db_sender, keys)
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

pub async fn create_test_group(admin_keys: &Keys) -> (Group, String) {
    let group_id = "test_group";
    let event = create_test_event(
        admin_keys,
        9007,
        vec![Tag::custom(TagKind::h(), [group_id])],
    )
    .await;
    let group = Group::new(&event, nostr_lmdb::Scope::Default).unwrap();
    (group, group_id.to_string())
}

pub async fn create_test_group_with_members(
    admin_keys: &Keys,
    member_keys: &Keys,
) -> (Group, String) {
    let (mut group, group_id) = create_test_group(admin_keys).await;

    // Add member to group
    let add_member_event = create_test_event(
        admin_keys,
        9000,
        vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ],
    )
    .await;

    group
        .add_members_from_event(Box::new(add_member_event), &admin_keys.public_key())
        .unwrap();

    (group, group_id)
}

pub async fn create_test_group_with_multiple_admins(
    admin_keys: &Keys,
    second_admin_keys: &Keys,
) -> (Group, String) {
    let (mut group, group_id) = create_test_group(admin_keys).await;

    // Add second admin to group
    let add_admin_event = create_test_event(
        admin_keys,
        9000,
        vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(second_admin_keys.public_key()),
            Tag::custom(TagKind::Custom("role".into()), ["admin"]),
        ],
    )
    .await;

    group
        .add_members_from_event(Box::new(add_admin_event), &admin_keys.public_key())
        .unwrap();

    (group, group_id)
}

pub async fn add_member_to_group(
    group: &mut Group,
    admin_keys: &Keys,
    member_keys: &Keys,
    group_id: &str,
) -> Event {
    let add_tags = vec![
        Tag::custom(TagKind::h(), [group_id]),
        Tag::public_key(member_keys.public_key()),
    ];
    let add_event = create_test_event(admin_keys, 9000, add_tags).await;

    group
        .add_members_from_event(Box::new(add_event.clone()), &admin_keys.public_key())
        .unwrap();

    add_event
}

pub async fn remove_member_from_group(
    group: &mut Group,
    admin_keys: &Keys,
    member_keys: &Keys,
    group_id: &str,
) -> Event {
    let remove_tags = vec![
        Tag::custom(TagKind::h(), [group_id]),
        Tag::public_key(member_keys.public_key()),
    ];
    let remove_event = create_test_event(admin_keys, 9001, remove_tags).await;

    group
        .remove_members(Box::new(remove_event.clone()), &admin_keys.public_key())
        .unwrap();

    remove_event
}

#[derive(Default)]
pub struct TestGroupMetadata<'a> {
    pub name: Option<&'a str>,
    pub about: Option<&'a str>,
    pub picture: Option<&'a str>,
    pub is_private: bool,
    pub is_closed: bool,
    pub is_broadcast: bool,
}

pub async fn create_test_metadata_event(
    admin_keys: &Keys,
    group_id: &str,
    metadata: TestGroupMetadata<'_>,
) -> Event {
    let mut tags = vec![Tag::custom(TagKind::h(), [group_id])];

    if let Some(name) = metadata.name {
        tags.push(Tag::custom(TagKind::Name, [name]));
    }
    if let Some(about) = metadata.about {
        tags.push(Tag::custom(TagKind::Custom("about".into()), [about]));
    }
    if let Some(picture) = metadata.picture {
        tags.push(Tag::custom(TagKind::Custom("picture".into()), [picture]));
    }
    if metadata.is_private {
        tags.push(Tag::custom(TagKind::Custom("private".into()), [""]));
    } else {
        tags.push(Tag::custom(TagKind::Custom("public".into()), [""]));
    }
    if metadata.is_closed {
        tags.push(Tag::custom(TagKind::Custom("closed".into()), [""]));
    } else {
        tags.push(Tag::custom(TagKind::Custom("open".into()), [""]));
    }
    if metadata.is_broadcast {
        tags.push(Tag::custom(TagKind::Custom("broadcast".into()), [""]));
    } else {
        tags.push(Tag::custom(TagKind::Custom("nonbroadcast".into()), [""]));
    }

    create_test_event(admin_keys, 9002, tags).await
}

pub async fn create_test_invite_event(
    admin_keys: &Keys,
    group_id: &str,
    invite_code: &str,
) -> Event {
    let tags = vec![
        Tag::custom(TagKind::h(), [group_id]),
        Tag::custom(TagKind::Custom("code".into()), [invite_code]),
    ];
    create_test_event(admin_keys, 9009, tags).await
}

pub async fn create_test_reusable_invite_event(
    admin_keys: &Keys,
    group_id: &str,
    invite_code: &str,
) -> Event {
    let tags = vec![
        Tag::custom(TagKind::h(), [group_id]),
        Tag::custom(TagKind::Custom("code".into()), [invite_code]),
        Tag::custom(TagKind::Custom("reusable".into()), Vec::<String>::new()),
    ];
    create_test_event(admin_keys, 9009, tags).await
}

pub async fn create_test_delete_event(
    keys: &Keys,
    group_id: &str,
    event_to_delete: &Event,
) -> Event {
    let tags = vec![
        Tag::custom(TagKind::h(), [group_id]),
        Tag::event(event_to_delete.id),
    ];
    create_test_event(keys, 9005, tags).await
}

pub async fn create_test_role_event(
    admin_keys: &Keys,
    group_id: &str,
    member_pubkey: PublicKey,
    role: &str,
) -> Event {
    let tags = vec![
        Tag::custom(TagKind::h(), [group_id]),
        Tag::custom(TagKind::p(), [member_pubkey.to_string(), role.to_string()]),
    ];
    create_test_event(admin_keys, 9006, tags).await
}
