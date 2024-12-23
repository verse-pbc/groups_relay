pub mod group;

use crate::error::Error;
use anyhow::Result;
use dashmap::{
    mapref::one::{Ref, RefMut},
    DashMap,
};
pub use group::{
    Group, GroupMember, GroupMetadata, GroupRole, Invite, GROUP_CONTENT_KINDS, KIND_GROUP_ADD_USER,
    KIND_GROUP_ADMINS, KIND_GROUP_CREATE, KIND_GROUP_CREATE_INVITE, KIND_GROUP_DELETE,
    KIND_GROUP_DELETE_EVENT, KIND_GROUP_EDIT_METADATA, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA,
    KIND_GROUP_REMOVE_USER, KIND_GROUP_SET_ROLES, KIND_GROUP_USER_JOIN_REQUEST,
    KIND_GROUP_USER_LEAVE_REQUEST, METADATA_EVENT_KINDS,
};
use nostr_database::NostrEventsDatabase;
use nostr_ndb::NdbDatabase;
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use tracing::info;

#[derive(Debug)]
pub struct Groups {
    groups: DashMap<String, Group>,
}

impl Groups {
    pub async fn load_groups(database: Arc<NdbDatabase>) -> Result<Self, Error> {
        let mut groups = HashMap::new();
        info!("Loading groups from relay...");

        // Step 1: Load current state from replaceable events
        let metadata_filter = vec![Filter::new()
            .kinds(vec![
                KIND_GROUP_METADATA, // 39000
                KIND_GROUP_ADMINS,   // 39001
                KIND_GROUP_MEMBERS,  // 39002
            ])
            .since(Timestamp::from(0))];

        let Ok(metadata_events) = database.query(metadata_filter).await else {
            return Err(Error::notice("Error querying metadata events"));
        };
        info!("Found {} metadata events", metadata_events.len());

        // Process events in order to build current state
        for event in metadata_events.clone() {
            let group_id = match Group::extract_group_id(&event) {
                Some(id) => id,
                None => return Err(Error::notice("Group ID not found")),
            };

            if event.kind == KIND_GROUP_METADATA {
                info!("[{}] Processing metadata", group_id);
                groups
                    .entry(group_id.to_string())
                    .or_insert_with(|| Group::from(&event))
                    .load_metadata_from_event(&event)?;
            } else if event.kind == KIND_GROUP_ADMINS || event.kind == KIND_GROUP_MEMBERS {
                info!("[{}] Processing members", group_id);
                groups
                    .entry(group_id.to_string())
                    .or_insert_with(|| Group::from(&event))
                    .load_members_from_event(&event)?;
            }
        }

        // Step 2: Load historical data for each group
        info!("Processing {} groups", groups.len());
        for (group_id, group) in groups.iter_mut() {
            info!("[{}] Loading historical data", group_id);

            let historical_filter = vec![Filter::new()
                .kinds(vec![
                    KIND_GROUP_CREATE,            // 9007
                    KIND_GROUP_USER_JOIN_REQUEST, // 9021
                    KIND_GROUP_CREATE_INVITE,     // 9009
                ])
                .custom_tag(SingleLetterTag::lowercase(Alphabet::H), vec![group_id])
                .since(Timestamp::from(0))];

            let Ok(historical_events) = database.query(historical_filter).await else {
                return Err(Error::notice("Error querying historical events"));
            };
            info!(
                "[{}] Found {} historical events",
                group_id,
                historical_events.len()
            );

            for event in historical_events {
                if event.kind == KIND_GROUP_CREATE {
                    info!("[{}] Found creation event", group_id);
                    group.created_at = event.created_at;
                } else if event.kind == KIND_GROUP_USER_JOIN_REQUEST {
                    info!("[{}] Processing join request", group_id);
                    group.load_join_request_from_event(&event)?;
                } else if event.kind == KIND_GROUP_CREATE_INVITE {
                    info!("[{}] Processing invite", group_id);
                    group.load_invite_from_event(&event)?;
                }
            }

            // Update timestamps
            group.updated_at = metadata_events
                .iter()
                .map(|e| e.created_at)
                .max()
                .unwrap_or(group.updated_at);
        }

        Ok(Self {
            groups: DashMap::from_iter(groups),
        })
    }

    pub fn get_group(&self, group_id: &str) -> Option<Ref<String, Group>> {
        self.groups.get(group_id)
    }

    pub fn get_group_mut(&self, group_id: &str) -> Option<RefMut<String, Group>> {
        self.groups.get_mut(group_id)
    }

