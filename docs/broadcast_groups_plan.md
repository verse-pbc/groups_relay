# Plan: Implement Broadcast-Only NIP-29 Groups

This plan outlines the steps required to implement a feature allowing NIP-29 groups to operate in a broadcast-only mode, where only Admins can publish standard content events. We will start by writing a failing test.

## Tasks

-   [x] **1. Add Test for Broadcast Mode Publishing Restrictions:**
    -   Location: `mod tests` in `src/groups/group.rs`.
    -   Create a new test function (e.g., `test_broadcast_mode_restrictions`).
    -   Scenario 1: Broadcast Mode Enabled
        -   Setup: Create a group, add a member. Update metadata via `kind:39000` or `kind:9002` to include the `["broadcast"]` tag.
        -   Test Admin Publishing: Create a standard event (e.g., `Kind::TextNote`) signed by the Admin. Assert that publishing succeeds.
        -   Test Member Publishing (Blocked): Create a standard event signed by the Member. Assert that publishing fails with `Error::restricted`.
        -   Test Member Publishing (Allowed): Create `kind:9021` (Join) and `kind:9022` (Leave) events signed by the Member. Assert that publishing succeeds for these specific kinds.
    -   Scenario 2: Broadcast Mode Disabled (Default)
        -   Setup: Create a group and add a member, ensuring the `["broadcast"]` tag is *not* present in metadata.
        -   Test Member Publishing: Create a standard event signed by the Member. Assert that publishing succeeds.
    -   Scenario 3: Disabling Broadcast Mode
        -   Setup: Create a group, enable broadcast mode via `kind:9002` with the `["broadcast"]` tag. Then send another `kind:9002` *without* the `["broadcast"]` tag.
        -   Test Member Publishing: Create a standard event signed by the Member. Assert that publishing now succeeds.

-   [x] **2. Modify `GroupMetadata` Struct:**
    -   Location: `src/groups/group.rs`
    -   Add a new boolean field, `is_broadcast: bool`, initialized to `false`.
-   [x] **3. Update Metadata Parsing:**
    -   `Group::load_metadata_from_event` (for `kind:39000`): Modify parsing to look for the `["broadcast"]` tag and set `is_broadcast` accordingly.
    -   `Group::set_metadata` (for `kind:9002`): Modify parsing. Check for the presence of the `["broadcast"]` tag *within the incoming event*. If present, set `self.metadata.is_broadcast = true`. If *absent*, explicitly set `self.metadata.is_broadcast = false`.
-   [x] **4. Update Metadata Generation:**
    -   Location: `Group::generate_metadata_event` in `src/groups/group.rs`.
    -   Modify the event generation to include the `["broadcast"]` tag if `self.metadata.is_broadcast` is `true`.
-   [x] **5. Enforce Publishing Restrictions:**
    -   Location: `Group::handle_group_content` (or identified validation point).
    -   Add logic: `if self.metadata.is_broadcast && !self.is_admin(&event.pubkey) && ![KIND_GROUP_USER_JOIN_REQUEST_9021, KIND_GROUP_USER_LEAVE_REQUEST_9022].contains(&event.kind)`.
    -   If the condition is true, reject the event with `Error::restricted`.
-   [x] **6. Update Existing Tests (If Necessary):**
    -   Review existing tests for regressions.