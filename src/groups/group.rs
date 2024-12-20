use crate::error::Error;
use anyhow::Result;
use nostr_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{collections::HashMap, collections::HashSet};
use strum::Display;
use strum::EnumIter;
use strum::IntoEnumIterator;
use tracing::{debug, error, info};
// Group Creation and Management
pub const KIND_GROUP_CREATE: Kind = Kind::Custom(9007); // Admin/Relay -> Relay: Create a new group
pub const KIND_GROUP_DELETE: Kind = Kind::Custom(9008); // Admin/Relay -> Relay: Delete an existing group

// Admin/Moderation Actions (9000-9005)
pub const KIND_GROUP_ADD_USER: Kind = Kind::Custom(9000); // Admin/Relay -> Relay: Add user to group
pub const KIND_GROUP_REMOVE_USER: Kind = Kind::Custom(9001); // Admin/Relay -> Relay: Remove user from group
pub const KIND_GROUP_EDIT_METADATA: Kind = Kind::Custom(9002); // Admin/Relay -> Relay: Edit group metadata
pub const KIND_GROUP_DELETE_EVENT: Kind = Kind::Custom(9005); // Admin/Relay -> Relay: Delete specific event
pub const KIND_GROUP_SET_ROLES: Kind = Kind::Custom(9006); // Admin/Relay -> Relay: Set roles for group. This was removed but at least 0xchat uses it
pub const KIND_GROUP_CREATE_INVITE: Kind = Kind::Custom(9009); // Admin/Relay -> Relay: Create invite for closed group

// User Actions (9021-9022)
pub const KIND_GROUP_USER_JOIN_REQUEST: Kind = Kind::Custom(9021); // User -> Relay: Request to join group
pub const KIND_GROUP_USER_LEAVE_REQUEST: Kind = Kind::Custom(9022); // User -> Relay: Request to leave group

// Relay-Generated Events (39000-39003)
pub const KIND_GROUP_METADATA: Kind = Kind::Custom(39000); // Relay -> All: Group metadata
pub const KIND_GROUP_ADMINS: Kind = Kind::Custom(39001); // Relay -> All: List of group admins
pub const KIND_GROUP_MEMBERS: Kind = Kind::Custom(39002); // Relay -> All: List of group members
pub const KIND_GROUP_ROLES: Kind = Kind::Custom(39003); // Relay -> All: Supported roles in group

pub const METADATA_EVENT_KINDS: [Kind; 4] = [
    KIND_GROUP_METADATA,
    KIND_GROUP_ADMINS,
    KIND_GROUP_MEMBERS,
    KIND_GROUP_ROLES,
];

// Regular content kinds allowed in groups
pub const GROUP_CONTENT_KINDS: [Kind; 3] = [Kind::Custom(9), Kind::Custom(11), Kind::Custom(10010)];

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
    pub pubkey: Option<PublicKey>,
    pub roles: HashSet<GroupRole>,
}

impl Invite {
    pub fn new(roles: HashSet<GroupRole>) -> Self {
        Self {
            pubkey: None,
            roles,
        }
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
            if let Some(pubkey) = invite.pubkey {
                write!(f, "pubkey: \"{}\", ", pubkey)?;
            }
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
        if event.kind != KIND_GROUP_CREATE {
            return Err(Error::notice(&format!(
                "Invalid event kind for group creation {}",
                event.kind
            )));
        }

        let group_id = event
            .tags
            .find(TagKind::h())
            .and_then(|t| t.content())
            .ok_or(Error::notice("Group ID not found"))?;

        let member = GroupMember::new_admin(event.pubkey);
        let members = HashMap::from([(event.pubkey, member)]);

        let mut group = Self {
            id: group_id.to_string(),
            metadata: GroupMetadata::new(group_id.to_string()),
            members,
            join_requests: HashSet::new(),
            invites: HashMap::new(),
            roles: HashSet::new(),
            created_at: event.created_at,
            updated_at: event.created_at,
        };

        group.update_state();
        Ok(group)
    }

