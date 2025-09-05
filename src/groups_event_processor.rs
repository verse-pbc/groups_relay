use crate::groups::{
    Group, ADDRESSABLE_EVENT_KINDS, KIND_GROUP_ADD_USER_9000, KIND_GROUP_CREATE_9007,
    KIND_GROUP_CREATE_INVITE_9009, KIND_GROUP_DELETE_9008, KIND_GROUP_DELETE_EVENT_9005,
    KIND_GROUP_EDIT_METADATA_9002, KIND_GROUP_REMOVE_USER_9001, KIND_GROUP_SET_ROLES_9006,
    KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022, NON_GROUP_ALLOWED_KINDS,
};
use crate::Groups;
use nostr_sdk::prelude::*;
use parking_lot::RwLock;
use relay_builder::{EventContext, EventProcessor, Result, StoreCommand};
use std::sync::Arc;
use tracing::debug;

/// Groups event processor implementing NIP-29 (Relay-based Groups) functionality.
///
/// This implementation provides all the business logic for managing groups, including:
/// - Group creation and management
/// - Member access control and permissions
/// - Group content validation and storage
/// - Deletion and moderation events
/// - Unmanaged group support
///
/// The processor is extracted from the original Nip29Middleware to enable reusability
/// and better testability while maintaining identical functionality.
#[derive(Debug, Clone)]
pub struct GroupsRelayProcessor {
    groups: Arc<Groups>,
    relay_pubkey: PublicKey,
}

impl GroupsRelayProcessor {
    /// Create a new groups event processor instance.
    ///
    /// # Arguments
    /// * `groups` - The groups state manager for this relay
    /// * `relay_pubkey` - The relay's public key
    pub fn new(groups: Arc<Groups>, relay_pubkey: PublicKey) -> Self {
        Self {
            groups,
            relay_pubkey,
        }
    }

    /// Get a reference to the groups state manager
    pub fn groups(&self) -> &Arc<Groups> {
        &self.groups
    }

    /// Get the relay public key
    pub fn relay_pubkey(&self) -> &PublicKey {
        &self.relay_pubkey
    }

    /// Checks if a filter is querying group-related data
    fn is_group_query(&self, filter: &Filter) -> bool {
        filter
            .generic_tags
            .contains_key(&SingleLetterTag::lowercase(Alphabet::H))
            || filter
                .generic_tags
                .contains_key(&SingleLetterTag::lowercase(Alphabet::D))
    }

    /// Checks if a filter is querying addressable event kinds
    fn is_addressable_query(&self, filter: &Filter) -> bool {
        filter
            .kinds
            .as_ref()
            .is_some_and(|kinds| kinds.iter().any(|k| ADDRESSABLE_EVENT_KINDS.contains(k)))
    }

    /// Gets all group tags from a filter
    fn get_group_tags<'a>(&self, filter: &'a Filter) -> impl Iterator<Item = String> + 'a {
        filter
            .generic_tags
            .iter()
            .filter(|(k, _)| k == &&SingleLetterTag::lowercase(Alphabet::H))
            .flat_map(|(_, tag_set)| tag_set.iter())
            .cloned()
    }
}

impl EventProcessor for GroupsRelayProcessor {
    fn verify_filters(
        &self,
        filters: &[Filter],
        _custom_state: Arc<RwLock<()>>,
        context: &EventContext,
    ) -> Result<()> {
        // For groups relay, we need to verify access to group queries
        for filter in filters {
            // Check if this filter queries group-related data
            if self.is_group_query(filter) {
                // Get all group tags from the filter
                let group_tags: Vec<String> = self.get_group_tags(filter).collect();

                // Verify access to each group mentioned in the filter
                for group_tag in group_tags {
                    if let Some(group_ref) = self.groups.get_group(&context.subdomain, &group_tag) {
                        // Managed group - check if the user can read from this group
                        let group = group_ref.value();
                        if group.metadata.private {
                            // Private group - user must be a member or relay admin
                            if let Some(pubkey) = &context.authed_pubkey {
                                // Relay admin has access to all groups
                                if pubkey != &self.relay_pubkey && !group.is_member(pubkey) {
                                    return Err(relay_builder::Error::restricted(format!(
                                        "Access denied to private group: {group_tag}"
                                    )));
                                }
                            } else {
                                return Err(relay_builder::Error::auth_required(format!(
                                    "Authentication required to access private group: {group_tag}"
                                )));
                            }
                        }
                        // Public groups allow everyone to read
                    }
                    // Unmanaged groups are allowed (everyone can read from them)
                }
            }

            // For addressable events, verify the user can access the groups they reference
            if self.is_addressable_query(filter) {
                // Addressable events might reference groups in their identifiers
                // For now, we'll allow these queries and rely on visibility filtering
                // during event delivery to handle access control
            }
        }

        Ok(())
    }

