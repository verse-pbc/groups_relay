use crate::error::Error;
use crate::StoreCommand;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use strum::{Display, EnumIter, IntoEnumIterator};
use tracing::{debug, error, info, warn};

#[derive(Debug, thiserror::Error)]
pub enum GroupError {
    #[error("Group not found: {0}")]
    NotFound(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Validation failed: {0}")]
    ValidationFailed(String),
    #[error("Invalid state: {0}")]
    InvalidState(String),
    #[error("Internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

impl From<GroupError> for Error {
    fn from(err: GroupError) -> Self {
        match err {
            GroupError::NotFound(msg) => Error::notice(msg),
            GroupError::PermissionDenied(msg) => Error::restricted(msg),
            GroupError::ValidationFailed(msg) => Error::notice(msg),
            GroupError::InvalidState(msg) => Error::notice(msg),
            GroupError::Internal(err) => Error::notice(err.to_string()),
        }
    }
}

// NIP-60 Cashu Wallet kinds
pub const KIND_WALLET_17375: Kind = Kind::Custom(17375);
pub const KIND_TOKEN_7375: Kind = Kind::Custom(7375);
pub const KIND_SPENDING_HISTORY_7376: Kind = Kind::Custom(7376);
pub const KIND_QUOTE_7374: Kind = Kind::Custom(7374);

// NIP-61 Nutzap kinds
pub const KIND_NUTZAP_INFO_10019: Kind = Kind::Custom(10019);
pub const KIND_NUTZAP_9321: Kind = Kind::Custom(9321);

pub const KIND_SIMPLE_LIST_10009: Kind = Kind::Custom(10009);
pub const KIND_CLAIM_28934: Kind = Kind::Custom(28934);

// MLS Related
pub const KIND_GIFT_WRAP: Kind = Kind::GiftWrap;
pub const KIND_MLS_KEY_PACKAGE: Kind = Kind::MlsKeyPackage;

pub const KIND_PUSH_REGISTRATION_3079: Kind = Kind::Custom(3079);
pub const KIND_PUSH_DEREGISTRATION_3080: Kind = Kind::Custom(3080);

pub const KIND_GENERAL_EVENT_DELETION: Kind = Kind::EventDeletion;

// Custom event kinds for groups
pub const KIND_GROUP_CREATE_9007: Kind = Kind::Custom(9007); // Admin/Relay -> Relay: Create a new group
pub const KIND_GROUP_DELETE_9008: Kind = Kind::Custom(9008); // Admin/Relay -> Relay: Delete an existing group
pub const KIND_GROUP_ADD_USER_9000: Kind = Kind::Custom(9000); // Admin/Relay -> Relay: Add user to group
pub const KIND_GROUP_REMOVE_USER_9001: Kind = Kind::Custom(9001); // Admin/Relay -> Relay: Remove user from group
pub const KIND_GROUP_EDIT_METADATA_9002: Kind = Kind::Custom(9002); // Admin/Relay -> Relay: Edit group metadata
pub const KIND_GROUP_DELETE_EVENT_9005: Kind = Kind::Custom(9005); // Admin/Relay -> Relay: Delete specific event
pub const KIND_GROUP_SET_ROLES_9006: Kind = Kind::Custom(9006); // Admin/Relay -> Relay: Set roles for group. This was removed but at least 0xchat uses it
pub const KIND_GROUP_CREATE_INVITE_9009: Kind = Kind::Custom(9009); // Admin/Relay -> Relay: Create invite for closed group

pub const KIND_GROUP_USER_JOIN_REQUEST_9021: Kind = Kind::Custom(9021); // User -> Relay: Request to join group
pub const KIND_GROUP_USER_LEAVE_REQUEST_9022: Kind = Kind::Custom(9022); // User -> Relay: Request to leave group

pub const KIND_GROUP_METADATA_39000: Kind = Kind::Custom(39000); // Relay -> All: Group metadata
pub const KIND_GROUP_ADMINS_39001: Kind = Kind::Custom(39001); // Relay -> All: List of group admins
pub const KIND_GROUP_MEMBERS_39002: Kind = Kind::Custom(39002); // Relay -> All: List of group members
pub const KIND_GROUP_ROLES_39003: Kind = Kind::Custom(39003); // Relay -> All: Supported roles in group

pub const ADDRESSABLE_EVENT_KINDS: [Kind; 4] = [
    KIND_GROUP_METADATA_39000,
    KIND_GROUP_ADMINS_39001,
    KIND_GROUP_MEMBERS_39002,
    KIND_GROUP_ROLES_39003,
];

pub const NON_GROUP_ALLOWED_KINDS: [Kind; 13] = [
    KIND_SIMPLE_LIST_10009,
    KIND_CLAIM_28934,
    KIND_WALLET_17375,
    KIND_TOKEN_7375,
    KIND_SPENDING_HISTORY_7376,
    KIND_QUOTE_7374,
    KIND_NUTZAP_INFO_10019,
    KIND_NUTZAP_9321,
    KIND_GIFT_WRAP,
    KIND_MLS_KEY_PACKAGE,
    KIND_GENERAL_EVENT_DELETION,
    KIND_PUSH_REGISTRATION_3079,
    KIND_PUSH_DEREGISTRATION_3080,
];

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
    KIND_CLAIM_28934,
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
    /// Broadcast = only admins can publish content events (except join/leave)
    pub is_broadcast: bool,
}

impl GroupMetadata {
    pub fn new(name: String) -> Self {
        Self {
            name,
            about: None,
            picture: None,
            private: true,
            closed: true,
            is_broadcast: false, // Default to false
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A Nostr group that implements NIP-29 group management.
///
/// Groups have the following key characteristics:
/// - Must always have at least one admin
/// - Can be public (readable by anyone) or private (requires authentication)
/// - Can be open (anyone can join) or closed (requires invite)
/// - Supports role-based access control (admin, member, custom roles)
/// - Maintains state for members, invites, and join requests
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

        let mut group = Self {
            id: group_id.to_string(),
            metadata: GroupMetadata::new(group_id.to_string()),
            updated_at: event.created_at,
            ..Default::default()
        };

        // Only set created_at for group creation events
        if event.kind == KIND_GROUP_CREATE_9007 {
            group.created_at = event.created_at;
        }

        group
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
        writeln!(f, "    closed: {},", self.metadata.closed)?;
        writeln!(f, "    is_broadcast: {},", self.metadata.is_broadcast)?;
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

    /// Checks if an event kind is a group management kind that requires a managed group to exist
    pub fn is_group_management_kind(kind: Kind) -> bool {
        ADDRESSABLE_EVENT_KINDS.contains(&kind)
            || ALL_GROUP_KINDS_EXCEPT_DELETE_AND_ADDRESSABLE.contains(&kind)
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
        if event.kind != KIND_GROUP_CREATE_9007 {
            return Err(Error::notice("Invalid event kind for group creation"));
        }

        let mut group = Self::from(event);
        if group.id.is_empty() {
            return Err(Error::notice("Group ID not found"));
        }

        // Add the creator as an admin
        group
            .members
            .insert(event.pubkey, GroupMember::new_admin(event.pubkey));

        Ok(group)
    }

    pub fn delete_group_request(
        &self,
        delete_group_request_event: Box<Event>,
        relay_pubkey: &PublicKey,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        if delete_group_request_event.kind != KIND_GROUP_DELETE_9008 {
            return Err(Error::notice("Invalid event kind for delete group"));
        }

        self.can_delete_group(authed_pubkey, relay_pubkey, &delete_group_request_event)?;

        // Delete all group kinds possible except this delete request (kind 9008)
        let non_addressable_filter =
            Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::H), self.id.to_string());

        let addressable_filter =
            Filter::new().custom_tag(SingleLetterTag::lowercase(Alphabet::D), self.id.to_string());

        Ok(vec![
            StoreCommand::DeleteEvents(non_addressable_filter),
            StoreCommand::DeleteEvents(addressable_filter),
            StoreCommand::SaveSignedEvent(delete_group_request_event),
        ])
    }

