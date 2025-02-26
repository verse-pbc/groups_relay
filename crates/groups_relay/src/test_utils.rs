use nostr_sdk::prelude::*;
use std::sync::Arc;
use std::time::Instant;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use crate::groups::group::Group;
use crate::nostr_database::RelayDatabase;
use crate::nostr_session_state::NostrConnectionState;

pub async fn setup_test() -> (TempDir, Arc<RelayDatabase>, Keys) {
    let tmp_dir = TempDir::new().unwrap();
    let db_path = tmp_dir.path().join("test.db");
    let keys = Keys::generate();
    let database =
        Arc::new(RelayDatabase::new(db_path.to_str().unwrap().to_string(), keys.clone()).unwrap());
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
    let token = CancellationToken::new();
    NostrConnectionState {
        challenge: None,
        authed_pubkey: pubkey,
        relay_url: RelayUrl::parse("ws://test.relay").expect("Invalid test relay URL"),
        relay_connection: None,
        connection_token: token.clone(),
        event_start_time: None,
        event_kind: None,
    }
}

pub async fn create_test_group(admin_keys: &Keys) -> (Group, String) {
    let group_id = "test_group";
    let event = create_test_event(
        admin_keys,
        9007,
        vec![Tag::custom(TagKind::h(), [group_id])],
    )
    .await;
    let group = Group::new(&event).unwrap();
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

pub async fn create_test_metadata_event(
    admin_keys: &Keys,
    group_id: &str,
    name: Option<&str>,
    about: Option<&str>,
    picture: Option<&str>,
    is_private: bool,
    is_closed: bool,
) -> Event {
    let mut tags = vec![Tag::custom(TagKind::h(), [group_id])];

    if let Some(name) = name {
        tags.push(Tag::custom(TagKind::Name, [name]));
    }
    if let Some(about) = about {
        tags.push(Tag::custom(TagKind::Custom("about".into()), [about]));
    }
    if let Some(picture) = picture {
        tags.push(Tag::custom(TagKind::Custom("picture".into()), [picture]));
    }
    if is_private {
        tags.push(Tag::custom(TagKind::Custom("private".into()), [""]));
    } else {
        tags.push(Tag::custom(TagKind::Custom("public".into()), [""]));
    }
    if is_closed {
        tags.push(Tag::custom(TagKind::Custom("closed".into()), [""]));
    } else {
        tags.push(Tag::custom(TagKind::Custom("open".into()), [""]));
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
