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

### 1. Update Invite Struct [✓]
```rust
pub struct Invite {
    pub event_id: EventId,
    pub roles: HashSet<GroupRole>,
    // New fields
    pub reusable: bool,
    pub redeemed_by: Option<(PublicKey, Timestamp)>,  // (who, when) if used
}
```

### 2. Update Invite Creation [✓]
- [✓] Modify `create_invite` to check for "reusable" tag
- [✓] Set `reusable` field based on tag presence
- [✓] Example event format:
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

### 3. Update Join Request Processing [✓]
- [✓] In `join_request`, when processing invite:
  - [✓] Check if invite exists and is valid (reusable or unused)
  - [✓] If invite doesn't exist yet -> add to join_requests (keep current behavior)
  - [✓] If invite exists but used -> add to join_requests (keep current behavior)
  - [✓] If invite valid -> auto-accept and mark as used with timestamp
- [✓] Add helper methods to make code more readable
  - [✓] `can_use()` - check if invite can be used
  - [✓] `mark_used()` - mark invite as used with metadata

### 4. Update State Loading [✓]
- [✓] Modify `load_invite_from_event`:
  - [✓] Parse reusable tag
  - [✓] Initialize new fields
  - [✓] Handle backward compatibility (existing invites are reusable)

### 5. Update Tests [✓]
- [✓] Add unit tests for:
  - [✓] Creating single-use invites
  - [✓] Creating reusable invites
  - [✓] Using single-use invite once
  - [✓] Attempting to reuse single-use invite (should go to join_requests)
  - [✓] Using reusable invite multiple times
  - [✓] State reconstruction with both types of invites
- [✓] Update integration tests to match new behavior

### 6. Discoveries and Notes
- Single-use invites are now the default, invites need an explicit "reusable" tag to be multi-use
- When an invite is used, we track both who used it and when it was used
- The integration tests in groups.rs had to be updated to account for the new behavior
- Some integration tests hung due to DashMap locking across await points; this has been resolved.