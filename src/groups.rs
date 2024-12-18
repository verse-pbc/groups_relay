pub mod group;

use crate::error::Error;
use anyhow::Result;
use dashmap::{
    mapref::one::{Ref, RefMut},
    DashMap,
};
pub use group::{
    Group, Invite, GROUP_CONTENT_KINDS, KIND_GROUP_ADD_USER, KIND_GROUP_CREATE,
    KIND_GROUP_CREATE_INVITE, KIND_GROUP_DELETE, KIND_GROUP_DELETE_EVENT, KIND_GROUP_EDIT_METADATA,
    KIND_GROUP_REMOVE_USER, KIND_GROUP_SET_ROLES, KIND_GROUP_USER_JOIN_REQUEST,
    KIND_GROUP_USER_LEAVE_REQUEST, METADATA_EVENT_KINDS,
};
use nostr_sdk::prelude::*;
use std::ops::{Deref, DerefMut};

#[derive(Debug)]
pub struct Groups {
    groups: DashMap<String, Group>,
}

impl Groups {
    pub async fn load_groups(client: &Client) -> Result<Self> {
        let groups = DashMap::new();
        Ok(Self { groups })
    }

    pub fn get_group_mut<'a>(&'a self, group_id: &str) -> Option<RefMut<'a, String, Group>> {
        self.groups.get_mut(group_id)
    }

    pub fn get_group<'a>(&'a self, group_id: &str) -> Result<Ref<'a, String, Group>, Error> {
        self.groups
            .get(group_id)
            .ok_or(Error::notice("Group not found"))
    }

    pub fn find_group_from_event_mut<'a>(
        &'a self,
        event: &Event,
    ) -> Result<Option<RefMut<'a, String, Group>>, Error> {
        let Some(group_id) = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::h() || t.kind() == TagKind::d())
            .and_then(|t| t.content())
        else {
            return Ok(None);
        };

        let group = self.get_group_mut(group_id);

        if let Some(group) = group {
            if event.kind != KIND_GROUP_USER_JOIN_REQUEST && !group.is_member(&event.pubkey) {
                return Err(Error::restricted(format!(
                    "User {} is not a member of this group",
                    event.pubkey
                )));
            }

            return Ok(Some(group));
        }

        Ok(None)
    }

    pub fn find_group_from_event<'a>(&'a self, event: &Event) -> Option<Ref<'a, String, Group>> {
        let group_id = event
            .tags
            .iter()
            .find(|t| t.kind() == TagKind::h() || t.kind() == TagKind::d())
            .and_then(|t| t.content())?;

        let group = self.get_group(group_id).ok();

        group
    }

    pub fn find_group_from_event_h_tag<'a>(
        &'a self,
        event: &Event,
    ) -> Option<Ref<'a, String, Group>> {
        let group_id = extract_group_h_tag(event)?;

        self.get_group(group_id).ok()
    }
}

// Helper function to extract group ID from event tags
fn extract_group_h_tag<'a>(event: &'a Event) -> Option<&'a str> {
    event.tags.find(TagKind::h()).and_then(|t| t.content())
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
