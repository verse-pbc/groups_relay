pub mod group;

use crate::error::Error;
use crate::metrics;
use crate::nostr_database::RelayDatabase;
use crate::StoreCommand;
use anyhow::Result;
use dashmap::{
    mapref::one::{Ref, RefMut},
    DashMap,
};
pub use group::{
    Group, GroupError, GroupMember, GroupMetadata, GroupRole, Invite, ADDRESSABLE_EVENT_KINDS,
    KIND_GROUP_ADD_USER_9000, KIND_GROUP_ADMINS_39001, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_MEMBERS_39002, KIND_GROUP_METADATA_39000,
    KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006, KIND_GROUP_USER_JOIN_REQUEST_9021,
    KIND_GROUP_USER_LEAVE_REQUEST_9022, KIND_SIMPLE_LIST_10009, NON_GROUP_ALLOWED_KINDS,
};
use nostr_sdk::prelude::*;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use tracing::info;

#[derive(Debug)]
pub struct Groups {
    db: Arc<RelayDatabase>,
    groups: DashMap<String, Group>,
    pub relay_pubkey: PublicKey,
}

impl Groups {
    pub async fn load_groups(
        database: Arc<RelayDatabase>,
        relay_pubkey: PublicKey,
    ) -> Result<Self, Error> {
        let mut groups = HashMap::new();
        info!("Loading groups from relay...");

        // Step 1: Load current state from replaceable events
        let metadata_filter = vec![Filter::new()
            .kinds(vec![
                KIND_GROUP_METADATA_39000, // 39000
                KIND_GROUP_ADMINS_39001,   // 39001
                KIND_GROUP_MEMBERS_39002,  // 39002
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
                None => {
                    return Err(Error::notice(format!(
                        "Group ID not found in event: {:?}",
                        event
                    )))
                }
            };

            if event.kind == KIND_GROUP_METADATA_39000 {
                info!("[{}] Processing metadata", group_id);
                groups
                    .entry(group_id.to_string())
                    .or_insert_with(|| Group::from(&event))
                    .load_metadata_from_event(&event)?;
            } else if event.kind == KIND_GROUP_ADMINS_39001
                || event.kind == KIND_GROUP_MEMBERS_39002
            {
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
                    KIND_GROUP_CREATE_9007,            // 9007
                    KIND_GROUP_USER_JOIN_REQUEST_9021, // 9021
                    KIND_GROUP_CREATE_INVITE_9009,     // 9009
                ])
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::H),
                    group_id.to_string(),
                )
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
                if event.kind == KIND_GROUP_CREATE_9007 {
                    info!("[{}] Found creation event", group_id);
                    group.created_at = event.created_at;
                } else if event.kind == KIND_GROUP_USER_JOIN_REQUEST_9021 {
                    info!("[{}] Processing join request", group_id);
                    group.load_join_request_from_event(&event)?;
                } else if event.kind == KIND_GROUP_CREATE_INVITE_9009 {
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
            db: database,
            groups: DashMap::from_iter(groups),
            relay_pubkey,
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

        let mut group_ref_opt = self.get_group_mut(group_id); // Lock acquired here

        if let Some(ref mut group_ref) = group_ref_opt {
            if event.pubkey != self.relay_pubkey && event.kind != KIND_GROUP_USER_LEAVE_REQUEST_9022
            {
                let verification_result = group_ref.verify_member_access(&event.pubkey, event.kind);
                if let Err(e) = verification_result {
                    return Err(e);
                }
            }
        }

        Ok(group_ref_opt)
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

    /// Handles group creation events (KIND_GROUP_CREATE_9007).
    /// Creates a group and generates associated metadata and membership events.
    pub async fn handle_group_create(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let Some(group_id) = Group::extract_group_id(&event) else {
            return Err(Error::notice("Group ID not found in event"));
        };

        if self.find_group_from_event(&event).is_some() {
            return Err(Error::notice("Group already exists"));
        }

        // If a group with this id existed (kind 9008), we don't let it be created again
        let deleted_events = match self
            .db
            .query(vec![Filter::new()
                .kinds(vec![KIND_GROUP_DELETE_9008])
                .custom_tag(
                    SingleLetterTag::lowercase(Alphabet::H),
                    group_id.to_string(),
                )])
            .await
        {
            Ok(events) => events,
            Err(e) => return Err(Error::notice(format!("Error querying database: {}", e))),
        };

        if !deleted_events.is_empty() {
            return Err(Error::notice("Group existed before and was deleted"));
        }

        // Find all previous participants in unmanaged group
        let previous_events = match self
            .db
            .query(vec![Filter::new().custom_tag(
                SingleLetterTag::lowercase(Alphabet::H),
                group_id.to_string(),
            )])
            .await
        {
            Ok(events) => events,
            Err(e) => return Err(Error::notice(format!("Error querying database: {}", e))),
        };

        let mut group = Group::new(&event)?;

        // Only allow migrating unmanaged groups to managed ones if creator is relay admin
        if !previous_events.is_empty() && event.pubkey != self.relay_pubkey {
            return Err(Error::notice(
                "Only relay admin can create a managed group from an unmanaged one",
            ));
        }

        // Add all previous participants as members
        let mut previous_participants = std::collections::HashSet::new();
        for prev_event in previous_events {
            // Skip any group management events
            if Group::is_group_management_kind(prev_event.kind) {
                continue;
            }
            previous_participants.insert(prev_event.pubkey);
        }

        for pubkey in previous_participants {
            if pubkey != event.pubkey {
                // Skip creator as they're already an admin
                group.add_pubkey(pubkey)?;
            }
        }

        self.groups.insert(group.id.to_string(), group.clone());

        metrics::groups_created().increment(1);

        let mut commands = vec![StoreCommand::SaveSignedEvent(event)];
        commands.extend(
            group
                .generate_all_state_events(&self.relay_pubkey)
                .into_iter()
                .map(StoreCommand::SaveUnsignedEvent),
        );

        Ok(commands)
    }

    pub fn handle_set_roles(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or(Error::notice("[SetRoles] Group not found"))?;

        group.set_roles(event, &self.relay_pubkey)
    }

    pub fn handle_put_user(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or(Error::notice("[PutUser] Group not found"))?;

        group.add_members_from_event(event, &self.relay_pubkey)
    }

    pub fn handle_remove_user(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or(Error::notice("[RemoveUser] Group not found"))?;

        group.remove_members(event, &self.relay_pubkey)
    }

    pub fn handle_group_content(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or(Error::notice("[GroupManagement] Group not found"))?;

        group.handle_group_content(event, &self.relay_pubkey)
    }

    pub fn handle_edit_metadata(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or(Error::notice("[EditMetadata] Group not found"))?;

        group.set_metadata(&event, &self.relay_pubkey)?;

        let mut commands = vec![StoreCommand::SaveSignedEvent(event)];
        commands.extend(
            group
                .generate_metadata_events(&self.relay_pubkey)
                .into_iter()
                .map(StoreCommand::SaveUnsignedEvent),
        );

        Ok(commands)
    }

    pub fn handle_create_invite(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let created;
        {
            let mut group = self
                .find_group_from_event_mut(&event)?
                .ok_or(Error::notice("[CreateInvite] Group not found"))?;
            created = group.create_invite(&event, &self.relay_pubkey)?;
        }

        if created {
            Ok(vec![StoreCommand::SaveSignedEvent(event)])
        } else {
            Ok(vec![StoreCommand::SaveSignedEvent(event)])
        }
    }

    pub fn handle_join_request(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let result;
        {
            let mut group = self
                .find_group_from_event_mut(&event)?
                .ok_or(Error::notice("[JoinRequest] Group not found"))?;

            result = group.join_request(event, &self.relay_pubkey);
        }

        result
    }

    pub fn handle_leave_request(&self, event: Box<Event>) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or(Error::notice("[LeaveRequest] Group not found"))?;

        group.leave_request(event, &self.relay_pubkey)
    }

