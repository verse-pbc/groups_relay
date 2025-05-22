use groups_relay::groups::group::{Group, GroupRole, KIND_GROUP_ADMINS_39001, KIND_GROUP_MEMBERS_39002};
use nostr_sdk::prelude::*;

#[tokio::test]
async fn test_role_merging_from_metadata_events() {
    // Create test keys
    let admin_keys = Keys::generate();
    let relay_keys = Keys::generate();
    
    // Create a group create event
    let group_id = "test_role_merge_group";
    let create_event = EventBuilder::new(
        Kind::from(9007),
        "",
    )
    .tag(Tag::custom(TagKind::h(), vec![group_id]))
    .sign_with_keys(&admin_keys)
    .unwrap();
    
    // Create group from the create event
    let mut group = Group::from(&create_event);
    
    // Create a 39001 (admins) event that lists the user as Admin
    let admins_event = EventBuilder::new(
        KIND_GROUP_ADMINS_39001,
        "",
    )
    .tag(Tag::custom(TagKind::d(), vec![group_id]))
    .tag(Tag::custom(TagKind::p(), vec![admin_keys.public_key().to_string(), "Admin".to_string()]))
    .sign_with_keys(&relay_keys)
    .unwrap();
    
    // Load the admins event
    group.load_members_from_event(&admins_event).unwrap();
    
    // At this point, the user should be an admin
    assert!(group.is_admin(&admin_keys.public_key()), "User should be admin after loading 39001 event");
    
    // Create a 39002 (members) event that lists the same user WITHOUT any role
    let members_event = EventBuilder::new(
        KIND_GROUP_MEMBERS_39002,
        "",
    )
    .tag(Tag::custom(TagKind::d(), vec![group_id]))
    .tag(Tag::custom(TagKind::p(), vec![admin_keys.public_key().to_string()])) // No role specified
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
    assert!(member.roles.contains(&GroupRole::Admin), "Should have Admin role");
    assert!(member.roles.contains(&GroupRole::Member), "Should have Member role");
    assert_eq!(member.roles.len(), 2, "Should have exactly 2 roles");
}

#[tokio::test]
async fn test_can_edit_members_with_merged_roles() {
    // Create test keys
    let admin_keys = Keys::generate();
    let relay_keys = Keys::generate();
    
    // Create a group
    let group_id = "test_edit_members_group";
    let create_event = EventBuilder::new(
        Kind::from(9007),
        "",
    )
    .tag(Tag::custom(TagKind::h(), vec![group_id]))
    .sign_with_keys(&admin_keys)
    .unwrap();
    
    let mut group = Group::from(&create_event);
    
    // Load admin role from 39001
    let admins_event = EventBuilder::new(
        KIND_GROUP_ADMINS_39001,
        "",
    )
    .tag(Tag::custom(TagKind::d(), vec![group_id]))
    .tag(Tag::custom(TagKind::p(), vec![admin_keys.public_key().to_string(), "Admin".to_string()]))
    .sign_with_keys(&relay_keys)
    .unwrap();
    
    group.load_members_from_event(&admins_event).unwrap();
    
    // Load member listing from 39002 (without role)
    let members_event = EventBuilder::new(
        KIND_GROUP_MEMBERS_39002,
        "",
    )
    .tag(Tag::custom(TagKind::d(), vec![group_id]))
    .tag(Tag::custom(TagKind::p(), vec![admin_keys.public_key().to_string()]))
    .sign_with_keys(&relay_keys)
    .unwrap();
    
    group.load_members_from_event(&members_event).unwrap();
    
    // User should be able to edit members
    assert!(
        group.can_edit_members(&admin_keys.public_key(), &relay_keys.public_key()),
        "Admin should be able to edit members even after 39002 event loaded"
    );
}