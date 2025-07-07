use groups_relay::group::{Group, GroupRole, KIND_GROUP_ADMINS_39001, KIND_GROUP_MEMBERS_39002};
use nostr_sdk::prelude::*;

#[tokio::test]
async fn test_role_merging_from_metadata_events() {
    // Create test keys
    let admin_keys = Keys::generate();
    let relay_keys = Keys::generate();

    // Create a group create event
    let group_id = "test_role_merge_group";
    let create_event = EventBuilder::new(Kind::from(9007), "")
        .tag(Tag::custom(TagKind::h(), vec![group_id]))
        .sign_with_keys(&admin_keys)
        .unwrap();

    // Create group from the create event
    let mut group = Group::from(&create_event);

    // Create a 39001 (admins) event that lists the user as Admin (only Admin role, no Member)
    let admins_event = EventBuilder::new(KIND_GROUP_ADMINS_39001, "")
        .tag(Tag::custom(TagKind::d(), vec![group_id]))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin_keys.public_key().to_string(), "Admin".to_string()],
        ))
        .sign_with_keys(&relay_keys)
        .unwrap();

    // Load the admins event
    group.load_members_from_event(&admins_event).unwrap();

    // At this point, the user should be an admin
    assert!(
        group.is_admin(&admin_keys.public_key()),
        "User should be admin after loading 39001 event"
    );

    // Create a 39002 (members) event that lists the same user WITHOUT any role
    let members_event = EventBuilder::new(KIND_GROUP_MEMBERS_39002, "")
        .tag(Tag::custom(TagKind::d(), vec![group_id]))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin_keys.public_key().to_string()],
        )) // No role specified
        .sign_with_keys(&relay_keys)
        .unwrap();

    // Load the members event - this should NOT remove the Admin role
    group.load_members_from_event(&members_event).unwrap();

    // The user should STILL be an admin (roles should be merged, not replaced)
    assert!(
        group.is_admin(&admin_keys.public_key()),
        "User should still be admin after loading 39002 event - roles should be merged, not replaced!"
    );

    // Also verify the user has both Member and Admin roles
    let member = group.members.get(&admin_keys.public_key()).unwrap();
    assert!(
        member.roles.contains(&GroupRole::Admin),
        "Should have Admin role"
    );
    assert!(
        member.roles.contains(&GroupRole::Member),
        "Should have Member role"
    );
    assert_eq!(member.roles.len(), 2, "Should have exactly 2 roles");
}

#[tokio::test]
async fn test_can_edit_members_with_merged_roles() {
    // Create test keys
    let admin_keys = Keys::generate();
    let relay_keys = Keys::generate();

    // Create a group
    let group_id = "test_edit_members_group";
    let create_event = EventBuilder::new(Kind::from(9007), "")
        .tag(Tag::custom(TagKind::h(), vec![group_id]))
        .sign_with_keys(&admin_keys)
        .unwrap();

    let mut group = Group::from(&create_event);

    // Load admin role from 39001
    let admins_event = EventBuilder::new(KIND_GROUP_ADMINS_39001, "")
        .tag(Tag::custom(TagKind::d(), vec![group_id]))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin_keys.public_key().to_string(), "Admin".to_string()],
        ))
        .sign_with_keys(&relay_keys)
        .unwrap();

    group.load_members_from_event(&admins_event).unwrap();

    // Load member listing from 39002 (without role)
    let members_event = EventBuilder::new(KIND_GROUP_MEMBERS_39002, "")
        .tag(Tag::custom(TagKind::d(), vec![group_id]))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin_keys.public_key().to_string()],
        ))
        .sign_with_keys(&relay_keys)
        .unwrap();

    group.load_members_from_event(&members_event).unwrap();

    // User should be able to edit members
    assert!(
        group.can_edit_members(&admin_keys.public_key(), &relay_keys.public_key()),
        "Admin should be able to edit members even after 39002 event loaded"
    );
}

#[tokio::test]
async fn test_group_creator_remains_admin_after_members_list() {
    // This test simulates the exact scenario where a user creates a group
    // and the backend generates both 39001 and 39002 events

    let creator_keys = Keys::generate();
    let relay_keys = Keys::generate();
    let other_member_keys = Keys::generate();

    // Create a group (simulating what happens when user creates a group)
    let group_id = "test_creator_admin_group";
    let create_event = EventBuilder::new(
        Kind::from(9007), // KIND_GROUP_CREATE_9007
        "",
    )
    .tag(Tag::custom(TagKind::h(), vec![group_id]))
    .sign_with_keys(&creator_keys)
    .unwrap();

    // Create group and verify creator is admin
    let mut group = Group::new(&create_event, nostr_lmdb::Scope::Default).unwrap();
    assert!(
        group.is_admin(&creator_keys.public_key()),
        "Creator should be admin immediately after group creation"
    );

    // Add another member to make it more realistic
    group.add_pubkey(other_member_keys.public_key()).unwrap();

    // Generate the events that would be created by the backend
    let admins_unsigned = group
        .generate_admins_event(&relay_keys.public_key())
        .unwrap();
    let members_unsigned = group.generate_members_event(&relay_keys.public_key());

    // Convert UnsignedEvent to Event
    let admins_event = admins_unsigned.sign(&relay_keys).await.unwrap();
    let members_event = members_unsigned.sign(&relay_keys).await.unwrap();

    // Verify the generated events are correct
    assert_eq!(admins_event.kind, KIND_GROUP_ADMINS_39001);
    assert_eq!(members_event.kind, KIND_GROUP_MEMBERS_39002);

    // Create a fresh group to simulate loading from database
    let mut loaded_group = Group::from(&create_event);

    // Load events in the order they might be processed
    // First load admins list
    loaded_group.load_members_from_event(&admins_event).unwrap();
    assert!(
        loaded_group.is_admin(&creator_keys.public_key()),
        "Creator should be admin after loading 39001"
    );

    // Then load members list - this is where the bug was happening
    loaded_group
        .load_members_from_event(&members_event)
        .unwrap();

    // Verify creator is STILL an admin
    assert!(
        loaded_group.is_admin(&creator_keys.public_key()),
        "Creator should remain admin after loading 39002 members list!"
    );

    // Verify both members are in the group
    assert_eq!(loaded_group.members.len(), 2, "Should have 2 members");
    assert!(
        loaded_group.is_member(&creator_keys.public_key()),
        "Creator should be a member"
    );
    assert!(
        loaded_group.is_member(&other_member_keys.public_key()),
        "Other user should be a member"
    );

    // Verify creator has both Admin and Member roles
    let creator_member = loaded_group
        .members
        .get(&creator_keys.public_key())
        .unwrap();
    assert!(
        creator_member.roles.contains(&GroupRole::Admin),
        "Creator should have Admin role"
    );
    assert!(
        creator_member.roles.contains(&GroupRole::Member),
        "Creator should have Member role"
    );

    // Verify other member only has Member role
    let other_member = loaded_group
        .members
        .get(&other_member_keys.public_key())
        .unwrap();
    assert!(
        !other_member.roles.contains(&GroupRole::Admin),
        "Other member should NOT have Admin role"
    );
    assert!(
        other_member.roles.contains(&GroupRole::Member),
        "Other member should have Member role"
    );
}