    pub fn add_members(&mut self, members_event: &Event) -> Result<bool, Error> {
        if !self.can_edit_members(&members_event.pubkey) {
            error!(
                "User {} is not authorized to add users to this group",
                members_event.pubkey
            );
            return Err(Error::notice(
                "User is not authorized to add users to this group",
            ));
        }

        let mut added_admins = false;
        for new_member in members_event.tags.filter(TagKind::p()) {
            let member = GroupMember::try_from(new_member)?;
            added_admins |= member.is(GroupRole::Admin);
            self.join_requests.remove(&member.pubkey);
            self.members.entry(member.pubkey).or_insert(member);
        }

        self.update_state();
        Ok(added_admins)
    }

    pub fn admin_pubkeys(&self) -> Vec<PublicKey> {
        self.members
            .values()
            .filter(|member| member.is(GroupRole::Admin))
            .map(|member| member.pubkey)
            .collect::<Vec<_>>()
    }

    pub fn remove_members(&mut self, members_event: &Event) -> Result<bool, Error> {
        if !self.can_edit_members(&members_event.pubkey) {
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
        for removed_member in members_event.tags.filter(TagKind::p()) {
            let Some(removed_pubkey) = removed_member
                .content()
                .and_then(|s| PublicKey::parse(s).ok())
            else {
                return Err(Error::notice("Invalid tag format"));
            };

            if admins.len() == 1 && admins.contains(&removed_pubkey) {
                return Err(Error::notice("Cannot remove last admin"));
            }

            if let Some(removed_user) = self.members.remove(&removed_pubkey) {
                removed_admins |= removed_user.is(GroupRole::Admin);
            }
        }

        self.update_state();
        Ok(removed_admins)
    }

    pub fn set_metadata(&mut self, event: &Event) -> Result<(), Error> {
        if event.kind != KIND_GROUP_METADATA
            && event.kind != KIND_GROUP_CREATE
            && event.kind != KIND_GROUP_EDIT_METADATA
        {
            return Err(Error::notice(&format!(
                "Invalid event kind for group metadata {}",
                event.kind
            )));
        }

        if !self.can_edit_metadata(&event.pubkey) {
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

    pub fn set_roles(&mut self, event: &Event) -> Result<(), Error> {
        if !self.can_edit_metadata(&event.pubkey) {
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

        self.update_state();
        Ok(())
    }

    pub fn join_request(&mut self, event: &Event) -> Result<bool, Error> {
        if event.kind != KIND_GROUP_USER_JOIN_REQUEST {
            return Err(Error::notice(&format!(
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

        if invite.pubkey.is_some() {
            info!(
                "Invite already used, adding join request for {}",
                event.pubkey
            );
            self.join_requests.insert(event.pubkey);
            self.update_state();
            return Ok(false);
        }

        // Invite code matched and is available
        info!("Invite code matched, adding member {}", event.pubkey);
        invite.pubkey = Some(event.pubkey);
        let roles = invite.roles.clone();

        self.members
            .insert(event.pubkey, GroupMember::new(event.pubkey, roles));

        self.join_requests.remove(&event.pubkey);
        self.update_state();
        Ok(true)
    }

    pub fn create_invite(&mut self, event: &Event) -> Result<bool, Error> {
        if event.kind != KIND_GROUP_CREATE_INVITE {
            return Err(Error::notice(&format!(
                "Invalid event kind for create invite {}",
                event.kind
            )));
        }

        if !self.can_create_invites(&event.pubkey) {
            return Err(Error::notice("User is not authorized to create invites"));
        }

        let invite_code = event
            .tags
            .find(TagKind::custom("code"))
            .and_then(|t| t.content())
            .ok_or(Error::notice("Invite code not found"))?;

        let invite = Invite::new(HashSet::from([GroupRole::Member]));

        self.invites.insert(invite_code.to_string(), invite);
        self.update_state();
        Ok(true)
    }

    pub fn leave_request(&mut self, event: &Event) -> Result<bool, Error> {
        if event.kind != KIND_GROUP_USER_LEAVE_REQUEST {
            return Err(Error::notice(&format!(
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
            .map(|t| {
                let [_, pubkey, roles @ ..] = t.as_slice() else {
                    return None;
                };

                let pubkey = PublicKey::parse(pubkey).ok()?;
                Some((pubkey, roles))
            })
            .flatten()
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

            let invite = Invite {
                pubkey: None,
                roles,
            };

            self.invites.insert(code.to_string(), invite);
            self.updated_at = event.created_at;
        }
        Ok(())
    }

    // Real-time event handling methods - used for incoming events
    pub fn handle_join_request(&mut self, event: &Event) -> Result<bool, Error> {
        if event.kind != KIND_GROUP_USER_JOIN_REQUEST {
            return Err(Error::notice(&format!(
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

        if invite.pubkey.is_some() {
            info!(
                "Invite already used, adding join request for {}",
                event.pubkey
            );
            self.join_requests.insert(event.pubkey);
            self.update_state();
            return Ok(false);
        }

        // Invite code matched and is available
        info!("Invite code matched, adding member {}", event.pubkey);
        invite.pubkey = Some(event.pubkey);
        let roles = invite.roles.clone();

        self.members
            .entry(event.pubkey)
            .or_insert(GroupMember::new(event.pubkey, roles));

        self.join_requests.remove(&event.pubkey);
        self.update_state();
        Ok(true)
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
        event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::h() || t.kind() == TagKind::d())
            .and_then(|t| t.content())
    }

    pub fn extract_group_h_tag(event: &Event) -> Option<&str> {
        event.tags.find(TagKind::h()).and_then(|t| t.content())
    }

    pub fn verify_member_access(&self, pubkey: &PublicKey, event_kind: Kind) -> Result<(), Error> {
        if event_kind != KIND_GROUP_USER_JOIN_REQUEST && !self.is_member(pubkey) {
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
        EventBuilder::new(KIND_GROUP_ADD_USER, "")
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

        let mut metadata_event = EventBuilder::new(KIND_GROUP_METADATA, "")
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

        EventBuilder::new(KIND_GROUP_ADMINS, "").tags(tags)
    }

    pub fn generate_members_event(&self) -> EventBuilder {
        let members: Vec<&PublicKey> = self.members.keys().collect();

        let mut tags = Vec::new();
        tags.push(Tag::identifier(self.id.clone()));

        for pubkey in members {
            tags.push(Tag::public_key(*pubkey));
        }

        EventBuilder::new(KIND_GROUP_MEMBERS, "").tags(tags)
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

        EventBuilder::new(KIND_GROUP_ROLES, &content).tags(tags)
    }
}

// Authorization checks
impl Group {
    pub fn can_edit_members(&self, pubkey: &PublicKey) -> bool {
        self.is_admin(pubkey)
    }

    pub fn can_edit_metadata(&self, pubkey: &PublicKey) -> bool {
        self.is_admin(pubkey)
    }

    pub fn can_create_invites(&self, pubkey: &PublicKey) -> bool {
        self.is_admin(pubkey)
    }

    pub fn can_see_event(&self, pubkey: &Option<PublicKey>, event: &Event) -> bool {
        // Public groups are always visible
        if !self.metadata.private {
            return true;
        }

        // Private groups need authentication
        let Some(pubkey) = pubkey else { return false };

        // You can see your own events
        if *pubkey == event.pubkey {
            return true;
        }

        // Admins can see everything
        if self.is_admin(pubkey) {
            return true;
        } else {
            debug!(
                "User {} is not an admin, checking if they are a member to see event {}, group is {:?}",
                pubkey, event.id, self
            );
        }

        // Members can see everything except invites
        self.is_member(pubkey) && event.kind != KIND_GROUP_CREATE_INVITE
    }
}
