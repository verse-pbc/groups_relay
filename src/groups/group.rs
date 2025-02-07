use crate::error::Error;
use crate::StoreCommand;
use anyhow::Result;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{collections::HashMap, collections::HashSet};
use strum::Display;
use strum::EnumIter;
use strum::IntoEnumIterator;
use tracing::{debug, error, info, warn};

// Group Creation and Management
pub const KIND_GROUP_CREATE_9007: Kind = Kind::Custom(9007); // Admin/Relay -> Relay: Create a new group
pub const KIND_GROUP_DELETE_9008: Kind = Kind::Custom(9008); // Admin/Relay -> Relay: Delete an existing group

// Admin/Moderation Actions (9000-9005)
pub const KIND_GROUP_ADD_USER_9000: Kind = Kind::Custom(9000); // Admin/Relay -> Relay: Add user to group
pub const KIND_GROUP_REMOVE_USER_9001: Kind = Kind::Custom(9001); // Admin/Relay -> Relay: Remove user from group
pub const KIND_GROUP_EDIT_METADATA_9002: Kind = Kind::Custom(9002); // Admin/Relay -> Relay: Edit group metadata
pub const KIND_GROUP_DELETE_EVENT_9005: Kind = Kind::Custom(9005); // Admin/Relay -> Relay: Delete specific event
pub const KIND_GROUP_SET_ROLES_9006: Kind = Kind::Custom(9006); // Admin/Relay -> Relay: Set roles for group. This was removed but at least 0xchat uses it
pub const KIND_GROUP_CREATE_INVITE_9009: Kind = Kind::Custom(9009); // Admin/Relay -> Relay: Create invite for closed group

// User Actions (9021-9022)
pub const KIND_GROUP_USER_JOIN_REQUEST_9021: Kind = Kind::Custom(9021); // User -> Relay: Request to join group
pub const KIND_GROUP_USER_LEAVE_REQUEST_9022: Kind = Kind::Custom(9022); // User -> Relay: Request to leave group

// Relay-Generated Events (39000-39003)
pub const KIND_GROUP_METADATA_39000: Kind = Kind::Custom(39000); // Relay -> All: Group metadata
pub const KIND_GROUP_ADMINS_39001: Kind = Kind::Custom(39001); // Relay -> All: List of group admins
pub const KIND_GROUP_MEMBERS_39002: Kind = Kind::Custom(39002); // Relay -> All: List of group members
pub const KIND_GROUP_ROLES_39003: Kind = Kind::Custom(39003); // Relay -> All: Supported roles in group

// Non Group Content Kinds
pub const KIND_SIMPLE_LIST_10009: Kind = Kind::Custom(10009); // Simple Groups (NIP-51): List of groups a user wants to remember being in
pub const KIND_CLAIM_28934: Kind = Kind::Custom(28934); // Claim (NIP-43): Claim auth

pub const ADDRESSABLE_EVENT_KINDS: [Kind; 4] = [
    KIND_GROUP_METADATA_39000,
    KIND_GROUP_ADMINS_39001,
    KIND_GROUP_MEMBERS_39002,
    KIND_GROUP_ROLES_39003,
];

pub const NON_GROUP_ALLOWED_KINDS: [Kind; 2] = [KIND_SIMPLE_LIST_10009, KIND_CLAIM_28934];

