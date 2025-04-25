# Single-Use Invites Implementation Plan

## Overview
Add support for single-use invites in the groups system as an extension to NIP-29. While NIP-29 specifies invite codes for closed groups, it doesn't define reusability behavior. This implementation extends the spec with single-use functionality.

## Current Behavior
- All invites are reusable by default (current implementation choice, not specified by NIP-29)
- No tracking of who used an invite
- No way to disable an invite after use

## Desired Behavior
- Invites are single-use by default (extension to NIP-29)
- Invites can be marked as reusable with a "reusable" tag (extension to NIP-29)
- Track who redeemed each invite and when
- Automatically disable single-use invites after first use

## NIP-29 Compliance
This implementation:
1. Maintains compliance with NIP-29's invite code usage in `kind:9021` events
2. Extends `kind:9009` create-invite event with optional reusability
3. Ensures full state reconstruction from event sequence
4. Preserves existing closed group behavior

### Extended Event Formats
- `kind:9009` (create-invite) extension:
```json
{
  "kind": 9009,
  "content": "",
  "tags": [
    ["h", "<group-id>"],
    ["code", "<invite-code>"],
    ["reusable"]  // Optional extension to NIP-29
  ]
}
```

## Implementation Context

### Key Files and Functions
- `crates/groups_relay/src/groups/group.rs`:
  - `join_request`: Processes join requests and invite validation
  - `create_invite`: Creates new invite codes
  - `load_invite_from_event`: Reconstructs invite state from events
  - `Group` struct: Contains invite and membership state

### Event Processing Flow
1. Events arrive and are processed in real-time order by the filter in `@groups.rs`
2. For join requests:
   - `join_request` function checks for matching invite code
   - If invite exists and is valid -> auto-accept member
   - If invite doesn't exist or is used -> add to join_requests
3. For invite creation:
   - `create_invite` validates and stores new invites
   - Generates KIND_GROUP_CREATE_INVITE_9009 event

### State Management
- Group state is stored in LMDB database
- Events needed for reconstruction:
  - KIND_GROUP_CREATE_INVITE_9009: Invite creation
  - KIND_GROUP_USER_JOIN_REQUEST_9021: Join requests
  - KIND_GROUP_ADD_USER_9000: Member additions
- State is rebuilt on relay restart using `load_invite_from_event`

## Implementation Steps

### 1. Update Invite Struct [ ]
```rust
pub struct Invite {
    pub event_id: EventId,
    pub roles: HashSet<GroupRole>,
    // New fields
    pub reusable: bool,
    pub redeemed_by: Option<(PublicKey, Timestamp)>,  // (who, when) if used
}
```

### 2. Update Invite Creation [ ]
- [ ] Modify `create_invite` to check for "reusable" tag
- [ ] Set `reusable` field based on tag presence
- [ ] Example event format:
```json
{
  "kind": 9009,
  "content": "",
  "tags": [
    ["h", "<group-id>"],
    ["code", "<invite-code>"],
    ["reusable"]  // Optional tag
  ]
}
```

### 3. Update Join Request Processing [ ]
- [ ] In `join_request`, when processing invite:
  - [ ] Check if invite exists and is valid (reusable or unused)
  - [ ] If invite doesn't exist yet -> add to join_requests (keep current behavior)
  - [ ] If invite exists but used -> add to join_requests (keep current behavior)
  - [ ] If invite valid -> auto-accept and mark as used with timestamp
- [ ] Update error handling:
  ```rust
  match code {
      Some(invite) if !invite.reusable && invite.redeemed_by.is_some() => {
          // Add to join_requests for manual approval
          self.join_requests.insert(event.pubkey);
          return self.create_join_request_commands(false, event, relay_pubkey);
      }
      Some(invite) => {
          // Valid invite, mark as used if not reusable
          if !invite.reusable {
              invite.redeemed_by = Some((event.pubkey, event.created_at));
          }
          // Add member and proceed with auto-accept
          ...
      }
      None => {
          // No matching invite, add to join_requests
          self.join_requests.insert(event.pubkey);
          return self.create_join_request_commands(false, event, relay_pubkey);
      }
  }
  ```

### 4. Update State Loading [ ]
- [ ] Modify `load_invite_from_event`:
  - [ ] Parse reusable tag
  - [ ] Initialize new fields
  - [ ] Handle backward compatibility (existing invites are reusable)
- [ ] No changes needed to `load_join_request_from_event` as join requests are processed immediately based on current invite state

### 5. Update Group State Events [ ]
- [ ] Update invite-related events to include:
  - [ ] Reusable flag
  - [ ] Redemption status (who and when)
- [ ] Ensure state can be fully reconstructed from events

### 6. Testing [ ]
Add tests for:
- [ ] Creating single-use invites
- [ ] Creating reusable invites
- [ ] Using single-use invite once
- [ ] Attempting to reuse single-use invite (should go to join_requests)
- [ ] Using reusable invite multiple times
- [ ] Join request with non-existent invite code (should go to join_requests)
- [ ] Join request with used invite code (should go to join_requests)
- [ ] State reconstruction with both types of invites
- [ ] Documentation [ ]
  - [ ] Update code documentation
  - [ ] Update NIP-29 documentation if needed
  - [ ] Add examples to README

### 7. Integration Points [ ]
- [ ] Update event filter in `@groups.rs` to handle single-use invite logic
- [ ] Modify group state commands in `StoreCommand` enum if needed
- [ ] Update any relevant NIP-29 event validation
- [ ] Consider impacts on:
  - Group membership checks
  - Event visibility rules

## Technical Considerations

### NIP-29 Compatibility
- No changes needed to NIP-29 event kinds or basic structure
- New behavior applies to all invites
- Only extends the spec with optional reusability tag

### State Management
- Invite state (including usage tracking) is maintained in memory in the Group struct
- State is rebuilt from events on relay restart via `load_groups`
- No persistence needed beyond the existing event storage
- Events are processed in order received, no special ordering needed
- Join requests with unknown invite codes default to manual approval

### Error Handling
New error cases to handle:
- Invite already used (handle by adding to join_requests)
- Invalid/unknown invite code (handle by adding to join_requests)
- Keep current error handling for other cases

### Event Processing
- Events processed by filter in `@groups.rs`
- Join request processing sequence:
  1. Event received by filter
  2. `join_request` function called
  3. Invite validation performed
  4. Member added or join request created
  5. State events generated and saved

## Progress Tracking
- Copy this file when starting work
- Use checkboxes to track progress
- Add notes under each section as needed
- Document any deviations from plan

## Notes
- Start Date: [Insert when work begins]
- Current Status: Planning
- Last Updated: [Current Date]

## Related Issues/PRs
- [Add links when created]