    pub fn handle_delete_event(
        &self,
        event: Box<Event>,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        let mut group = self
            .find_group_from_event_mut(&event)?
            .ok_or_else(|| Error::notice("Group not found for this group content"))?;

        let commands = group.delete_event_request(event, &self.relay_pubkey, authed_pubkey)?;
        Ok(commands)
    }

    pub fn handle_delete_group(
        &self,
        event: Box<Event>,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        let group = self
            .find_group_from_event(&event)
            .ok_or_else(|| Error::notice("[DeleteGroup] Group not found"))?;

        let group_key = group.key().clone();
        let commands = group.delete_group_request(event, &self.relay_pubkey, authed_pubkey)?;
        drop(group);

        self.groups.remove(&group_key);

        Ok(commands)
    }

    /// Returns counts of groups by their privacy settings
    pub fn count_groups_by_privacy(&self) -> [(bool, bool, usize); 4] {
        let mut counts = [
            (false, false, 0),
            (false, true, 0),
            (true, false, 0),
            (true, true, 0),
        ];

        for group in self.iter() {
            let group = group.value();
            let idx = match (group.metadata.private, group.metadata.closed) {
                (false, false) => 0,
                (false, true) => 1,
                (true, false) => 2,
                (true, true) => 3,
            };
            counts[idx].2 += 1;
        }

        counts
    }