#[tokio::test]
async fn test_multiple_admins_preserved_after_members_event() {
    // Test that multiple admins are preserved when members list is loaded

    let admin1_keys = Keys::generate();
    let admin2_keys = Keys::generate();
    let member_keys = Keys::generate();
    let relay_keys = Keys::generate();

    let group_id = "test_multiple_admins_group";
    let create_event = EventBuilder::new(Kind::from(9007), "")
        .tag(Tag::custom(TagKind::h(), vec![group_id]))
        .sign_with_keys(&admin1_keys)
        .unwrap();

    let mut group = Group::from(&create_event);

    // Create 39001 event with two admins
    let admins_event = EventBuilder::new(KIND_GROUP_ADMINS_39001, "")
        .tag(Tag::custom(TagKind::d(), vec![group_id]))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin1_keys.public_key().to_string(), "Admin".to_string()],
        ))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin2_keys.public_key().to_string(), "Admin".to_string()],
        ))
        .sign_with_keys(&relay_keys)
        .unwrap();

    group.load_members_from_event(&admins_event).unwrap();

    // Create 39002 event listing all members (including admins)
    let members_event = EventBuilder::new(KIND_GROUP_MEMBERS_39002, "")
        .tag(Tag::custom(TagKind::d(), vec![group_id]))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin1_keys.public_key().to_string()],
        ))
        .tag(Tag::custom(
            TagKind::p(),
            vec![admin2_keys.public_key().to_string()],
        ))
        .tag(Tag::custom(
            TagKind::p(),
            vec![member_keys.public_key().to_string()],
        ))
        .sign_with_keys(&relay_keys)
        .unwrap();

    group.load_members_from_event(&members_event).unwrap();

    // Verify all admins are still admins
    assert!(
        group.is_admin(&admin1_keys.public_key()),
        "Admin1 should still be admin"
    );
    assert!(
        group.is_admin(&admin2_keys.public_key()),
        "Admin2 should still be admin"
    );
    assert!(
        !group.is_admin(&member_keys.public_key()),
        "Regular member should not be admin"
    );

    // Verify everyone is a member
    assert!(
        group.is_member(&admin1_keys.public_key()),
        "Admin1 should be member"
    );
    assert!(
        group.is_member(&admin2_keys.public_key()),
        "Admin2 should be member"
    );
    assert!(
        group.is_member(&member_keys.public_key()),
        "Regular member should be member"
    );
}

#[tokio::test]
async fn test_admins_event_only_contains_admin_roles() {
    // Test that 39001 events only contain admin roles, not member roles

    let creator_keys = Keys::generate();
    let relay_keys = Keys::generate();

    // Create a group where creator is admin
    let group_id = "test_admin_roles_only";
    let create_event = EventBuilder::new(Kind::from(9007), "")
        .tag(Tag::custom(TagKind::h(), vec![group_id]))
        .sign_with_keys(&creator_keys)
        .unwrap();

    let group = Group::new(&create_event, nostr_lmdb::Scope::Default).unwrap();

    // Generate the 39001 event
    let admins_unsigned = group
        .generate_admins_event(&relay_keys.public_key())
        .unwrap();
    let admins_event = admins_unsigned.sign(&relay_keys).await.unwrap();

    // Check the tags
    let p_tags: Vec<_> = admins_event
        .tags
        .iter()
        .filter(|t| t.kind() == TagKind::p())
        .collect();

    assert_eq!(p_tags.len(), 1, "Should have exactly one admin");

    // Get the roles from the p tag
    let admin_tag = p_tags[0];

    // Check tag structure using as_slice()
    let tag_slice = admin_tag.as_slice();

    // Should be ["p", pubkey, "Admin"] - NO "Member" role
    assert!(tag_slice.len() >= 3, "Tag should have at least 3 elements");
    assert_eq!(tag_slice[0], "p");
    assert_eq!(tag_slice[1], creator_keys.public_key().to_string());
    assert_eq!(tag_slice[2], "Admin");

    // Ensure there's no "Member" role in the tag
    assert_eq!(
        tag_slice.len(),
        3,
        "Should only have p, pubkey, and Admin role"
    );
    assert!(
        !tag_slice.contains(&"Member".to_string()),
        "Admin tag should not contain Member role"
    );
}
