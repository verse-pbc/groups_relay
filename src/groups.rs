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