    /// Returns counts of active groups by their privacy settings
    pub async fn count_active_groups_by_privacy(&self) -> Result<[(bool, bool, usize); 4], Error> {
        let mut counts = [
            (false, false, 0),
            (false, true, 0),
            (true, false, 0),
            (true, true, 0),
        ];

        for group in self.iter() {
            let group = group.value();
            // First check member count
            if group.members.len() < 2 {
                continue;
            }

            // Then check for content events
            let events = match self
                .db
                .query(vec![Filter::new()
                    .custom_tag(
                        SingleLetterTag::lowercase(Alphabet::H),
                        group.id.to_string(),
                    )
                    .limit(1)])
                .await
            {
                Ok(events) => events,
                Err(e) => return Err(Error::notice(format!("Error querying database: {}", e))),
            };

            // Check if any event is a content event (not a 9xxx management event)
            let has_content = events.iter().any(|e| match e.kind {
                Kind::Custom(k) => !(9000..=9999).contains(&k),
                _ => true,
            });

            if has_content {
                let idx = match (group.metadata.private, group.metadata.closed) {
                    (false, false) => 0,
                    (false, true) => 1,
                    (true, false) => 2,
                    (true, true) => 3,
                };
                counts[idx].2 += 1;
            }
        }

        Ok(counts)
    }

    /// Returns the number of active groups (groups with 2+ members and at least one content event)
    pub async fn count_active_groups(&self) -> Result<usize, Error> {
        let counts = self.count_active_groups_by_privacy().await?;
        Ok(counts.iter().map(|(_, _, count)| count).sum())
    }

    /// Verifies if a user has access to a group
    /// Returns Ok(()) if access is allowed, or an appropriate error if not
    pub fn verify_group_access(
        &self,
        group: &Group,
        pubkey: Option<PublicKey>,
    ) -> Result<(), GroupError> {
        if !group.metadata.private {
            return Ok(());
        }

        let pubkey = pubkey.ok_or_else(|| {
            GroupError::PermissionDenied("Authentication required for private group".to_string())
        })?;

        if pubkey == self.relay_pubkey || group.is_member(&pubkey) {
            Ok(())
        } else {
            Err(GroupError::PermissionDenied(
                "Not a member of this private group".to_string(),
            ))
        }
    }