pub const ALL_GROUP_KINDS_EXCEPT_DELETE_AND_ADDRESSABLE: [Kind; 10] = [
    KIND_GROUP_CREATE_9007,
    KIND_GROUP_ADD_USER_9000,
    KIND_GROUP_REMOVE_USER_9001,
    KIND_GROUP_EDIT_METADATA_9002,
    KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_CREATE_INVITE_9009,
    KIND_GROUP_USER_JOIN_REQUEST_9021,
    KIND_GROUP_USER_LEAVE_REQUEST_9022,
    KIND_SIMPLE_LIST_10009,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMetadata {
    pub name: String,
    pub about: Option<String>,
    pub picture: Option<String>,
    /// Private = needs authentication to read
    pub private: bool,
    /// Closed = automatic creation of 9000 events when a 9021 comes
    pub closed: bool,
}

impl GroupMetadata {
    pub fn new(name: String) -> Self {
        Self {
            name,
            about: None,
            picture: None,
            private: true,
            closed: true,
        }
    }
}

#[derive(Display, Debug, Clone, Serialize, Deserialize, EnumIter, PartialEq, Eq, Hash)]
pub enum GroupRole {
    Admin,
    Member,
    Custom(String),
}

impl GroupRole {
    fn as_tuple(&self) -> (&str, &str) {
        match self {
            GroupRole::Admin => ("admin", "Can edit metadata and manage users"),
            GroupRole::Member => ("member", "Regular group member"),
            GroupRole::Custom(name) => (name, "Custom role"),
        }
    }
}

impl FromStr for GroupRole {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            return Ok(GroupRole::Member);
        }

        match s.as_str() {
            "admin" => Ok(GroupRole::Admin),
            "member" => Ok(GroupRole::Member),
            custom if custom.trim().is_empty() => Ok(GroupRole::Member),
            custom => Ok(GroupRole::Custom(custom.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupMember {
    pub pubkey: PublicKey,
    pub roles: HashSet<GroupRole>,
}

impl GroupMember {
    pub fn new(pubkey: PublicKey, roles: HashSet<GroupRole>) -> Self {
        Self { pubkey, roles }
    }

    pub fn is(&self, role: GroupRole) -> bool {
        self.roles.contains(&role)
    }

    pub fn new_admin(pubkey: PublicKey) -> Self {
        Self {
            pubkey,
            roles: HashSet::from([GroupRole::Admin]),
        }
    }

    pub fn new_member(pubkey: PublicKey) -> Self {
        Self {
            pubkey,
            roles: HashSet::from([GroupRole::Member]),
        }
    }
}

impl TryFrom<&Tag> for GroupMember {
    type Error = Error;

    fn try_from(tag: &Tag) -> Result<Self, Error> {
        if tag.kind() != TagKind::p() {
            return Err(Error::notice("Invalid tag kind"));
        }

        let [_, pubkey, roles @ ..] = tag.as_slice() else {
            return Err(Error::notice("Invalid tag format"));
        };

        let pubkey = PublicKey::parse(pubkey).map_err(|_| Error::notice("Invalid pubkey"))?;

        if roles.is_empty() {
            return Ok(Self {
                pubkey,
                roles: HashSet::from([GroupRole::Member]),
            });
        }

        Ok(Self {
            pubkey,
            roles: roles
                .iter()
                .map(|role| GroupRole::from_str(role))
                .collect::<Result<_, _>>()?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    pub event_id: EventId,
    pub roles: HashSet<GroupRole>,
}

impl Invite {
    pub fn new(event_id: EventId, roles: HashSet<GroupRole>) -> Self {
        Self { event_id, roles }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub metadata: GroupMetadata,
    pub members: HashMap<PublicKey, GroupMember>,
    pub join_requests: HashSet<PublicKey>,
    pub invites: HashMap<String, Invite>,
    pub roles: HashSet<GroupRole>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl Default for Group {
    fn default() -> Self {
        Self {
            id: "".to_string(),
            metadata: GroupMetadata::new("".to_string()),
            members: HashMap::new(),
            join_requests: HashSet::new(),
            invites: HashMap::new(),
            roles: HashSet::new(),
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }
}

impl From<&Event> for Group {
    fn from(event: &Event) -> Self {
        let Some(group_id) = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::h() || t.kind() == TagKind::d())
            .and_then(|t| t.content())
        else {
            return Self::default();
        };

        Self {
            id: group_id.to_string(),
            metadata: GroupMetadata::new(group_id.to_string()),
            created_at: event.created_at,
            updated_at: event.created_at,
            ..Default::default()
        }
    }
}

impl std::fmt::Debug for Group {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{{")?;
        writeln!(f, "  id: \"{}\",", self.id)?;
        writeln!(f, "  metadata: {{")?;
        writeln!(f, "    name: \"{}\",", self.metadata.name)?;
        if let Some(about) = &self.metadata.about {
            writeln!(f, "    about: \"{}\",", about)?;
        }
        if let Some(picture) = &self.metadata.picture {
            writeln!(f, "    picture: \"{}\",", picture)?;
        }
        writeln!(f, "    private: {},", self.metadata.private)?;
        writeln!(f, "    closed: {}", self.metadata.closed)?;
        writeln!(f, "  }},")?;
        writeln!(f, "  members: {{")?;
        for (pubkey, member) in &self.members {
            writeln!(
                f,
                "    {}: {{ roles: [{}] }},",
                pubkey,
                member
                    .roles
                    .iter()
                    .map(|r| format!("\"{}\"", r))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        writeln!(f, "  }},")?;
        writeln!(
            f,
            "  join_requests: [{}],",
            self.join_requests
                .iter()
                .map(|pk| format!("\"{}\"", pk))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
        writeln!(f, "  invites: {{")?;
        for (code, invite) in &self.invites {
            write!(f, "    {}: {{ ", code)?;
            writeln!(
                f,
                "roles: [{}] }},",
                invite
                    .roles
                    .iter()
                    .map(|r| format!("\"{}\"", r))
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        writeln!(f, "  }},")?;
        writeln!(
            f,
            "  roles: [{}],",
            self.roles
                .iter()
                .map(|r| format!("\"{}\"", r))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
        writeln!(f, "  created_at: {},", self.created_at.as_u64())?;
        writeln!(f, "  updated_at: {}", self.updated_at.as_u64())?;
        write!(f, "}}")
    }
}

impl Group {
    fn update_state(&mut self) {
        self.updated_at = Timestamp::now();
        info!("Group state: {:?}", self);
    }

    pub fn new_with_id(id: String) -> Self {
        Self {
            id: id.clone(),
            metadata: GroupMetadata::new(id),
            members: HashMap::new(),
            join_requests: HashSet::new(),
            invites: HashMap::new(),
            roles: HashSet::new(),
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    pub fn new(event: &Event) -> Result<Self, Error> {
        let Some(group_id) = Self::extract_group_id(event) else {
            return Err(Error::notice("Group ID not found"));
        };

        let mut group = Self {
            id: group_id.to_string(),
            metadata: GroupMetadata::new(group_id.to_string()),
            created_at: event.created_at,
            updated_at: event.created_at,
            ..Default::default()
        };

        // Add the creator as an admin
        group.members.insert(
            event.pubkey,
            GroupMember {
                pubkey: event.pubkey,
                roles: HashSet::from([GroupRole::Admin]),
            },
        );

        Ok(group)
    }

    pub fn delete_group_request(
        &self,
        delete_group_request_event: &Event,
        relay_pubkey: &PublicKey,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        if delete_group_request_event.kind != KIND_GROUP_DELETE_9008 {
            return Err(Error::notice("Invalid event kind for delete group"));
        }

        if !self.can_delete_group(authed_pubkey, relay_pubkey, delete_group_request_event)? {
            return Err(Error::notice("User is not authorized to delete this group"));
        }

        // Delete all group kinds possible except this delete request (kind 9008)
        let non_addressable_filter = Filter::new().custom_tag(
            SingleLetterTag::lowercase(Alphabet::H),
            &[self.id.to_string()],
        );

        let addressable_filter = Filter::new().custom_tag(
            SingleLetterTag::lowercase(Alphabet::D),
            &[self.id.to_string()],
        );

        Ok(vec![
            StoreCommand::DeleteEvents(non_addressable_filter),
            StoreCommand::DeleteEvents(addressable_filter),
            StoreCommand::SaveSignedEvent(delete_group_request_event.clone()),
        ])
    }

    pub fn delete_event_request(
        &mut self,
        delete_request_event: &Event,
        relay_pubkey: &PublicKey,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        if delete_request_event.kind != KIND_GROUP_DELETE_EVENT_9005 {
            return Err(Error::notice("Invalid event kind for delete event"));
        }

        if !self.can_delete_event(authed_pubkey, relay_pubkey, delete_request_event)? {
            return Err(Error::notice("User is not authorized to delete this event"));
        }

        let event_ids: Vec<_> = delete_request_event.tags.event_ids().copied().collect();

        // We may be deleting invites, remove them from memory too.
        let codes_to_remove: Vec<_> = self
            .invites
            .iter()
            .filter_map(|(code, invite)| {
                if event_ids.contains(&invite.event_id) {
                    Some(code.clone())
                } else {
                    None
                }
            })
            .collect();

        for code in codes_to_remove {
            self.invites.remove(&code);
        }

        let filter = Filter::new().ids(event_ids);

        Ok(vec![
            StoreCommand::DeleteEvents(filter),
            StoreCommand::SaveSignedEvent(delete_request_event.clone()),
        ])
    }

    pub fn add_members_from_event(
        &mut self,
        members_event: &Event,
        relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        if !self.can_edit_members(&members_event.pubkey, relay_pubkey) {
            error!(
                "User {} is not authorized to add users to this group",
                members_event.pubkey
            );

            return Err(Error::notice(
                "User is not authorized to add users to this group",
            ));
        }

        let group_members = members_event
            .tags
            .filter(TagKind::p())
            .map(GroupMember::try_from)
            .filter_map(Result::ok);

        self.add_members(group_members);
        Ok(true)
    }

    pub fn add_members(&mut self, group_members: impl Iterator<Item = GroupMember>) {
        for member in group_members {
            self.join_requests.remove(&member.pubkey);
            self.members.insert(member.pubkey, member);
        }

        self.update_roles();
        self.update_state();
    }

    pub fn add_pubkey(&mut self, pubkey: PublicKey) {
        let member = GroupMember::new_member(pubkey);
        self.add_members(vec![member].into_iter());
    }

    pub fn admin_pubkeys(&self) -> Vec<PublicKey> {
        self.members
            .values()
            .filter(|member| member.is(GroupRole::Admin))
            .map(|member| member.pubkey)
            .collect::<Vec<_>>()
    }

    pub fn remove_members(
        &mut self,
        members_event: &Event,
        relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        if !self.can_edit_members(&members_event.pubkey, relay_pubkey) {
            error!(
                "User {} is not authorized to remove users from this group",
                members_event.pubkey
            );
            return Err(Error::notice(
                "User is not authorized to remove users from this group",
            ));
        }

        let admins = self.admin_pubkeys();

        for tag in members_event.tags.filter(TagKind::p()) {
            let Some(removed_pubkey) = tag.content().and_then(|s| PublicKey::parse(s).ok()) else {
                return Err(Error::notice("Invalid tag format"));
            };

            if admins.len() == 1 && admins.contains(&removed_pubkey) {
                return Err(Error::notice("Cannot remove last admin"));
            }

            self.members.remove(&removed_pubkey);
        }

        self.update_roles();
        self.update_state();
        Ok(true)
    }

    pub fn set_metadata(&mut self, event: &Event, relay_pubkey: &PublicKey) -> Result<(), Error> {
        if event.kind != KIND_GROUP_METADATA_39000
            && event.kind != KIND_GROUP_CREATE_9007
            && event.kind != KIND_GROUP_EDIT_METADATA_9002
        {
            return Err(Error::notice(format!(
                "Invalid event kind for group metadata {}",
                event.kind
            )));
        }

        if !self.can_edit_metadata(&event.pubkey, relay_pubkey) {
            return Err(Error::notice("User is not authorized to edit metadata"));
        }

        if event.tags.find(TagKind::custom("private")).is_some() {
            self.metadata.private = true;
        } else if event.tags.find(TagKind::custom("public")).is_some() {
            self.metadata.private = false;
        }

        if event.tags.find(TagKind::custom("closed")).is_some() {
            self.metadata.closed = true;
        } else if event.tags.find(TagKind::custom("open")).is_some() {
            self.metadata.closed = false;
        }

        if let Some(name_tag) = event.tags.find(TagKind::Name).and_then(|t| t.content()) {
            self.metadata.name = name_tag.to_string();
        }

        if let Some(about_tag) = event
            .tags
            .find(TagKind::custom("about"))
            .and_then(|t| t.content())
        {
            self.metadata.about = Some(about_tag.to_string());
        }

        if let Some(picture_tag) = event
            .tags
            .find(TagKind::custom("picture"))
            .and_then(|t| t.content())
        {
            self.metadata.picture = Some(picture_tag.to_string());
        }

        self.update_state();
        Ok(())
    }

    /// Changes the roles of one or more group members.
    ///
    /// This method enforces several important constraints to maintain group integrity:
    /// 1. Only admins or the relay can change roles
    /// 2. The last admin's role cannot be changed to non-admin
    /// 3. The target users must already be members of the group
    ///
    /// # Arguments
    /// * `event` - The event containing role changes. Must have p-tags with pubkey and role.
    /// * `relay_pubkey` - The relay's public key, which has special permissions.
    ///
    /// # Returns
    /// * `Ok(())` if the roles were successfully updated
    /// * `Err` if:
    ///   - The user is not authorized to change roles
    ///   - Attempting to remove the last admin
    ///   - Invalid tag format
    pub fn set_roles(&mut self, event: &Event, relay_pubkey: &PublicKey) -> Result<(), Error> {
        if !self.can_edit_members(&event.pubkey, relay_pubkey) {
            return Err(Error::notice("User is not authorized to set roles"));
        }

        // First check if this would remove the last admin
        let current_admins = self.admin_pubkeys();
        for tag in event.tags.filter(TagKind::p()) {
            let member = GroupMember::try_from(tag)?;
            if current_admins.len() == 1
                && current_admins.contains(&member.pubkey)
                && !member.roles.contains(&GroupRole::Admin)
            {
                return Err(Error::notice("Cannot remove last admin"));
            }
        }

        // If we get here, it's safe to update the roles
        for tag in event.tags.filter(TagKind::p()) {
            let member = GroupMember::try_from(tag)?;
            if let Some(existing_member) = self.members.get_mut(&member.pubkey) {
                existing_member.roles = member.roles;
            }
        }

        self.update_roles();
        self.update_state();
        Ok(())
    }

    pub fn join_request(&mut self, event: &Event) -> Result<bool, Error> {
        if event.kind != KIND_GROUP_USER_JOIN_REQUEST_9021 {
            return Err(Error::notice(format!(
                "Invalid event kind for join request {}",
                event.kind
            )));
        }

        if !self.metadata.closed {
            info!("Public group, adding member {}", event.pubkey);
            self.members
                .entry(event.pubkey)
                .or_insert(GroupMember::new_member(event.pubkey));
            self.join_requests.remove(&event.pubkey);
            self.update_state();
            return Ok(true);
        }

        let code = event
            .tags
            .find(TagKind::custom("code"))
            .and_then(|t| t.content())
            .and_then(|code| self.invites.get_mut(code));

        let Some(invite) = code else {
            info!("Invite not found, adding join request for {}", event.pubkey);
            self.join_requests.insert(event.pubkey);
            self.update_state();
            return Ok(false);
        };

        // Invite code matched
        info!("Invite code matched, adding member {}", event.pubkey);
        let roles = invite.roles.clone();

        self.members
            .insert(event.pubkey, GroupMember::new(event.pubkey, roles));

        self.join_requests.remove(&event.pubkey);
        self.update_state();
        Ok(true)
    }

    pub fn create_invite(
        &mut self,
        invite_event: &Event,
        relay_pubkey: &PublicKey,
    ) -> Result<bool, Error> {
        if invite_event.kind != KIND_GROUP_CREATE_INVITE_9009 {
            return Err(Error::notice(format!(
                "Invalid event kind for create invite {}",
                invite_event.kind
            )));
        }

        if !self.can_create_invites(&invite_event.pubkey, relay_pubkey) {
            return Err(Error::notice("User is not authorized to create invites"));
        }

        let invite_code = invite_event
            .tags
            .find(TagKind::custom("code"))
            .and_then(|t| t.content())
            .ok_or(Error::notice("Invite code not found"))?;

        // Check for duplicate invite code
        if self.invites.contains_key(invite_code) {
            return Err(Error::notice("Invite code already exists"));
        }

        let invite = Invite::new(invite_event.id, HashSet::from([GroupRole::Member]));

        self.invites.insert(invite_code.to_string(), invite);
        self.update_state();
        Ok(true)
    }

    pub fn leave_request(&mut self, event: &Event) -> Result<bool, Error> {
        if event.kind != KIND_GROUP_USER_LEAVE_REQUEST_9022 {
            return Err(Error::notice(format!(
                "Invalid event kind for leave request {}",
                event.kind
            )));
        }

        self.join_requests.remove(&event.pubkey);
        let removed = self.members.remove(&event.pubkey).is_some();
        self.update_state();
        Ok(removed)
    }

    pub fn is_admin(&self, pubkey: &PublicKey) -> bool {
        let member = self.members.get(pubkey);
        if let Some(member) = member {
            member.is(GroupRole::Admin)
        } else {
            false
        }
    }

    pub fn is_member(&self, pubkey: &PublicKey) -> bool {
        self.members.contains_key(pubkey)
    }

    // State loading methods - used during startup to rebuild state from stored events
    pub fn load_metadata_from_event(&mut self, event: &Event) -> Result<(), Error> {
        let name = event.tags.find(TagKind::Name).and_then(|t| t.content());
        let about = event
            .tags
            .find(TagKind::custom("about"))
            .and_then(|t| t.content());
        let picture = event
            .tags
            .find(TagKind::custom("picture"))
            .and_then(|t| t.content());
        let private = event.tags.find(TagKind::custom("private")).is_some();
        let closed = event.tags.find(TagKind::custom("closed")).is_some();

        self.metadata = GroupMetadata {
            name: name.unwrap_or(&self.id).to_string(),
            about: about.map(|s| s.to_string()),
            picture: picture.map(|s| s.to_string()),
            private,
            closed,
        };

        self.updated_at = event.created_at;
        Ok(())
    }

    pub fn load_members_from_event(&mut self, event: &Event) -> Result<(), Error> {
        let pubkey_and_roles = event
            .tags
            .iter()
            .filter(|t| t.kind() == TagKind::p())
            .filter_map(|t| {
                let [_, pubkey, roles @ ..] = t.as_slice() else {
                    return None;
                };

                let pubkey = PublicKey::parse(pubkey).ok()?;
                Some((pubkey, roles))
            })
            .collect::<Vec<_>>();

        for (pubkey, roles) in pubkey_and_roles {
            let mut roles = roles.to_vec();
            if roles.is_empty() {
                roles.push(GroupRole::Member.to_string());
            }

            let roles = roles
                .iter()
                .map(|r| GroupRole::from_str(r).unwrap_or(GroupRole::Member))
                .collect::<HashSet<_>>();

            self.members.insert(pubkey, GroupMember::new(pubkey, roles));
        }

        self.update_roles();
        self.updated_at = event.created_at;
        Ok(())
    }

    pub fn load_join_request_from_event(&mut self, event: &Event) -> Result<(), Error> {
        if !self.members.contains_key(&event.pubkey) {
            self.join_requests.insert(event.pubkey);
            self.updated_at = event.created_at;
        }
        Ok(())
    }

    pub fn load_invite_from_event(&mut self, event: &Event) -> Result<(), Error> {
        if let Some(code) = event
            .tags
            .find(TagKind::custom("code"))
            .and_then(|t| t.content())
        {
            let roles = event
                .tags
                .iter()
                .filter(|t| t.kind() == TagKind::custom("role"))
                .filter_map(|t| t.content())
                .map(|r| GroupRole::from_str(r).unwrap_or(GroupRole::Member))
                .collect();

            let invite = Invite::new(event.id, roles);

            self.invites.insert(code.to_string(), invite);
            self.updated_at = event.created_at;
        }
        Ok(())
    }

    // Helper methods
    pub fn update_roles(&mut self) {
        let unique_roles = self
            .members
            .values()
            .flat_map(|m| m.roles.iter().cloned())
            .collect::<HashSet<_>>();

        self.roles = unique_roles;
    }

    pub fn extract_group_id(event: &Event) -> Option<&str> {
        match event.kind {
            Kind::ParameterizedReplaceable(_) => {
                event.tags.find(TagKind::d()).and_then(|t| t.content())
            }
            _ => event.tags.find(TagKind::h()).and_then(|t| t.content()),
        }
    }

    pub fn extract_group_h_tag(event: &Event) -> Option<&str> {
        event.tags.find(TagKind::h()).and_then(|t| t.content())
    }

    pub fn verify_member_access(&self, pubkey: &PublicKey, event_kind: Kind) -> Result<(), Error> {
        if event_kind != KIND_GROUP_USER_JOIN_REQUEST_9021
            && self.metadata.closed
            && !self.is_member(pubkey)
        {
            return Err(Error::restricted(format!(
                "User {} is not a member of this group",
                pubkey
            )));
        }
        Ok(())
    }
}

// Event generation based on current state
impl Group {
    pub fn generate_put_user_event(&self, pubkey: &PublicKey) -> EventBuilder {
        EventBuilder::new(KIND_GROUP_ADD_USER_9000, "")
            .tag(Tag::custom(
                TagKind::p(),
                [
                    pubkey.to_string(),
                    GroupRole::Member.as_tuple().0.to_string(),
                ],
            ))
            .tag(Tag::custom(TagKind::h(), [self.id.clone()]))
    }

    pub fn generate_metadata_event(&self) -> EventBuilder {
        // Private = needs authentication to read
        let access = if self.metadata.private {
            "private"
        } else {
            "public"
        };

        // Open = automatic creation of 9000 events when a 9021 comes
        let visibility = if self.metadata.closed {
            "closed"
        } else {
            "open"
        };

        let mut metadata_event = EventBuilder::new(KIND_GROUP_METADATA_39000, "")
            .tag(Tag::identifier(self.id.clone()))
            .tag(Tag::custom(TagKind::Name, [self.metadata.name.clone()]))
            .tag(Tag::custom(TagKind::custom(access), &[] as &[String]))
            .tag(Tag::custom(TagKind::custom(visibility), &[] as &[String]));

        if let Some(about) = &self.metadata.about {
            metadata_event =
                metadata_event.tag(Tag::custom(TagKind::custom("about"), [about.clone()]));
        }

        if let Some(picture) = &self.metadata.picture {
            metadata_event =
                metadata_event.tag(Tag::custom(TagKind::custom("picture"), [picture.clone()]));
        }

        metadata_event
    }

    pub fn generate_admins_event(&self) -> EventBuilder {
        let admins = self.members.values().filter(|member| {
            member
                .roles
                .iter()
                .any(|role| matches!(role, GroupRole::Admin))
        });

        let mut tags = Vec::new();
        tags.push(Tag::identifier(self.id.clone()));

        for admin in admins {
            let mut tag_vals: Vec<String> = vec![admin.pubkey.to_string()];
            tag_vals.extend(admin.roles.iter().map(|role| format!("{:?}", role)));

            let tag = Tag::custom(TagKind::p(), tag_vals);
            tags.push(tag);
        }

        EventBuilder::new(KIND_GROUP_ADMINS_39001, "").tags(tags)
    }

    pub fn generate_members_event(&self) -> EventBuilder {
        let members: Vec<&PublicKey> = self.members.keys().collect();

        let mut tags = Vec::new();
        tags.push(Tag::identifier(self.id.clone()));

        for pubkey in members {
            tags.push(Tag::public_key(*pubkey));
        }

        EventBuilder::new(KIND_GROUP_MEMBERS_39002, "").tags(tags)
    }

    pub fn generate_roles_event(&self) -> EventBuilder {
        let supported_roles: Vec<(String, String)> = GroupRole::iter()
            .map(|role| {
                let (name, description) = role.as_tuple();
                (name.to_string(), description.to_string())
            })
            .collect();

        let mut tags = Vec::new();
        tags.push(Tag::identifier(self.id.clone()));

        for (role_name, role_description) in supported_roles {
            tags.push(Tag::custom(
                TagKind::custom("role"),
                vec![role_name, role_description],
            ));
        }

        let content = "List of roles supported by this group".to_string();

        EventBuilder::new(KIND_GROUP_ROLES_39003, &content).tags(tags)
    }
}

// Authorization checks
impl Group {
    pub fn can_edit_members(&self, pubkey: &PublicKey, relay_pubkey: &PublicKey) -> bool {
        if self.is_admin(pubkey) {
            return true;
        }

        // Relay pubkey can see all events
        if relay_pubkey == pubkey {
            debug!("Relay pubkey {} can edit members", relay_pubkey);
            return true;
        }

        false
    }

    pub fn can_edit_metadata(&self, pubkey: &PublicKey, relay_pubkey: &PublicKey) -> bool {
        if self.is_admin(pubkey) {
            return true;
        }

        // Relay pubkey can see all events
        if relay_pubkey == pubkey {
            debug!("Relay pubkey {} can edit metadata", relay_pubkey);
            return true;
        }

        false
    }

    pub fn can_create_invites(&self, pubkey: &PublicKey, relay_pubkey: &PublicKey) -> bool {
        if self.is_admin(pubkey) {
            return true;
        }

        // Relay pubkey can see all events
        if relay_pubkey == pubkey {
            debug!("Relay pubkey {} can create invites", relay_pubkey);
            return true;
        }

        false
    }

    pub fn can_delete_group(
        &self,
        authed_pubkey: &Option<PublicKey>,
        relay_pubkey: &PublicKey,
        delete_group_event: &Event,
    ) -> Result<bool, Error> {
        self.can_delete_event(authed_pubkey, relay_pubkey, delete_group_event)
    }

    pub fn can_delete_event(
        &self,
        authed_pubkey: &Option<PublicKey>,
        relay_pubkey: &PublicKey,
        event: &Event,
    ) -> Result<bool, Error> {
        let Some(authed_pubkey) = authed_pubkey else {
            warn!(
                "User is not authenticated, cannot delete event {}, kind {}",
                event.id, event.kind
            );
            return Err(Error::auth_required("User is not authenticated"));
        };

        // Relay pubkey can delete all events
        if relay_pubkey == authed_pubkey {
            debug!(
                "Relay pubkey {} can delete event {}, kind {}",
                relay_pubkey, event.id, event.kind
            );
            return Ok(true);
        }

        if self.is_admin(&event.pubkey) {
            return Ok(true);
        }

        Ok(false)
    }

    pub fn can_see_event(
        &self,
        authed_pubkey: &Option<PublicKey>,
        relay_pubkey: &PublicKey,
        event: &Event,
    ) -> Result<bool, Error> {
        // Public groups are always visible
        if !self.metadata.private {
            debug!(
                "Public group, can see event {}, kind {}",
                event.id, event.kind
            );
            return Ok(true);
        }

        // Private groups need authentication
        let Some(authed_pubkey) = authed_pubkey else {
            warn!(
                "User is not authenticated, cannot see event {}, kind {}",
                event.id, event.kind
            );
            return Err(Error::auth_required("User is not authenticated"));
        };

        // Relay pubkey can see all events
        if relay_pubkey == authed_pubkey {
            debug!(
                "Relay pubkey {} can see event {}, kind {}",
                relay_pubkey, event.id, event.kind
            );
            return Ok(true);
        }

        // You can see your own events
        if *authed_pubkey == event.pubkey {
            debug!(
                "User {} can see their own event {}, kind {}",
                authed_pubkey, event.id, event.kind
            );
            return Ok(true);
        }

        // Admins can see everything
        if self.is_admin(authed_pubkey) {
            debug!(
                "User {} is an admin, can see event {}, kind {}",
                authed_pubkey, event.id, event.kind
            );
            return Ok(true);
        }

        // Members can see everything except invites
        if self.is_member(authed_pubkey) && event.kind != KIND_GROUP_CREATE_INVITE_9009 {
            debug!(
                "User {} is a member, can see event {}, kind {}",
                authed_pubkey, event.id, event.kind
            );
            return Ok(true);
        }

        warn!(
            "User {} is not a member, cannot see event {}, kind {}",
            authed_pubkey, event.id, event.kind
        );
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_keys() -> (Keys, Keys, Keys) {
        let admin_keys = Keys::generate();
        let member_keys = Keys::generate();
        let non_member_keys = Keys::generate();
        (admin_keys, member_keys, non_member_keys)
    }

    async fn create_test_event(keys: &Keys, kind: Kind, tags: Vec<Tag>) -> Event {
        let mut builder = EventBuilder::new(kind, "");
        for tag in tags {
            builder = builder.tag(tag);
        }

        builder
            .custom_created_at(Timestamp::now())
            .sign(keys)
            .await
            .unwrap()
    }

    async fn create_test_group(admin_keys: &Keys) -> (Group, String) {
        let group_id = "test_group_123".to_string();
        let tags = vec![Tag::custom(TagKind::h(), [group_id.clone()])];
        let event = create_test_event(admin_keys, KIND_GROUP_CREATE_9007, tags).await;

        let group = Group::new(&event).unwrap();
        (group, group_id)
    }

    #[tokio::test]
    async fn test_group_creation() {
        let (admin_keys, _, _) = create_test_keys();
        let (group, group_id) = create_test_group(&admin_keys).await;

        assert_eq!(group.id, group_id);
        assert_eq!(group.metadata.name, group_id);
        assert!(group.is_admin(&admin_keys.public_key()));
        assert_eq!(group.members.len(), 1);
    }

    #[tokio::test]
    async fn test_add_members() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let tags = vec![Tag::public_key(member_keys.public_key())];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, tags).await;

        assert!(group
            .add_members_from_event(&add_event, &admin_keys.public_key())
            .is_ok());
        assert!(group.is_member(&member_keys.public_key()));
        assert!(!group.is_admin(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_remove_members() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // First add a member
        let add_tags = vec![Tag::public_key(member_keys.public_key())];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        group
            .add_members_from_event(&add_event, &admin_keys.public_key())
            .unwrap();

        // Then remove them
        let remove_tags = vec![Tag::public_key(member_keys.public_key())];
        let remove_event =
            create_test_event(&admin_keys, KIND_GROUP_REMOVE_USER_9001, remove_tags).await;

        assert!(group
            .remove_members(&remove_event, &admin_keys.public_key())
            .is_ok());
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_name() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let tags = vec![Tag::custom(TagKind::Name, ["New Group Name"])];
        let metadata_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.name, "New Group Name");
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_about() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let tags = vec![Tag::custom(TagKind::custom("about"), ["About text"])];
        let metadata_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.about, Some("About text".to_string()));
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_picture() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let tags = vec![Tag::custom(TagKind::custom("picture"), ["picture_url"])];
        let metadata_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_visibility() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Test setting to public
        let public_tags = vec![Tag::custom(TagKind::custom("public"), &[] as &[String])];
        let public_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, public_tags).await;

        assert!(group
            .set_metadata(&public_event, &admin_keys.public_key())
            .is_ok());
        assert!(!group.metadata.private);

        // Test setting back to private
        let private_tags = vec![Tag::custom(TagKind::custom("private"), &[] as &[String])];
        let private_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, private_tags).await;

        assert!(group
            .set_metadata(&private_event, &admin_keys.public_key())
            .is_ok());
        assert!(group.metadata.private);
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_multiple_fields() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let tags = vec![
            Tag::custom(TagKind::Name, ["New Group Name"]),
            Tag::custom(TagKind::custom("about"), ["About text"]),
            Tag::custom(TagKind::custom("picture"), ["picture_url"]),
            Tag::custom(TagKind::custom("public"), &[] as &[String]),
        ];
        let metadata_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, tags).await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.name, "New Group Name");
        assert_eq!(group.metadata.about, Some("About text".to_string()));
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
        assert!(!group.metadata.private);
    }

    #[tokio::test]
    async fn test_invite_system_admin_can_create_invite() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let invite_code = "test_invite_123";
        let create_tags = vec![Tag::custom(TagKind::custom("code"), [invite_code])];
        let create_invite_event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, create_tags).await;

        assert!(group
            .create_invite(&create_invite_event, &admin_keys.public_key())
            .unwrap());
        assert!(group.invites.contains_key(invite_code));
    }

    #[tokio::test]
    async fn test_invite_system_user_can_join_with_valid_invite() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Create invite
        let invite_code = "test_invite_123";
        let create_tags = vec![Tag::custom(TagKind::custom("code"), [invite_code])];
        let create_invite_event =
            create_test_event(&admin_keys, KIND_GROUP_CREATE_INVITE_9009, create_tags).await;
        group
            .create_invite(&create_invite_event, &admin_keys.public_key())
            .unwrap();

        // Use invite
        let join_tags = vec![Tag::custom(TagKind::custom("code"), [invite_code])];
        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, join_tags).await;

        assert!(group.join_request(&join_event).unwrap());
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_join_request_adds_to_join_requests() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let join_event =
            create_test_event(&member_keys, KIND_GROUP_USER_JOIN_REQUEST_9021, vec![]).await;

        assert!(!group.join_request(&join_event).unwrap());
        assert!(group.join_requests.contains(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_leave_request_removes_member() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Add member manually
        let add_tags = vec![Tag::public_key(member_keys.public_key())];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        group
            .add_members_from_event(&add_event, &admin_keys.public_key())
            .unwrap();

        // Test leave request
        let leave_event =
            create_test_event(&member_keys, KIND_GROUP_USER_LEAVE_REQUEST_9022, vec![]).await;

        assert!(group.leave_request(&leave_event).unwrap());
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_event_visibility_admin_can_see_events() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (group, _) = create_test_group(&admin_keys).await;

        let test_event = create_test_event(&member_keys, Kind::Custom(9), vec![]).await;

        assert!(group
            .can_see_event(
                &Some(admin_keys.public_key()),
                &admin_keys.public_key(),
                &test_event
            )
            .unwrap());
    }

    #[tokio::test]
    async fn test_event_visibility_member_can_see_events() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Add a member
        let add_tags = vec![Tag::public_key(member_keys.public_key())];
        let add_event = create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        group
            .add_members_from_event(&add_event, &admin_keys.public_key())
            .unwrap();

        let test_event = create_test_event(&member_keys, Kind::Custom(9), vec![]).await;

        assert!(group
            .can_see_event(
                &Some(member_keys.public_key()),
                &admin_keys.public_key(),
                &test_event
            )
            .unwrap());
    }

    #[tokio::test]
    async fn test_event_visibility_non_member_cannot_see_events() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys();
        let (group, _) = create_test_group(&admin_keys).await;

        let test_event = create_test_event(&member_keys, Kind::Custom(9), vec![]).await;

        assert!(!group
            .can_see_event(
                &Some(non_member_keys.public_key()),
                &admin_keys.public_key(),
                &test_event
            )
            .unwrap());
    }

    #[tokio::test]
    async fn test_event_visibility_unauthenticated_user_cannot_see_events() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (group, _) = create_test_group(&admin_keys).await;

        let test_event = create_test_event(&member_keys, Kind::Custom(9), vec![]).await;

        assert!(group
            .can_see_event(&None, &admin_keys.public_key(), &test_event)
            .is_err());
    }

    #[tokio::test]
    async fn test_event_visibility_public_group_events_visible_to_all() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Make group public
        let public_tags = vec![Tag::custom(TagKind::custom("public"), &[] as &[String])];
        let public_event =
            create_test_event(&admin_keys, KIND_GROUP_EDIT_METADATA_9002, public_tags).await;
        group
            .set_metadata(&public_event, &admin_keys.public_key())
            .unwrap();

        let test_event = create_test_event(&member_keys, Kind::Custom(9), vec![]).await;

        assert!(group
            .can_see_event(&None, &admin_keys.public_key(), &test_event)
            .unwrap());
        assert!(group
            .can_see_event(
                &Some(non_member_keys.public_key()),
                &admin_keys.public_key(),
                &test_event
            )
            .unwrap());
    }

    #[tokio::test]
    async fn test_role_management_can_add_member_with_admin_role() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        let add_tags = vec![Tag::custom(
            TagKind::p(),
            [member_keys.public_key().to_string(), "Admin".to_string()],
        )];
        let add_admin_event =
            create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;

        group
            .add_members_from_event(&add_admin_event, &admin_keys.public_key())
            .unwrap();
        assert!(group.is_admin(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_role_management_admin_can_edit_metadata() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Add member as admin
        let add_tags = vec![Tag::custom(
            TagKind::p(),
            [member_keys.public_key().to_string(), "Admin".to_string()],
        )];
        let add_admin_event =
            create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_tags).await;
        group
            .add_members_from_event(&add_admin_event, &admin_keys.public_key())
            .unwrap();

        // Test admin permissions
        let metadata_tags = vec![Tag::custom(TagKind::Name, ["New Name"])];
        let metadata_event =
            create_test_event(&member_keys, KIND_GROUP_EDIT_METADATA_9002, metadata_tags).await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
    }

    #[tokio::test]
    async fn test_delete_event_request_non_member_cannot_delete_events() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a test event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            Kind::Custom(9),
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        let delete_request = create_test_event(
            &non_member_keys,
            KIND_GROUP_DELETE_EVENT_9005,
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result = group.delete_event_request(
            &delete_request,
            &relay_pubkey,
            &Some(non_member_keys.public_key()),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_event_request_member_cannot_delete_events() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a test event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            Kind::Custom(9),
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        let delete_request = create_test_event(
            &member_keys,
            KIND_GROUP_DELETE_EVENT_9005,
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result = group.delete_event_request(
            &delete_request,
            &relay_pubkey,
            &Some(member_keys.public_key()),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_event_request_admin_can_delete_events() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a test event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            Kind::Custom(9),
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        let delete_request = create_test_event(
            &admin_keys,
            KIND_GROUP_DELETE_EVENT_9005,
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result = group.delete_event_request(
            &delete_request,
            &relay_pubkey,
            &Some(admin_keys.public_key()),
        );
        assert!(result.is_ok());
        if let Ok(commands) = result {
            assert_eq!(commands.len(), 2);
            assert_eq!(
                commands[0],
                StoreCommand::DeleteEvents(Filter::new().ids([event_to_delete.id]))
            );
            assert_eq!(
                commands[1],
                StoreCommand::SaveSignedEvent(delete_request.clone())
            );
        }
    }

    #[tokio::test]
    async fn test_delete_event_request_relay_can_delete_events() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a test event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            Kind::Custom(9),
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        let delete_request = create_test_event(
            &non_member_keys,
            KIND_GROUP_DELETE_EVENT_9005,
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result =
            group.delete_event_request(&delete_request, &relay_pubkey, &Some(relay_pubkey));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_event_request_wrong_event_kind_is_rejected() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a test event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            Kind::Custom(9),
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        let delete_request = create_test_event(
            &admin_keys,
            Kind::Custom(9),
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result = group.delete_event_request(
            &delete_request,
            &relay_pubkey,
            &Some(admin_keys.public_key()),
        );
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_event_request_unauthenticated_request_is_rejected() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a test event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            Kind::Custom(9),
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        let delete_request = create_test_event(
            &admin_keys,
            KIND_GROUP_DELETE_EVENT_9005,
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result = group.delete_event_request(&delete_request, &relay_pubkey, &None);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_event_request_deleting_invite_removes_it_from_invites_map() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create an invite
        let invite_code = "test_invite_123";
        let create_invite_event = create_test_event(
            &admin_keys,
            KIND_GROUP_CREATE_INVITE_9009,
            vec![Tag::custom(TagKind::custom("code"), [invite_code])],
        )
        .await;

        // Add the invite to the group
        group
            .create_invite(&create_invite_event, &relay_pubkey)
            .unwrap();
        assert!(group.invites.contains_key(invite_code));

        // Delete the invite event
        let delete_invite_request = create_test_event(
            &admin_keys,
            KIND_GROUP_DELETE_EVENT_9005,
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(create_invite_event.id),
            ],
        )
        .await;

        let result = group.delete_event_request(
            &delete_invite_request,
            &relay_pubkey,
            &Some(admin_keys.public_key()),
        );
        assert!(result.is_ok());
        assert!(
            !group.invites.contains_key(invite_code),
            "Invite should be removed from the invites map after deletion"
        );
    }

    #[tokio::test]
    async fn test_remove_members_cannot_remove_last_admin() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Attempt to remove the last admin
        let remove_tags = vec![Tag::public_key(admin_keys.public_key())];
        let remove_event =
            create_test_event(&admin_keys, KIND_GROUP_REMOVE_USER_9001, remove_tags).await;

        // Should fail with "Cannot remove last admin" error
        let result = group.remove_members(&remove_event, &admin_keys.public_key());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Cannot remove last admin");
    }

    #[tokio::test]
    async fn test_group_creation_always_has_admin() {
        let (admin_keys, _, _) = create_test_keys();
        let (group, _) = create_test_group(&admin_keys).await;

        // Verify there is exactly one admin
        let admins: Vec<_> = group
            .members
            .values()
            .filter(|member| member.is(GroupRole::Admin))
            .collect();
        assert_eq!(admins.len(), 1, "A new group should have exactly one admin");
        assert_eq!(
            admins[0].pubkey,
            admin_keys.public_key(),
            "The group creator should be the admin"
        );

        // Verify the group cannot be created without an admin
        let group_without_admin = Group {
            id: "test".to_string(),
            metadata: GroupMetadata::new("test".to_string()),
            members: HashMap::new(), // Empty members map = no admin
            ..Default::default()
        };
        assert!(
            group_without_admin.admin_pubkeys().is_empty(),
            "Group should have no admins"
        );
    }

    #[tokio::test]
    async fn test_set_roles_cannot_change_last_admin() {
        let (admin_keys, _, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Attempt to change the last admin to a regular member
        let set_roles_tags = vec![Tag::custom(
            TagKind::p(),
            [admin_keys.public_key().to_string(), "member".to_string()],
        )];
        let set_roles_event =
            create_test_event(&admin_keys, KIND_GROUP_SET_ROLES_9006, set_roles_tags).await;

        // Should fail with "Cannot remove last admin" error
        let result = group.set_roles(&set_roles_event, &admin_keys.public_key());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Cannot remove last admin");

        // Verify the admin still has admin role
        assert!(group.is_admin(&admin_keys.public_key()));
    }

    #[tokio::test]
    async fn test_set_roles_can_change_admin_when_multiple_admins() {
        let (admin_keys, member_keys, _) = create_test_keys();
        let (mut group, _) = create_test_group(&admin_keys).await;

        // First add the user as a regular member
        let add_member_tags = vec![Tag::public_key(member_keys.public_key())];
        let add_member_event =
            create_test_event(&admin_keys, KIND_GROUP_ADD_USER_9000, add_member_tags).await;
        group
            .add_members_from_event(&add_member_event, &admin_keys.public_key())
            .unwrap();
        assert!(group.is_member(&member_keys.public_key()));

        // Then make them an admin
        let add_admin_tags = vec![Tag::custom(
            TagKind::p(),
            [member_keys.public_key().to_string(), "admin".to_string()],
        )];
        let add_admin_event =
            create_test_event(&admin_keys, KIND_GROUP_SET_ROLES_9006, add_admin_tags).await;
        group
            .set_roles(&add_admin_event, &admin_keys.public_key())
            .unwrap();
        assert!(group.is_admin(&member_keys.public_key()));

        // Now we can change the original admin to a member since there's another admin
        let change_role_tags = vec![Tag::custom(
            TagKind::p(),
            [admin_keys.public_key().to_string(), "member".to_string()],
        )];
        let change_role_event =
            create_test_event(&admin_keys, KIND_GROUP_SET_ROLES_9006, change_role_tags).await;

        // Should succeed
        let result = group.set_roles(&change_role_event, &admin_keys.public_key());
        assert!(result.is_ok());
        assert!(!group.is_admin(&admin_keys.public_key()));
        assert!(group.is_admin(&member_keys.public_key()));
    }
}