    pub fn find_group_from_event_mut<'a>(
        &'a self,
        event: &Event,
    ) -> Result<Option<RefMut<'a, String, Group>>, Error> {
        let Some(group_id) = Group::extract_group_id(event) else {
            return Ok(None);
        };

        let mut group = self.get_group_mut(group_id);

        if let Some(ref mut group) = group {
            group.verify_member_access(&event.pubkey, event.kind)?;
        }

        Ok(group)
    }

    pub fn find_group_from_event<'a>(&'a self, event: &Event) -> Option<Ref<'a, String, Group>> {
        let group_id = Group::extract_group_id(event)?;
        self.get_group(group_id)
    }

    pub fn find_group_from_event_h_tag<'a>(
        &'a self,
        event: &Event,
    ) -> Option<Ref<'a, String, Group>> {
        let group_id = Group::extract_group_h_tag(event)?;
        self.get_group(group_id)
    }

    pub fn handle_group_create(&self, event: &Event) -> Result<Group, Error> {
        if let Some(group) = self.find_group_from_event(event) {
            return Err(Error::notice("Group already exists"));
        }

        let group = Group::new(event)?;
        self.groups.insert(group.id.to_string(), group.clone());
        Ok(group)
    }

    pub fn handle_set_roles(&self, event: &Event) -> Result<(), Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.set_roles(event)
    }

    pub fn handle_put_user(&self, event: &Event) -> Result<bool, Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.add_members(event)
    }

    pub fn handle_remove_user(&self, event: &Event) -> Result<bool, Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.remove_members(event)
    }

    pub fn handle_edit_metadata(&self, event: &Event) -> Result<(), Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.set_metadata(event)
    }

    pub fn handle_create_invite(&self, event: &Event) -> Result<(), Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.create_invite(event)?;
        Ok(())
    }

    pub fn handle_join_request(&self, event: &Event) -> Result<bool, Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.join_request(event)
    }

    pub fn handle_leave_request(&self, event: &Event) -> Result<bool, Error> {
        let mut group = self
            .find_group_from_event_mut(event)?
            .ok_or(Error::notice("Group not found"))?;

        group.leave_request(event)
    }
}

impl Deref for Groups {
    type Target = DashMap<String, Group>;

    fn deref(&self) -> &Self::Target {
        &self.groups
    }
}

impl DerefMut for Groups {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::{EventBuilder, Keys, NostrSigner};
    use std::time::Instant;

    async fn create_test_keys() -> (Keys, Keys, Keys) {
        (Keys::generate(), Keys::generate(), Keys::generate())
    }

    async fn create_test_event(keys: &Keys, kind: Kind, tags: Vec<Tag>) -> Event {
        let unsigned_event = EventBuilder::new(kind, "")
            .tags(tags)
            .build_with_ctx(&Instant::now(), keys.public_key());
        keys.sign_event(unsigned_event).await.unwrap()
    }

    async fn setup_test_groups() -> (Groups, Keys, Keys, Keys, String) {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let group_id = "test_group_123".to_string();
        let tags = vec![Tag::custom(TagKind::h(), [group_id.clone()])];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE, tags).await;

        let groups = Groups {
            groups: DashMap::new(),
        };
        groups.handle_group_create(&event).unwrap();

        (groups, admin_keys, member_keys, non_member_keys, group_id)
    }

    #[tokio::test]
    async fn test_handle_group_create() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Test creating a duplicate group
        let tags = vec![Tag::custom(TagKind::h(), [group_id.clone()])];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE, tags).await;
        assert!(groups.handle_group_create(&event).is_err());

        // Verify group exists and admin is set
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_admin(&admin_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_set_roles() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Add a member with admin role
        let tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::custom(
                TagKind::p(),
                [member_keys.public_key().to_string(), "Admin".to_string()],
            ),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_SET_ROLES, tags).await;
        assert!(groups.handle_set_roles(&event).is_ok());

        // Verify member has admin role
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_admin(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_put_user() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Add a member
        let tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::public_key(member_keys.public_key()),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER, tags).await;
        assert!(groups.handle_put_user(&event).unwrap());

        // Verify member was added
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_remove_user() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // First add a member
        let add_tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER, add_tags).await;
        groups.handle_put_user(&add_event).unwrap();

        // Then remove them
        let remove_tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::public_key(member_keys.public_key()),
        ];
        let remove_event =
            create_test_event(&admin_keys, KIND_GROUP_REMOVE_USER, remove_tags).await;
        assert!(groups.handle_remove_user(&remove_event).unwrap());

        // Verify member was removed
        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_edit_metadata() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Edit metadata
        let tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::custom(TagKind::Name, ["New Group Name"]),
            Tag::custom(TagKind::custom("about"), ["About text"]),
            Tag::custom(TagKind::custom("picture"), ["picture_url"]),
            Tag::custom(TagKind::custom("public"), &[] as &[String]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA, tags).await;
        assert!(groups.handle_edit_metadata(&event).is_ok());

        // Verify metadata was updated
        let group = groups.get_group(&group_id).unwrap();
        assert_eq!(group.metadata.name, "New Group Name");
        assert_eq!(group.metadata.about, Some("About text".to_string()));
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
        assert!(!group.metadata.private);
    }

    #[tokio::test]
    async fn test_handle_create_invite() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Create invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE, tags).await;
        assert!(groups.handle_create_invite(&event).is_ok());

        // Verify invite was created
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.invites.contains_key(invite_code));

        // Drop the group reference before proceeding
        drop(group);

        // Test using invite
        let join_tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST, join_tags).await;
        assert!(groups.handle_join_request(&join_event).unwrap());

        // Verify member was added
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_join_leave_requests() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Test join request
        let join_tags = vec![Tag::custom(TagKind::h(), [group_id.clone()])];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST, join_tags).await;
        assert!(!groups.handle_join_request(&join_event).unwrap());

        // Manually add member
        let add_tags = vec![
            Tag::custom(TagKind::h(), [group_id.clone()]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER, add_tags).await;
        groups.handle_put_user(&add_event).unwrap();

        // Test leave request
        let leave_tags = vec![Tag::custom(TagKind::h(), [group_id.clone()])];
        let leave_event =
            create_test_event(&member_keys, KIND_GROUP_USER_LEAVE_REQUEST, leave_tags).await;
        assert!(groups.handle_leave_request(&leave_event).unwrap());

        // Verify member was removed
        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.is_member(&member_keys.public_key()));
    }
}