    /// Verifies if a user has access to a group by ID
    /// Returns Ok(()) if access is allowed, or an appropriate error if not
    pub fn verify_group_access_by_id(
        &self,
        group_id: &str,
        pubkey: Option<PublicKey>,
    ) -> Result<(), GroupError> {
        let group = self
            .get_group(group_id)
            .ok_or_else(|| GroupError::NotFound(format!("Group {} not found", group_id)))?;
        self.verify_group_access(&group, pubkey)
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
    use crate::nostr_database::RelayDatabase;
    use std::time::Instant;
    use tempfile::TempDir;

    const TEST_GROUP_ID: &str = "test_group_123";

    async fn create_test_keys() -> (Keys, Keys, Keys) {
        (Keys::generate(), Keys::generate(), Keys::generate())
    }

    async fn create_test_event(keys: &Keys, kind: Kind, tags: Vec<Tag>) -> Box<Event> {
        let unsigned_event = EventBuilder::new(kind, "")
            .tags(tags)
            .build_with_ctx(&Instant::now(), keys.public_key());
        let event = keys.sign_event(unsigned_event).await.unwrap();
        Box::new(event)
    }

    async fn create_test_groups_with_db(admin_keys: &Keys) -> Groups {
        let temp_dir = TempDir::new().unwrap();
        let db = Arc::new(
            RelayDatabase::new(
                temp_dir
                    .path()
                    .join("test.db")
                    .to_string_lossy()
                    .to_string(),
                admin_keys.clone(),
            )
            .unwrap(),
        );

        std::mem::forget(temp_dir);

        Groups {
            db,
            groups: DashMap::new(),
            relay_pubkey: admin_keys.public_key(),
        }
    }

    async fn setup_test_groups() -> (Groups, Keys, Keys, Keys, String) {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let tags = vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE_9007, tags).await;

        let groups = create_test_groups_with_db(&admin_keys).await;
        groups.handle_group_create(event).await.unwrap();

        (
            groups,
            admin_keys,
            member_keys,
            non_member_keys,
            TEST_GROUP_ID.to_string(),
        )
    }