    fn can_see_event(
        &self,
        event: &Event,
        _custom_state: Arc<RwLock<()>>,
        context: &EventContext,
    ) -> Result<bool> {
        // Check if this is a group event
        if let Some(group_ref) = self.groups.find_group_from_event(event, &context.subdomain) {
            // Group event - check access control using the group's can_see_event method
            group_ref.value().can_see_event(
                &context.authed_pubkey,
                &context.relay_pubkey,
                event,
            )
        } else {
            // Not a group event or unmanaged group - allow it through
            Ok(true)
        }
    }

    async fn handle_event(
        &self,
        event: Event,
        _custom_state: Arc<RwLock<()>>,
        context: &EventContext,
    ) -> Result<Vec<StoreCommand>> {
        let subdomain = context.subdomain.clone();

        // Allow events through for unmanaged groups (groups not in relay state)
        // Per NIP-29: In unmanaged groups, everyone is considered a member
        // These groups can later be converted to managed groups by the relay admin
        if event.tags.find(TagKind::h()).is_some()
            && !Group::is_group_management_kind(event.kind)
            && self
                .groups
                .find_group_from_event(&event, &subdomain)
                .is_none()
        {
            debug!(target: "groups_relay_logic", "Processing unmanaged group event: kind={}, id={}", event.kind, event.id);
            return Ok(vec![StoreCommand::SaveSignedEvent(
                Box::new(event),
                (*subdomain).clone(),
                None,
            )]);
        }

        let events_to_save = match event.kind {
            k if k == KIND_GROUP_CREATE_9007 => {
                debug!(target: "groups_relay_logic", "Processing group create event: id={}", event.id);
                let commands = self.groups
                    .handle_group_create(Box::new(event), &subdomain)
                    .await?;
                debug!(target: "groups_relay_logic", "Group create generated {} commands", commands.len());
                for cmd in &commands {
                    match cmd {
                        StoreCommand::SaveSignedEvent(_, _, _) => {
                            debug!(target: "groups_relay_logic", "  - SaveSignedEvent");
                        }
                        StoreCommand::SaveUnsignedEvent(evt, _, _) => {
                            debug!(target: "groups_relay_logic", "  - SaveUnsignedEvent: kind={}", evt.kind);
                        }
                        StoreCommand::DeleteEvents(_, _, _) => {
                            debug!(target: "groups_relay_logic", "  - DeleteEvents");
                        }
                    }
                }
                commands
            }

            k if k == KIND_GROUP_EDIT_METADATA_9002 => {
                debug!(target: "groups_relay_logic", "Processing group edit metadata event: id={}", event.id);
                self.groups
                    .handle_edit_metadata(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_USER_JOIN_REQUEST_9021 => {
                debug!(target: "groups_relay_logic", "Processing group join request: id={}", event.id);
                self.groups
                    .handle_join_request(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_USER_LEAVE_REQUEST_9022 => {
                debug!(target: "groups_relay_logic", "Processing group leave request: id={}", event.id);
                self.groups
                    .handle_leave_request(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_SET_ROLES_9006 => {
                debug!(target: "groups_relay_logic", "Processing group set roles event: id={}", event.id);
                self.groups.handle_set_roles(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_ADD_USER_9000 => {
                debug!(target: "groups_relay_logic", "Processing group add user event: id={}", event.id);
                self.groups.handle_put_user(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_REMOVE_USER_9001 => {
                debug!(target: "groups_relay_logic", "Processing group remove user event: id={}", event.id);
                self.groups
                    .handle_remove_user(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_DELETE_9008 => {
                debug!(target: "groups_relay_logic", "Processing group deletion event: id={}", event.id);
                self.groups
                    .handle_delete_group(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_DELETE_EVENT_9005 => {
                debug!(target: "groups_relay_logic", "Processing group content event deletion: id={}", event.id);
                self.groups
                    .handle_delete_event(Box::new(event), &subdomain)?
            }

            k if k == KIND_GROUP_CREATE_INVITE_9009 => {
                debug!(target: "groups_relay_logic", "Processing group create invite event: id={}", event.id);
                self.groups
                    .handle_create_invite(Box::new(event), &subdomain)?
            }

            k if !NON_GROUP_ALLOWED_KINDS.contains(&k)
                && event.tags.find(TagKind::h()).is_some() =>
            {
                debug!(target: "groups_relay_logic", "Processing group content event: kind={}, id={}", event.kind, event.id);
                self.groups
                    .handle_group_content(Box::new(event), &subdomain)?
            }

            _ => {
                debug!(target: "groups_relay_logic", "Processing non-group event: kind={}, id={}", event.kind, event.id);
                vec![StoreCommand::SaveSignedEvent(
                    Box::new(event),
                    (*subdomain).clone(),
                    None,
                )]
            }
        };

        debug!(target: "groups_relay_logic", "Returning {} store commands from handle_event", events_to_save.len());
        Ok(events_to_save)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{create_test_event, create_test_keys, setup_test};
    use nostr_lmdb::Scope;

    fn empty_state() -> Arc<RwLock<()>> {
        Arc::new(RwLock::new(()))
    }

    #[tokio::test]
    async fn test_groups_relay_logic_creation() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                admin_keys.public_key(),
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let processor = GroupsRelayProcessor::new(groups.clone(), admin_keys.public_key());

        // Verify the logic was created correctly
        assert_eq!(processor.relay_pubkey(), &admin_keys.public_key());
        assert!(Arc::ptr_eq(processor.groups(), &groups));
    }

    #[tokio::test]
    async fn test_groups_relay_logic_non_group_event_visibility() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                admin_keys.public_key(),
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let processor = GroupsRelayProcessor::new(groups, admin_keys.public_key());
        let (_admin_keys, member_keys, _non_member_keys) = create_test_keys().await;

        // Create a non-group event (no 'h' tag)
        let event = create_test_event(&member_keys, 1, vec![]).await;

        let member_pubkey = member_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(member_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey: admin_keys.public_key(),
        };

        // Non-group events should be visible to everyone
        assert!(processor
            .can_see_event(&event, empty_state(), &context)
            .unwrap());
    }

    #[tokio::test]
    async fn test_groups_relay_logic_unmanaged_group_event() {
        let (_tmp_dir, database, admin_keys) = setup_test().await;
        let groups = Arc::new(
            Groups::load_groups(
                database.clone(),
                admin_keys.public_key(),
                "wss://test.relay.com".to_string(),
            )
            .await
            .unwrap(),
        );

        let processor = GroupsRelayProcessor::new(groups, admin_keys.public_key());
        let (_admin_keys, member_keys, _non_member_keys) = create_test_keys().await;

        // Create an unmanaged group event (has 'h' tag but group doesn't exist)
        let event = create_test_event(
            &member_keys,
            11, // Group content event
            vec![Tag::custom(TagKind::h(), ["unmanaged_group"])],
        )
        .await;

        let member_pubkey = member_keys.public_key();
        let context = EventContext {
            authed_pubkey: Some(member_pubkey),
            subdomain: Arc::new(Scope::Default),
            relay_pubkey: admin_keys.public_key(),
        };

        // Unmanaged group events should be visible (everyone is considered a member)
        assert!(processor
            .can_see_event(&event, empty_state(), &context)
            .unwrap());

        // Test handle_event for unmanaged group
        let commands = processor
            .handle_event(event.clone(), empty_state(), &context)
            .await
            .unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            StoreCommand::SaveSignedEvent(saved_event, scope, _) => {
                assert_eq!(saved_event.id, event.id);
                assert_eq!(*scope, Scope::Default);
            }
            _ => panic!("Expected SaveSignedEvent command"),
        }
    }
}