    pub fn delete_event_request(
        &mut self,
        delete_request_event: Box<Event>,
        relay_pubkey: &PublicKey,
        authed_pubkey: &Option<PublicKey>,
    ) -> Result<Vec<StoreCommand>, Error> {
        if delete_request_event.kind != KIND_GROUP_DELETE_EVENT_9005 {
            return Err(Error::notice("Invalid event kind for delete event"));
        }

        // Get the event IDs from the tags
        let event_ids: Vec<_> = delete_request_event.tags.event_ids().copied().collect();
        if event_ids.is_empty() {
            return Err(Error::notice("No event IDs found in delete request"));
        }

        self.can_delete_event(authed_pubkey, relay_pubkey, &delete_request_event, "event")?;

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
            StoreCommand::SaveSignedEvent(delete_request_event),
        ])
    }

    pub fn add_members_from_event(
        &mut self,
        members_event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        if members_event.kind != KIND_GROUP_ADD_USER_9000 {
            return Err(Error::notice("Invalid event kind for add members"));
        }

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

        self.add_members(group_members)?;

        let mut events = vec![StoreCommand::SaveSignedEvent(members_event)];
        let admins_event = self.generate_admins_event(relay_pubkey);
        events.push(StoreCommand::SaveUnsignedEvent(admins_event));
        let members_event = self.generate_members_event(relay_pubkey);
        events.push(StoreCommand::SaveUnsignedEvent(members_event));

        Ok(events)
    }

    pub fn add_members(
        &mut self,
        group_members: impl Iterator<Item = GroupMember>,
    ) -> Result<(), Error> {
        for member in group_members {
            self.join_requests.remove(&member.pubkey);

            // If the member exists, check if we're removing the last admin
            if let Some(existing) = self.members.get(&member.pubkey) {
                // Prevent removing the last admin role.
                if self.admin_pubkeys().len() == 1
                    && existing.roles.contains(&GroupRole::Admin)
                    && !member.roles.contains(&GroupRole::Admin)
                {
                    return Err(Error::notice("Cannot unset last admin role"));
                }
            }

            self.members.insert(member.pubkey, member);
        }

        self.update_roles();
        self.update_state();
        Ok(())
    }

    pub fn add_pubkey(&mut self, pubkey: PublicKey) -> Result<(), Error> {
        let member = GroupMember::new_member(pubkey);
        self.add_members(vec![member].into_iter())
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
        members_event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        if members_event.kind != KIND_GROUP_REMOVE_USER_9001 {
            return Err(Error::notice("Invalid event kind for remove members"));
        }

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
        let mut removed_admins = false;

        for tag in members_event.tags.filter(TagKind::p()) {
            let member = GroupMember::try_from(tag)?;
            let removed_pubkey = member.pubkey;

            // Exit early if this removal would remove the last admin.
            if admins.len() == 1 && admins.contains(&removed_pubkey) {
                return Err(Error::notice("Cannot remove last admin"));
            }

            // Skip if the member doesn't exist.
            if !self.members.contains_key(&removed_pubkey) {
                continue;
            }

            let is_admin = self.is_admin(&removed_pubkey);
            self.members.remove(&removed_pubkey);
            self.join_requests.remove(&removed_pubkey);

            if is_admin {
                removed_admins = true;
            }
        }

        self.update_roles();
        self.update_state();

        let mut events = vec![StoreCommand::SaveSignedEvent(members_event)];
        if removed_admins {
            let admins_event = self.generate_admins_event(relay_pubkey);
            events.push(StoreCommand::SaveUnsignedEvent(admins_event));
        }
        let members_event = self.generate_members_event(relay_pubkey);
        events.push(StoreCommand::SaveUnsignedEvent(members_event));

        Ok(events)
    }

    pub fn set_metadata(&mut self, event: &Event, relay_pubkey: &PublicKey) -> Result<(), Error> {
        if event.kind != KIND_GROUP_EDIT_METADATA_9002 {
            return Err(Error::notice("Invalid event kind for set metadata"));
        }

        if !self.can_edit_metadata(&event.pubkey, relay_pubkey) {
            return Err(Error::notice("User cannot edit metadata"));
        }

        // Explicitly check for broadcast tag presence *in the incoming event*
        let broadcast_tag_present = event.tags.find(TagKind::custom("broadcast")).is_some();

        // Assume false unless the tag is explicitly present
        self.metadata.is_broadcast = broadcast_tag_present;

        for tag in event.tags.iter() {
            match tag.kind() {
                TagKind::Name => {
                    if let Some(name) = tag.content() {
                        self.metadata.name = name.to_string();
                    }
                }
                TagKind::Custom(kind) => match kind.as_ref() {
                    "about" => {
                        self.metadata.about = tag.content().map(|s| s.to_string());
                    }
                    "picture" => {
                        self.metadata.picture = tag.content().map(|s| s.to_string());
                    }
                    "public" => {
                        self.metadata.private = false;
                    }
                    "private" => {
                        self.metadata.private = true;
                    }
                    "open" => {
                        self.metadata.closed = false;
                    }
                    "closed" => {
                        self.metadata.closed = true;
                    }
                    _ => {}
                },
                _ => {}
            }
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
    pub fn set_roles(
        &mut self,
        event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        if event.kind != KIND_GROUP_SET_ROLES_9006 {
            return Err(Error::notice("Invalid event kind for set roles"));
        }

        if !self.can_edit_members(&event.pubkey, relay_pubkey) {
            return Err(Error::notice("User is not authorized to set roles"));
        }

        let current_admins = self.admin_pubkeys();
        for tag in event.tags.filter(TagKind::p()) {
            let member = GroupMember::try_from(tag)?;
            if current_admins.len() == 1
                && current_admins.contains(&member.pubkey)
                && !member.roles.contains(&GroupRole::Admin)
            {
                return Err(Error::notice("Cannot unset last admin role"));
            }
        }

        for tag in event.tags.filter(TagKind::p()) {
            let member = GroupMember::try_from(tag)?;
            if let Some(existing_member) = self.members.get_mut(&member.pubkey) {
                existing_member.roles = member.roles;
            }
        }

        self.update_roles();
        self.update_state();

        let roles_event = self.generate_roles_event(relay_pubkey);
        let members_event = self.generate_members_event(relay_pubkey);

        Ok(vec![
            StoreCommand::SaveSignedEvent(event),
            StoreCommand::SaveUnsignedEvent(roles_event),
            StoreCommand::SaveUnsignedEvent(members_event),
        ])
    }

    /// Processes a join request for the group.
    ///
    /// This method handles join requests in different ways depending on the group type and request:
    /// 1. If user is already a member: Returns Ok(false) without any changes
    /// 2. For open groups: Automatically adds the user as a member
    /// 3. For closed groups with invite: Adds user with roles from invite
    /// 4. For closed groups without invite: Adds user to join requests
    ///
    /// # Arguments
    /// * `event` - The join request event containing:
    ///   - The pubkey of the user requesting to join
    ///   - Optional invite code in the 'code' tag
    ///
    /// # Returns
    /// * `Ok(true)` - User was successfully added as a member
    /// * `Ok(false)` - User was added to join requests or is already a member
    /// * `Err` - Invalid event kind or other error
    pub fn join_request(
        &mut self,
        event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        if event.kind != KIND_GROUP_USER_JOIN_REQUEST_9021 {
            return Err(Error::notice(format!(
                "Invalid event kind for join request {}",
                event.kind
            )));
        }

        // If user is already a member, do nothing
        if self.members.contains_key(&event.pubkey) {
            info!("User {} is already a member", event.pubkey);
            return Err(Error::notice("User is already a member"));
        }

        if !self.metadata.closed {
            info!("Public group, adding member {}", event.pubkey);
            self.members
                .entry(event.pubkey)
                .or_insert(GroupMember::new_member(event.pubkey));
            self.join_requests.remove(&event.pubkey);
            self.update_state();
            return self.create_join_request_commands(true, event, relay_pubkey);
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
            return self.create_join_request_commands(false, event, relay_pubkey);
        };

        info!("Invite code matched, adding member {}", event.pubkey);
        let roles = invite.roles.clone();
        self.members
            .insert(event.pubkey, GroupMember::new(event.pubkey, roles));

        self.join_requests.remove(&event.pubkey);
        self.update_state();
        self.create_join_request_commands(true, event, relay_pubkey)
    }

    /// Handles group management events (add/remove users).
    /// Returns updated group events if the management action was successful.
    pub fn handle_group_content(
        &mut self,
        event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        let is_admin = self.is_admin(&event.pubkey);
        let is_member = self.is_member(&event.pubkey);
        let event_pubkey = event.pubkey;
        let event_kind = event.kind;
        let event_id = event.id;

        // Check broadcast restrictions first
        if self.metadata.is_broadcast
            && !is_admin
            && ![
                KIND_GROUP_USER_JOIN_REQUEST_9021,
                KIND_GROUP_USER_LEAVE_REQUEST_9022,
            ]
            .contains(&event_kind)
        {
            warn!(
                "Blocked non-admin event {} (kind {}) in broadcast group {}",
                event_id, event_kind, self.id
            );
            // Use Error::restricted as planned
            return Err(Error::restricted("Only admins can post in broadcast mode"));
        }

        let mut commands = vec![StoreCommand::SaveSignedEvent(event)];

        // For private and closed groups, only members can post
        if self.metadata.private && self.metadata.closed && !is_member {
            return Err(Error::notice("User is not a member of this group"));
        }

        // Open groups auto-join the author when posting
        if !self.metadata.closed && !is_member {
            self.add_pubkey(event_pubkey)?;
            commands.extend(
                self.generate_membership_events(relay_pubkey)
                    .into_iter()
                    .map(StoreCommand::SaveUnsignedEvent),
            );
        } else if !is_member {
            // For closed groups, non-members can't post
            return Err(Error::notice("User is not a member of this group"));
        }

        Ok(commands)
    }

    fn create_join_request_commands(
        &self,
        auto_joined: bool,
        event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        if event.kind != KIND_GROUP_USER_JOIN_REQUEST_9021 {
            return Err(Error::notice(format!(
                "Invalid event kind for join request {}",
                event.kind
            )));
        }

        let mut commands = vec![StoreCommand::SaveSignedEvent(event)];
        if auto_joined {
            commands.extend(
                self.generate_membership_events(relay_pubkey)
                    .into_iter()
                    .map(StoreCommand::SaveUnsignedEvent),
            );
        }

        Ok(commands)
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

        info!("Creating invite with code: {:?}", invite_event.tags);
        let invite_code = invite_event
            .tags
            .find(TagKind::custom("code"))
            .and_then(|t| t.content())
            .ok_or_else(|| Error::notice("Invite code not found in tag"))?;

        // Check for duplicate invite code
        if self.invites.contains_key(invite_code) {
            return Err(Error::notice("Invite code already exists"));
        }

        let invite = Invite::new(invite_event.id, HashSet::from([GroupRole::Member]));

        self.invites.insert(invite_code.to_string(), invite);
        self.update_state();
        Ok(true)
    }

    pub fn leave_request(
        &mut self,
        event: Box<Event>,
        relay_pubkey: &PublicKey,
    ) -> Result<Vec<StoreCommand>, Error> {
        if event.kind != KIND_GROUP_USER_LEAVE_REQUEST_9022 {
            return Err(Error::notice(format!(
                "Invalid event kind for leave request {}",
                event.kind
            )));
        }

        self.join_requests.remove(&event.pubkey);
        let removed = self.members.remove(&event.pubkey).is_some();
        self.update_state();
        if removed {
            let members_event = self.generate_members_event(relay_pubkey);
            Ok(vec![
                StoreCommand::SaveSignedEvent(event),
                StoreCommand::SaveUnsignedEvent(members_event),
            ])
        } else {
            Ok(vec![])
        }
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
        let name = event
            .tags
            .find(TagKind::custom("name"))
            .and_then(|t| t.content());
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
        let is_broadcast = event.tags.find(TagKind::custom("broadcast")).is_some();

        self.metadata = GroupMetadata {
            name: name.unwrap_or(&self.id).to_string(),
            about: about.map(|s| s.to_string()),
            picture: picture.map(|s| s.to_string()),
            private,
            closed,
            is_broadcast,
        };

        self.update_timestamps(event);
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
        self.update_timestamps(event);
        Ok(())
    }

    pub fn load_join_request_from_event(&mut self, event: &Event) -> Result<(), Error> {
        if !self.members.contains_key(&event.pubkey) {
            self.join_requests.insert(event.pubkey);
            self.update_timestamps(event);
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
            self.update_timestamps(event);
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
            k if k.is_addressable() => event.tags.find(TagKind::d()).and_then(|t| t.content()),
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

    pub fn update_timestamps(&mut self, event: &Event) {
        // Only update created_at if this is a group creation event
        if event.kind == KIND_GROUP_CREATE_9007 {
            self.created_at = event.created_at;
        }
        // Always update updated_at to the latest timestamp
        self.updated_at = std::cmp::max(self.updated_at, event.created_at);
    }

    /// Generates all membership-related events for the group
    pub fn generate_membership_events(&self, relay_pubkey: &PublicKey) -> Vec<UnsignedEvent> {
        vec![
            self.generate_put_user_event(relay_pubkey),
            self.generate_admins_event(relay_pubkey),
            self.generate_members_event(relay_pubkey),
        ]
    }

    /// Generates all metadata-related events for the group
    pub fn generate_metadata_events(&self, relay_pubkey: &PublicKey) -> Vec<UnsignedEvent> {
        vec![
            self.generate_metadata_event(relay_pubkey),
            self.generate_roles_event(relay_pubkey),
        ]
    }

    /// Generates all group state events
    pub fn generate_all_state_events(&self, relay_pubkey: &PublicKey) -> Vec<UnsignedEvent> {
        let mut events = self.generate_metadata_events(relay_pubkey);
        events.extend(self.generate_membership_events(relay_pubkey));
        events
    }
}

// Event generation based on current state
impl Group {
    pub fn generate_put_user_event(&self, pubkey: &PublicKey) -> UnsignedEvent {
        UnsignedEvent::new(
            *pubkey,
            Timestamp::now_with_supplier(&Instant::now()),
            KIND_GROUP_ADD_USER_9000,
            vec![
                Tag::custom(
                    TagKind::p(),
                    vec![
                        pubkey.to_string(),
                        GroupRole::Member.as_tuple().0.to_string(),
                    ],
                ),
                Tag::custom(TagKind::h(), [self.id.clone()]),
            ],
            "".to_string(),
        )
    }

    pub fn generate_metadata_event(&self, pubkey: &PublicKey) -> UnsignedEvent {
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

        let mut tags = vec![
            Tag::identifier(self.id.clone()),
            Tag::custom(TagKind::Name, [self.metadata.name.clone()]),
            Tag::custom(TagKind::custom(access), &[] as &[String]),
            Tag::custom(TagKind::custom(visibility), &[] as &[String]),
        ];

        if let Some(about) = &self.metadata.about {
            tags.push(Tag::custom(TagKind::custom("about"), [about.clone()]));
        }

        if let Some(picture) = &self.metadata.picture {
            tags.push(Tag::custom(TagKind::custom("picture"), [picture.clone()]));
        }

        // Add broadcast tag if needed
        if self.metadata.is_broadcast {
            tags.push(Tag::custom(TagKind::custom("broadcast"), &[] as &[String]));
        }

        UnsignedEvent::new(
            *pubkey,
            Timestamp::now_with_supplier(&Instant::now()),
            KIND_GROUP_METADATA_39000,
            tags,
            "".to_string(),
        )
    }

    pub fn generate_admins_event(&self, pubkey: &PublicKey) -> UnsignedEvent {
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

        UnsignedEvent::new(
            *pubkey,
            Timestamp::now_with_supplier(&Instant::now()),
            KIND_GROUP_ADMINS_39001,
            tags,
            "".to_string(),
        )
    }

    pub fn generate_members_event(&self, pubkey: &PublicKey) -> UnsignedEvent {
        let members: Vec<&PublicKey> = self.members.keys().collect();

        let mut tags = Vec::new();
        tags.push(Tag::identifier(self.id.clone()));

        for pubkey in members {
            tags.push(Tag::public_key(*pubkey));
        }

        UnsignedEvent::new(
            *pubkey,
            Timestamp::now_with_supplier(&Instant::now()),
            KIND_GROUP_MEMBERS_39002,
            tags,
            "".to_string(),
        )
    }

    pub fn generate_roles_event(&self, pubkey: &PublicKey) -> UnsignedEvent {
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

        UnsignedEvent::new(
            *pubkey,
            Timestamp::now_with_supplier(&Instant::now()),
            KIND_GROUP_ROLES_39003,
            tags,
            "List of roles supported by this group".to_string(),
        )
    }
}

// Authorization checks
impl Group {
    pub fn can_edit_members(&self, pubkey: &PublicKey, relay_pubkey: &PublicKey) -> bool {
        if pubkey == relay_pubkey {
            return true;
        }

        if !self.is_admin(pubkey) {
            return false;
        }

        true
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
    ) -> Result<(), Error> {
        self.can_delete_event(authed_pubkey, relay_pubkey, delete_group_event, "group")
    }

    pub fn can_delete_event(
        &self,
        authed_pubkey: &Option<PublicKey>,
        relay_pubkey: &PublicKey,
        event: &Event,
        target: &str,
    ) -> Result<(), Error> {
        let Some(authed_pubkey) = authed_pubkey else {
            return Err(Error::auth_required("User is not authenticated"));
        };

        // Relay pubkey can delete all events
        if relay_pubkey == authed_pubkey {
            debug!(
                "Relay pubkey {} can delete {} {}, kind {}",
                relay_pubkey, target, event.id, event.kind
            );
            return Ok(());
        }

        // Only admins can delete events
        if self.is_admin(authed_pubkey) {
            debug!(
                "Admin {} can delete {} {}, kind {}",
                authed_pubkey, target, event.id, event.kind
            );
            return Ok(());
        }

        Err(Error::restricted(
            "User is not authorized to delete this event",
        ))
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
    use crate::test_utils::{
        add_member_to_group, create_test_delete_event, create_test_event, create_test_group,
        create_test_invite_event, create_test_keys, create_test_metadata_event,
        create_test_role_event, remove_member_from_group,
    };
    #[tokio::test]
    async fn test_group_creation() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (group, group_id) = create_test_group(&admin_keys).await;

        assert_eq!(group.id, group_id);
        assert_eq!(group.metadata.name, group_id);
        assert!(group.is_admin(&admin_keys.public_key()));
        assert_eq!(group.members.len(), 1);
    }

    #[tokio::test]
    async fn test_add_members() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        add_member_to_group(&mut group, &admin_keys, &member_keys, &group_id).await;

        assert!(group.is_member(&member_keys.public_key()));
        assert!(!group.is_admin(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_remove_members() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // First add a member
        add_member_to_group(&mut group, &admin_keys, &member_keys, &group_id).await;
        assert!(group.is_member(&member_keys.public_key()));

        // Then remove them
        remove_member_from_group(&mut group, &admin_keys, &member_keys, &group_id).await;
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_metadata_management() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let event = create_test_metadata_event(
            &admin_keys,
            &group_id,
            Some("test_name"),
            Some("test_about"),
            Some("test_picture"),
            true,
            true,
            None, // is_broadcast
        )
        .await;

        assert!(group.set_metadata(&event, &admin_keys.public_key()).is_ok());
        assert_eq!(group.metadata.name, "test_name");
        assert_eq!(group.metadata.about, Some("test_about".to_string()));
        assert_eq!(group.metadata.picture, Some("test_picture".to_string()));
        assert!(group.metadata.private);
        assert!(group.metadata.closed);
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_name() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let metadata_event = create_test_metadata_event(
            &admin_keys,
            &group_id,
            Some("New Group Name"),
            None,
            None,
            true,
            true,
            None, // is_broadcast
        )
        .await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.name, "New Group Name");
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_about() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let metadata_event = create_test_metadata_event(
            &admin_keys,
            &group_id,
            None,
            Some("About text"),
            None,
            true,
            true,
            None, // is_broadcast
        )
        .await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.about, Some("About text".to_string()));
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_picture() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let metadata_event = create_test_metadata_event(
            &admin_keys,
            &group_id,
            None,
            None,
            Some("picture_url"),
            true,
            true,
            None, // is_broadcast
        )
        .await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_visibility() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // Test setting to public
        let public_event =
            create_test_metadata_event(&admin_keys, &group_id, None, None, None, false, true, None)
                .await;

        assert!(group
            .set_metadata(&public_event, &admin_keys.public_key())
            .is_ok());
        assert!(!group.metadata.private);

        // Test setting back to private
        let private_event =
            create_test_metadata_event(&admin_keys, &group_id, None, None, None, true, true, None)
                .await;

        assert!(group
            .set_metadata(&private_event, &admin_keys.public_key())
            .is_ok());
        assert!(group.metadata.private);
    }

    #[tokio::test]
    async fn test_metadata_management_can_set_multiple_fields() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let metadata_event = create_test_metadata_event(
            &admin_keys,
            &group_id,
            Some("New Group Name"),
            Some("About text"),
            Some("picture_url"),
            false,
            true,
            None, // is_broadcast
        )
        .await;

        assert!(group
            .set_metadata(&metadata_event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.metadata.name, "New Group Name");
        assert_eq!(group.metadata.about, Some("About text".to_string()));
        assert_eq!(group.metadata.picture, Some("picture_url".to_string()));
        assert!(!group.metadata.private);
    }

    #[tokio::test]
    async fn test_invite_system() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let event = create_test_invite_event(&admin_keys, &group_id, "test_invite_123").await;

        assert!(group
            .create_invite(&event, &admin_keys.public_key())
            .is_ok());
        assert_eq!(group.invites.len(), 1);
    }

    #[tokio::test]
    async fn test_invite_system_admin_can_create_invite() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let invite_code = "test_invite_123";
        let create_invite_event =
            create_test_invite_event(&admin_keys, &group_id, invite_code).await;

        assert!(group
            .create_invite(&create_invite_event, &admin_keys.public_key())
            .unwrap());
        assert!(group.invites.contains_key(invite_code));
    }

    #[tokio::test]
    async fn test_invite_system_user_can_join_with_valid_invite() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // Create invite
        let invite_code = "test_invite_123";
        let create_invite_event =
            create_test_invite_event(&admin_keys, &group_id, invite_code).await;
        group
            .create_invite(&create_invite_event, &admin_keys.public_key())
            .unwrap();

        // Use invite
        let join_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::custom(TagKind::Custom("code".into()), [invite_code]),
        ];
        let join_event = create_test_event(&member_keys, 9021, join_tags).await;

        assert!(!group
            .join_request(Box::new(join_event), &member_keys.public_key())
            .unwrap()
            .is_empty());
        assert!(group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_join_request() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let event = create_test_event(&member_keys, 9021, tags).await;

        assert!(!group
            .join_request(Box::new(event), &member_keys.public_key())
            .unwrap()
            .is_empty());
        assert_eq!(group.join_requests.len(), 1);
    }

    #[tokio::test]
    async fn test_join_request_adds_to_join_requests() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let join_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let join_event = create_test_event(&member_keys, 9021, join_tags).await;

        assert!(!group
            .join_request(Box::new(join_event), &member_keys.public_key())
            .unwrap()
            .is_empty());
        assert!(group.join_requests.contains(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_join_request_from_existing_member() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // First add the member
        group.members.insert(
            member_keys.public_key(),
            GroupMember::new_member(member_keys.public_key()),
        );
        let initial_member_count = group.members.len();

        // Try to join again
        let join_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let join_event = create_test_event(&member_keys, 9021, join_tags).await;

        // Should return error with message "User is already a member"
        assert_eq!(
            group
                .join_request(Box::new(join_event), &member_keys.public_key())
                .unwrap_err()
                .to_string(),
            "User is already a member"
        );

        // Verify member is still there with same role
        let member = group.members.get(&member_keys.public_key()).unwrap();
        assert!(member.roles.contains(&GroupRole::Member));
        // Member count should not change
        assert_eq!(group.members.len(), initial_member_count);
    }

    #[tokio::test]
    async fn test_leave_request_removes_member() {
        let (admin_keys, member_keys, relay_pubkey) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // Add member manually
        add_member_to_group(&mut group, &admin_keys, &member_keys, &group_id).await;
        assert!(group.is_member(&member_keys.public_key()));

        // Test leave request
        let leave_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let leave_event = create_test_event(&member_keys, 9022, leave_tags).await;

        let result = group.leave_request(Box::new(leave_event), &relay_pubkey.public_key());

        assert!(!result.unwrap().is_empty());
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_event_visibility() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (group, group_id) = create_test_group(&admin_keys).await;

        let tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let event = create_test_event(&member_keys, 11, tags).await;

        assert!(group
            .can_see_event(
                &Some(member_keys.public_key()),
                &admin_keys.public_key(),
                &event
            )
            .unwrap());
    }

    #[tokio::test]
    async fn test_event_visibility_admin_can_see_events() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (group, group_id) = create_test_group(&admin_keys).await;

        let test_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let test_event = create_test_event(&member_keys, 9, test_tags).await;

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
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // Add a member
        let add_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, 9000, add_tags).await;
        group
            .add_members_from_event(Box::new(add_event), &admin_keys.public_key())
            .unwrap();

        let test_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let test_event = create_test_event(&member_keys, 9, test_tags).await;

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
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let (group, group_id) = create_test_group(&admin_keys).await;

        let test_tags = vec![Tag::custom(TagKind::h(), [&group_id])];
        let test_event = create_test_event(&member_keys, 9, test_tags).await;

        assert!(!group
            .can_see_event(
                &Some(non_member_keys.public_key()),
                &admin_keys.public_key(),
                &test_event
            )
            .unwrap());
    }

    #[tokio::test]
    async fn test_delete_event_request_unauthenticated() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        let event = create_test_event(
            &member_keys,
            11,
            vec![Tag::custom(TagKind::h(), [&group_id])],
        )
        .await;
        let delete_event = create_test_delete_event(&admin_keys, &group_id, &event).await;

        let result = group.delete_event_request(Box::new(delete_event), &relay_pubkey, &None);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Auth required: User is not authenticated"
        );
    }

    #[tokio::test]
    async fn test_delete_event_request_wrong_kind() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, _group_id) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create a regular event to delete
        let event_to_delete = create_test_event(
            &member_keys,
            11, // Regular event
            vec![Tag::custom(TagKind::h(), [group.id.clone()])],
        )
        .await;

        // Create delete request with wrong kind (9 instead of 9005)
        let delete_request = create_test_event(
            &admin_keys,
            9, // Wrong kind - should be 9005
            vec![
                Tag::custom(TagKind::h(), [group.id.clone()]),
                Tag::event(event_to_delete.id),
            ],
        )
        .await;

        let result = group.delete_event_request(
            Box::new(delete_request),
            &relay_pubkey,
            &Some(admin_keys.public_key()),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid event kind for delete event"
        );
    }

    #[tokio::test]
    async fn test_delete_event_request_non_member() {
        let (admin_keys, _, non_member_keys) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        let event = create_test_event(
            &admin_keys,
            11,
            vec![Tag::custom(TagKind::h(), [&group_id])],
        )
        .await;
        let delete_event = create_test_delete_event(&non_member_keys, &group_id, &event).await;

        let result = group.delete_event_request(
            Box::new(delete_event),
            &relay_pubkey,
            &Some(non_member_keys.public_key()),
        );

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Restricted: User is not authorized to delete this event"
        );
    }

    #[tokio::test]
    async fn test_remove_members_cannot_remove_last_admin() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        let tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(admin_keys.public_key()),
        ];
        let event = create_test_event(&admin_keys, 9001, tags).await;

        assert!(group
            .remove_members(Box::new(event), &admin_keys.public_key())
            .is_err());
    }

    #[tokio::test]
    async fn test_group_creation_always_has_admin() {
        let (admin_keys, _, _) = create_test_keys().await;
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
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // Attempt to change the last admin to a regular member
        let event =
            create_test_role_event(&admin_keys, &group_id, admin_keys.public_key(), "member").await;

        // Should fail with "Cannot remove last admin" error
        let result = group.set_roles(Box::new(event), &admin_keys.public_key());
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot unset last admin role"
        );

        // Verify the admin still has admin role
        assert!(group.is_admin(&admin_keys.public_key()));
    }

    #[tokio::test]
    async fn test_set_roles_can_change_admin_when_multiple_admins() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // First add the user as a regular member
        add_member_to_group(&mut group, &admin_keys, &member_keys, &group_id).await;
        assert!(group.is_member(&member_keys.public_key()));

        // Then make them an admin
        let add_admin_event =
            create_test_role_event(&admin_keys, &group_id, member_keys.public_key(), "admin").await;
        group
            .set_roles(Box::new(add_admin_event), &admin_keys.public_key())
            .unwrap();
        assert!(group.is_admin(&member_keys.public_key()));

        // Now we can change the original admin to a member since there's another admin
        let event =
            create_test_role_event(&admin_keys, &group_id, admin_keys.public_key(), "member").await;

        // Should succeed
        let result = group.set_roles(Box::new(event), &admin_keys.public_key());
        assert!(result.is_ok());
        assert!(!group.is_admin(&admin_keys.public_key()));
        assert!(group.is_admin(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_delete_event_request_deleting_invite() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // Create an invite
        let invite_code = "test_invite_123";
        let create_invite_event =
            create_test_invite_event(&admin_keys, &group_id, invite_code).await;
        group
            .create_invite(&create_invite_event, &relay_pubkey)
            .unwrap();
        assert!(group.invites.contains_key(invite_code));

        // Delete the invite event
        let delete_event =
            create_test_delete_event(&admin_keys, &group_id, &create_invite_event).await;
        let result = group.delete_event_request(
            Box::new(delete_event),
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
    async fn test_handle_remove_user_admin_removes_member() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, group_id) = create_test_group(&admin_keys).await;
        let relay_pubkey = admin_keys.public_key();

        // First add a member
        let add_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let add_event = create_test_event(&admin_keys, 9000, add_tags).await;
        group
            .add_members_from_event(Box::new(add_event), &relay_pubkey)
            .unwrap();
        assert!(group.is_member(&member_keys.public_key()));

        // Then remove them
        let remove_tags = vec![
            Tag::custom(TagKind::h(), [&group_id]),
            Tag::public_key(member_keys.public_key()),
        ];
        let remove_event = create_test_event(&admin_keys, 9001, remove_tags).await;

        let result = group.remove_members(Box::new(remove_event), &relay_pubkey);

        assert!(!result.unwrap().is_empty());
        assert!(!group.is_member(&member_keys.public_key()));
    }

    #[tokio::test]
    async fn test_add_pubkey_propagates_errors() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Try to add the existing admin as a regular member, which should fail
        // because it would remove the last admin role
        let result = group.add_pubkey(admin_keys.public_key());
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot unset last admin role"
        );

        // Verify admin still has admin role
        let stored_member = group.members.get(&admin_keys.public_key()).unwrap();
        assert!(stored_member.roles.contains(&GroupRole::Admin));
    }

    #[tokio::test]
    async fn test_add_members_allows_role_downgrades() {
        let (admin_keys, member_keys, _) = create_test_keys().await;
        let (mut group, _) = create_test_group(&admin_keys).await;

        // First add a second admin
        let mut admin_roles = HashSet::new();
        admin_roles.insert(GroupRole::Admin);
        let second_admin = GroupMember::new(member_keys.public_key(), admin_roles);
        assert!(group.add_members(vec![second_admin].into_iter()).is_ok());

        // Now downgrade the second admin to a regular member (should succeed since we have another admin)
        let member_basic = GroupMember::new_member(member_keys.public_key());
        let result = group.add_members(vec![member_basic].into_iter());
        assert!(result.is_ok());

        // Verify member was downgraded to just Member role
        let stored_member = group.members.get(&member_keys.public_key()).unwrap();
        assert_eq!(stored_member.roles.len(), 1);
        assert!(stored_member.roles.contains(&GroupRole::Member));
        assert!(!stored_member.roles.contains(&GroupRole::Admin));

        // Verify original admin still has admin role
        let original_admin = group.members.get(&admin_keys.public_key()).unwrap();
        assert!(original_admin.roles.contains(&GroupRole::Admin));
    }

    #[tokio::test]
    async fn test_add_members_prevents_last_admin_downgrade() {
        let (admin_keys, _, _) = create_test_keys().await;
        let (mut group, _) = create_test_group(&admin_keys).await;

        // Try to downgrade the only admin to a regular member (should fail)
        let member_basic = GroupMember::new_member(admin_keys.public_key());
        let result = group.add_members(vec![member_basic].into_iter());
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot unset last admin role"
        );

        // Verify admin still has admin role
        let stored_member = group.members.get(&admin_keys.public_key()).unwrap();
        assert!(stored_member.roles.contains(&GroupRole::Admin));
    }

    #[tokio::test]
    async fn test_broadcast_mode_restrictions() {
        let (admin_keys, member_keys, non_member_keys) = create_test_keys().await;
        let relay_keys = Keys::generate(); // For generating state events
        let (mut group, group_id) = create_test_group(&admin_keys).await;

        // Add member
        add_member_to_group(&mut group, &admin_keys, &member_keys, &group_id).await;

        // --- Scenario 1: Broadcast Mode Enabled ---
        // Setup: Update metadata via kind:9002 to include ["broadcast"] tag.
        let broadcast_metadata_event = create_test_metadata_event(
            &admin_keys,
            &group_id,
            None,       // name
            None,       // about
            None,       // picture
            true,       // is_private
            true,       // is_closed
            Some(true), // is_broadcast
        )
        .await;
        assert!(group
            .set_metadata(&broadcast_metadata_event, &admin_keys.public_key())
            .is_ok());
        assert!(group.metadata.is_broadcast);

        // Test Admin Publishing (succeeds)
        let admin_text_note = create_test_event(
            &admin_keys,
            Kind::TextNote.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id.clone()])],
        )
        .await;
        assert!(group
            .handle_group_content(Box::new(admin_text_note), &relay_keys.public_key())
            .is_ok());

        // Test Member Publishing Standard Content (blocked)
        let member_text_note = create_test_event(
            &member_keys,
            Kind::TextNote.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id.clone()])],
        )
        .await;
        let result =
            group.handle_group_content(Box::new(member_text_note), &relay_keys.public_key());
        assert!(matches!(result, Err(Error::Restricted { .. }))); // Use Error::Restricted
        assert_eq!(
            result.unwrap_err().to_string(),
            "Restricted: Only admins can post in broadcast mode"
        );

        // Test Non-Member Publishing Standard Content (blocked, handled by membership check)
        let non_member_text_note = create_test_event(
            &non_member_keys,
            Kind::TextNote.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id.clone()])],
        )
        .await;
        let result_non_member =
            group.handle_group_content(Box::new(non_member_text_note), &relay_keys.public_key());
        // In a closed, broadcast group, the broadcast restriction for non-admins takes precedence
        assert!(matches!(result_non_member, Err(Error::Restricted { .. })));
        assert_eq!(
            result_non_member.unwrap_err().to_string(),
            "Restricted: Only admins can post in broadcast mode" // Broadcast check happens first
        );

        // Test Member Publishing Join Request (allowed)
        // Note: A member trying to join again is pointless, but the broadcast check shouldn't block it.
        // We need a separate test for the join_request function's logic.
        // Here, we just test if handle_group_content would block it based on broadcast.
        // Let's assume, hypothetically, join requests went through handle_group_content.
        // We know they don't, but this checks the broadcast logic isolation.
        let member_join_request = create_test_event(
            &member_keys,
            KIND_GROUP_USER_JOIN_REQUEST_9021.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id.clone()])],
        )
        .await;
        // We expect this *not* to fail with Error::Restricted, but it might fail other checks.
        // The `handle_group_content` doesn't handle join/leave, so we can't directly assert here.
        // We rely on the code inspection: the check explicitly allows these kinds.

        // Test Member Publishing Leave Request (allowed)
        // Similar to join requests, leave requests are handled separately.
        let member_leave_request = create_test_event(
            &member_keys,
            KIND_GROUP_USER_LEAVE_REQUEST_9022.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id.clone()])],
        )
        .await;
        // We expect this *not* to fail with Error::Restricted.

        // --- Scenario 2: Broadcast Mode Disabled (Default) ---
        // Setup: Create a new group without the broadcast tag.
        let (admin_keys_2, member_keys_2, _) = create_test_keys().await;
        let (mut group_2, group_id_2) = create_test_group(&admin_keys_2).await;
        add_member_to_group(&mut group_2, &admin_keys_2, &member_keys_2, &group_id_2).await;
        assert!(!group_2.metadata.is_broadcast); // Should be false by default

        // Test Member Publishing (succeeds)
        let member_text_note_2 = create_test_event(
            &member_keys_2,
            Kind::TextNote.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id_2.clone()])],
        )
        .await;
        assert!(group_2
            .handle_group_content(Box::new(member_text_note_2), &relay_keys.public_key())
            .is_ok());

        // --- Scenario 3: Disabling Broadcast Mode ---
        // Setup: Enable broadcast mode, then disable it.
        let (admin_keys_3, member_keys_3, _) = create_test_keys().await;
        let (mut group_3, group_id_3) = create_test_group(&admin_keys_3).await;
        add_member_to_group(&mut group_3, &admin_keys_3, &member_keys_3, &group_id_3).await;

        // Enable broadcast
        let broadcast_metadata_event_3 = create_test_metadata_event(
            &admin_keys_3,
            &group_id_3,
            None,
            None,
            None,
            true,
            true,
            Some(true),
        )
        .await;
        assert!(group_3
            .set_metadata(&broadcast_metadata_event_3, &admin_keys_3.public_key())
            .is_ok());
        assert!(group_3.metadata.is_broadcast);

        // Disable broadcast by sending metadata event *without* the broadcast tag
        let no_broadcast_metadata_event_3 = create_test_metadata_event(
            &admin_keys_3,
            &group_id_3,
            None, // name
            None, // about
            None, // picture
            true, // is_private
            true, // is_closed
            None, // is_broadcast -> defaults to None/false
        )
        .await;
        assert!(group_3
            .set_metadata(&no_broadcast_metadata_event_3, &admin_keys_3.public_key())
            .is_ok());
        assert!(!group_3.metadata.is_broadcast);

        // Test Member Publishing (succeeds again)
        let member_text_note_3 = create_test_event(
            &member_keys_3,
            Kind::TextNote.as_u16(),
            vec![Tag::custom(TagKind::h(), [group_id_3.clone()])],
        )
        .await;
        assert!(group_3
            .handle_group_content(Box::new(member_text_note_3), &relay_keys.public_key())
            .is_ok());

        // Remove the panic! as the test should now pass (or fail meaningfully)
        // panic!("Test structure added, but logic and assertions are pending implementation.");
    }
}