    #[tokio::test]
    async fn test_handle_group_create_sets_admin() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Verify group exists and admin is set
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_admin(&admin_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_group_create_rejects_duplicate_group() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Test creating a duplicate group
        let tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE_9007, tags).await;
        assert!(groups.handle_group_create(event).await.is_err());
    }

    #[tokio::test]
    async fn test_handle_set_roles_admin_can_promote_member() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let groups = create_test_groups_with_db(&admin_keys).await;

        // Create group and add member
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007,
            vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])],
        )
        .await;
        groups.handle_group_create(create_event).await.unwrap();

        let add_event = create_test_event(
            &admin_keys,
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(add_event).unwrap();

        // Promote member to admin
        let set_roles_event = create_test_event(
            &admin_keys,
            KIND_GROUP_SET_ROLES_9006,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::custom(
                    TagKind::p(),
                    [member_keys.public_key().to_string(), "admin".to_string()],
                ),
            ],
        )
        .await;

        groups.handle_set_roles(set_roles_event).unwrap();

        let group = groups.get_group(TEST_GROUP_ID).unwrap();
        assert!(group
            .members
            .get(&member_keys.public_key())
            .unwrap()
            .is(GroupRole::Admin));
    }

    #[tokio::test]
    async fn test_handle_set_roles_non_admin_cannot_set_roles() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let groups = create_test_groups_with_db(&admin_keys).await;

        // Create group and add member
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007,
            vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])],
        )
        .await;
        groups.handle_group_create(create_event).await.unwrap();

        let add_event = create_test_event(
            &admin_keys,
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(add_event).unwrap();

        // Attempt to set roles as non-admin
        let set_roles_event = create_test_event(
            &non_member_keys,
            KIND_GROUP_SET_ROLES_9006,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::custom(
                    TagKind::p(),
                    [member_keys.public_key().to_string(), "admin".to_string()],
                ),
            ],
        )
        .await;

        assert!(groups.handle_set_roles(set_roles_event).is_err());
    }

    #[tokio::test]
    async fn test_handle_put_user_admin_can_add_member() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let groups = create_test_groups_with_db(&admin_keys).await;

        // Create group
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007,
            vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])],
        )
        .await;
        groups.handle_group_create(create_event).await.unwrap();

        // Add member
        let add_event = create_test_event(
            &admin_keys,
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        let result = groups.handle_put_user(add_event).unwrap();
        assert!(!result.is_empty());

        let group = groups.get_group(TEST_GROUP_ID).unwrap();
        assert!(group.members.contains_key(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_put_user_non_admin_cannot_add_member() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let groups = create_test_groups_with_db(&admin_keys).await;

        // Create group
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007,
            vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])],
        )
        .await;
        groups.handle_group_create(create_event).await.unwrap();

        // Attempt to add member as non-admin
        let add_event = create_test_event(
            &non_member_keys,
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        assert!(groups.handle_put_user(add_event).is_err());
    }

    #[tokio::test]
    async fn test_handle_remove_user_admin_can_remove_member() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let groups = create_test_groups_with_db(&admin_keys).await;

        // Create group and add member
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007,
            vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])],
        )
        .await;
        groups.handle_group_create(create_event).await.unwrap();

        let add_event = create_test_event(
            &admin_keys,
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(add_event).unwrap();

        // Remove member
        let remove_event = create_test_event(
            &admin_keys,
            KIND_GROUP_REMOVE_USER_9001,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::custom(TagKind::p(), [member_keys.public_key().to_string()]),
            ],
        )
        .await;

        let result = groups.handle_remove_user(remove_event);
        assert!(!result.unwrap().is_empty());

        let group = groups.get_group(TEST_GROUP_ID).unwrap();
        assert!(!group.members.contains_key(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_remove_user_non_admin_cannot_remove_member() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let groups = create_test_groups_with_db(&admin_keys).await;

        // Create group and add member
        let create_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_9007,
            vec![Tag::custom(TagKind::h(), [TEST_GROUP_ID])],
        )
        .await;
        groups.handle_group_create(create_event).await.unwrap();

        let add_event = create_test_event(
            &admin_keys,
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;
        groups.handle_put_user(add_event).unwrap();

        // Attempt to remove member as non-admin
        let remove_event = create_test_event(
            &non_member_keys,
            KIND_GROUP_REMOVE_USER_9001,
            vec![
                Tag::custom(TagKind::h(), [TEST_GROUP_ID]),
                Tag::public_key(member_keys.public_key()),
            ],
        )
        .await;

        assert!(groups.handle_remove_user(remove_event).is_err());
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_can_set_name() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Name, ["New Group Name"]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_ok());

        let group = groups.get_group(&group_id).unwrap();
        assert_eq!(group.metadata.name, "New Group Name");
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_can_set_about() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("about"), ["About text"]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_ok());

        let group = groups.get_group(&group_id).unwrap();
        assert_eq!(group.metadata.about, Some("About text".to_string()));
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_can_set_picture() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("picture"), ["picture_url"]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_ok());

        let group = groups.get_group(&group_id).unwrap();
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_can_set_visibility() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("public"), &[] as &[String]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_ok());

        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.metadata.private);
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_can_set_multiple_fields() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Name, ["New Group Name"]),
            Tag::custom(TagKind::custom("about"), ["About text"]),
            Tag::custom(TagKind::custom("picture"), ["picture_url"]),
            Tag::custom(TagKind::custom("public"), &[] as &[String]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_ok());

        let group = groups.get_group(&group_id).unwrap();
        assert_eq!(group.metadata.name, "New Group Name");
        assert_eq!(group.metadata.about, Some("About text".to_string()));
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
        assert!(!group.metadata.private);
    }

    #[tokio::test]
    async fn test_handle_create_invite_creates_valid_invite() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Create invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags.clone()).await;
        groups.handle_create_invite(event).unwrap();

        // Verify invite was created
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.invites.contains_key(invite_code));
    }

    #[tokio::test]
    async fn test_handle_create_invite_can_be_used_to_join() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Create invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags.clone()).await;
        groups.handle_create_invite(event).unwrap();

        // Use invite
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;
        assert!(!groups.handle_join_request(join_event).unwrap().is_empty());

        // Verify member was added
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_create_invite_marks_invite_as_used() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Create invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags.clone()).await;
        groups.handle_create_invite(event).unwrap();

        // Use invite
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;
        groups.handle_join_request(join_event).unwrap();
    }

    #[tokio::test]
    async fn test_handle_join_request_with_valid_invite() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Create invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags.clone()).await;
        groups.handle_create_invite(event).unwrap();

        // Use invite to join
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;
        assert!(!groups.handle_join_request(join_event).unwrap().is_empty());

        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_join_request_with_invalid_invite() {
        let (groups, _, member_keys, _, group_id) = setup_test_groups().await;

        // Try to join with invalid invite code
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), ["invalid_code"]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;

        // According to NIP-29, the join request should be saved
        let result = groups.handle_join_request(join_event).unwrap();
        assert_eq!(result.len(), 1, "Join request should be saved");

        match &result[0] {
            StoreCommand::SaveSignedEvent(event) => {
                assert_eq!(event.kind, KIND_GROUP_USER_JOIN_REQUEST_9021);
            }
            _ => panic!("Expected SaveSignedEvent command"),
        }

        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_join_request_without_invite_adds_to_requests() {
        let (groups, _, member_keys, _, group_id) = setup_test_groups().await;

        // Join without invite code
        let join_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;

        // According to NIP-29, the join request should be saved
        let result = groups.handle_join_request(join_event).unwrap();
        assert_eq!(result.len(), 1, "Join request should be saved");

        match &result[0] {
            StoreCommand::SaveSignedEvent(event) => {
                assert_eq!(event.kind, KIND_GROUP_USER_JOIN_REQUEST_9021);
            }
            _ => panic!("Expected SaveSignedEvent command"),
        }

        let group = groups.get_group(&group_id).unwrap();
        // The join request should be added to the join_requests set
        assert!(group.join_requests.contains(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_leave_request_member_can_leave() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Add member first
        let add_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        groups.handle_put_user(add_event).unwrap();

        // Test leave request
        let leave_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let leave_event =
            create_test_event(&member_keys, KIND_GROUP_USER_LEAVE_REQUEST_9022, leave_tags).await;

        // Get the store commands
        let leave_event_id = leave_event.id;
        let commands = groups.handle_leave_request(leave_event).unwrap();

        // Verify the commands
        assert_eq!(
            commands.len(),
            2,
            "Should have 2 commands: save leave event and update members"
        );
        match &commands[0] {
            StoreCommand::SaveSignedEvent(event) => assert_eq!(event.id, leave_event_id),
            _ => panic!("First command should be SaveSignedEvent"),
        }
        match &commands[1] {
            StoreCommand::SaveUnsignedEvent(event) => {
                assert_eq!(event.kind, KIND_GROUP_MEMBERS_39002);
                // Verify the member is not in the members list
                assert!(!event
                    .tags
                    .filter(TagKind::p())
                    .filter_map(|t| t.content())
                    .any(|t| t == member_keys.public_key().to_string()));
            }
            _ => panic!("Second command should be SaveUnsignedEvent for members"),
        }

        // Also verify the state change
        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_leave_request_non_member_cannot_leave() {
        let (groups, _, non_member_keys, _, group_id) = setup_test_groups().await;

        // Test leave request from non-member
        let leave_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let leave_event = create_test_event(
            &non_member_keys,
            KIND_GROUP_USER_LEAVE_REQUEST_9022,
            leave_tags,
        )
        .await;
        assert!(groups.handle_leave_request(leave_event).unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_handle_leave_request_admin_can_leave_if_not_last_admin() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Add member as admin
        let add_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        groups.handle_put_user(add_event).unwrap();

        // Make member an admin
        let set_roles_event = create_test_event(
            &admin_keys,
            KIND_GROUP_SET_ROLES_9006,
            vec![
                Tag::custom(TagKind::h(), [&group_id]),
                Tag::custom(
                    TagKind::p(),
                    [member_keys.public_key().to_string(), "admin".to_string()],
                ),
            ],
        )
        .await;
        groups.handle_set_roles(set_roles_event).unwrap();

        // Original admin tries to leave
        let leave_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let leave_event =
            create_test_event(&admin_keys, KIND_GROUP_USER_LEAVE_REQUEST_9022, leave_tags).await;
        assert!(!groups.handle_leave_request(leave_event).unwrap().is_empty());

        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.is_member(&admin_keys.public_key()));
        assert!(group.is_admin(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_leave_request_last_admin_can_leave() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Test leave request from last admin
        let leave_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let leave_event =
            create_test_event(&admin_keys, KIND_GROUP_USER_LEAVE_REQUEST_9022, leave_tags).await;

        // The last admin should not be able to leave
        let result = groups.handle_leave_request(leave_event);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Cannot remove last admin");

        // Verify admin is still in the group
        let group = groups.get_group(&group_id).unwrap();
        assert!(group.is_member(&admin_keys.public_key()));
        assert!(group.is_admin(&admin_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_leave_request_removes_from_join_requests() {
        let (groups, _, member_keys, _, group_id) = setup_test_groups().await;

        // Add to join requests
        let join_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;
        groups.handle_join_request(join_event).unwrap();

        // Test leave request
        let leave_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let leave_event =
            create_test_event(&member_keys, KIND_GROUP_USER_LEAVE_REQUEST_9022, leave_tags).await;
        assert!(groups.handle_leave_request(leave_event).unwrap().is_empty());

        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.join_requests.contains(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_non_admin_cannot_edit() {
        let (groups, _, non_member_keys, _, group_id) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Name, ["New Group Name"]),
        ];
        let event = create_test_event(&non_member_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_member_cannot_edit() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Add member first
        let add_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        groups.handle_put_user(add_event).unwrap();

        // Try to edit metadata as member
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Name, ["New Group Name"]),
        ];
        let event = create_test_event(&member_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_rejects_invalid_group() {
        let (groups, admin_keys, _, _, _) = setup_test_groups().await;

        let tags = vec![
            Tag::custom(TagKind::h(), ["invalid_group_id"]),
            Tag::custom(TagKind::Name, ["New Group Name"]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;
        assert!(groups.handle_edit_metadata(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_edit_metadata_preserves_unmodified_fields() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // First set multiple fields
        let initial_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Name, ["Initial Name"]),
            Tag::custom(TagKind::custom("about"), ["Initial About"]),
            Tag::custom(TagKind::custom("picture"), ["initial_picture_url"]),
        ];
        let initial_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, initial_tags).await;
        groups.handle_edit_metadata(initial_event).unwrap();

        // Then update only the name
        let update_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Name, ["Updated Name"]),
        ];
        let update_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, update_tags).await;
        groups.handle_edit_metadata(update_event).unwrap();

        // Verify other fields are preserved
        let group = groups.get_group(&group_id).unwrap();
        assert_eq!(group.metadata.name, "Updated Name");
        assert_eq!(group.metadata.about, Some("Initial About".to_string()));
        assert_eq!(
            group.metadata.picture,
            Some("initial_picture_url".to_string())
        );
    }

    #[tokio::test]
    async fn test_handle_create_invite_non_admin_cannot_create() {
        let (groups, _, non_member_keys, _, group_id) = setup_test_groups().await;

        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event = create_test_event(&non_member_keys, KIND_GROUP_CREATE_INVITE_9009, tags).await;
        assert!(groups.handle_create_invite(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_create_invite_member_cannot_create() {
        let (groups, admin_keys, member_keys, _, group_id) = setup_test_groups().await;

        // Add member first
        let add_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        groups.handle_put_user(add_event).unwrap();

        // Try to create invite as member
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event = create_test_event(&member_keys, KIND_GROUP_CREATE_INVITE_9009, tags).await;
        assert!(groups.handle_create_invite(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_create_invite_rejects_duplicate_code() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        // Create first invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags.clone()).await;
        groups.handle_create_invite(event).unwrap();

        // Try to create invite with same code
        let duplicate_event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags).await;
        assert!(groups.handle_create_invite(duplicate_event).is_err());
    }

    #[tokio::test]
    async fn test_handle_create_invite_rejects_missing_code() {
        let (groups, admin_keys, _, _, group_id) = setup_test_groups().await;

        let tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags).await;
        assert!(groups.handle_create_invite(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_create_invite_rejects_invalid_group() {
        let (groups, admin_keys, _, _, _) = setup_test_groups().await;

        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), ["invalid_group_id"]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event = create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags).await;
        assert!(groups.handle_create_invite(event).is_err());
    }

    #[tokio::test]
    async fn test_handle_join_request_with_used_invite() {
        let (groups, admin_keys, member_keys, non_member_keys, group_id) =
            setup_test_groups().await;

        // Create and use invite
        let invite_code = "test_invite_123";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags.clone()).await;
        groups.handle_create_invite(event).unwrap();

        // First member uses invite
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;
        groups.handle_join_request(join_event).unwrap();

        // Second member tries to use same invite
        let join_tags2 = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event2 = create_test_event(
            &non_member_keys,
            KIND_GROUP_USER_JOIN_REQUEST_9021,
            join_tags2,
        )
        .await;
        groups.handle_join_request(join_event2).unwrap();

        // With single-use invites, second user should be added to join_requests instead of members
        let group = groups.get_group(&group_id).unwrap();
        assert!(!group.is_member(&non_member_keys.public_key()));
        assert!(group.join_requests.contains(&non_member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_handle_join_request_with_reusable_invite() {
        let (groups, admin_keys, member_keys, non_member_keys, group_id) =
            setup_test_groups().await;

        // Create reusable invite - note: using local create_test_event that returns Box<Event>
        let invite_code = "test_reusable_invite";
        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
            // Add reusable tag
            Tag::custom(TagKind::custom("reusable"), Vec::<String>::new()),
        ];
        let create_invite_event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, tags).await;
        groups.handle_create_invite(create_invite_event).unwrap();

        // Verify the invite exists and is reusable - IN A SCOPE
        {
            let group = groups.get_group(&group_id).unwrap();
            assert!(group.invites.contains_key(invite_code));
            assert!(group.invites.get(invite_code).unwrap().reusable);
        }

        // First user joins with reusable invite
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;
        groups.handle_join_request(join_event).unwrap();

        // Verify first user was added - IN A SCOPE
        {
            let group = groups.get_group(&group_id).unwrap();
            assert!(group.is_member(&member_keys.public_key()));
        }

        // Second user tries to use the same reusable invite
        let join_tags2 = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::custom("code"), [invite_code]),
        ];
        let join_event2 = create_test_event(
            &non_member_keys,
            KIND_GROUP_USER_JOIN_REQUEST_9021,
            join_tags2,
        )
        .await;
        groups.handle_join_request(join_event2).unwrap();

        // With reusable invites, both users should become members - IN A SCOPE
        {
            let group = groups.get_group(&group_id).unwrap();
            assert!(group.is_member(&member_keys.public_key()));
            assert!(group.is_member(&non_member_keys.public_key()));
        }
    }
}
