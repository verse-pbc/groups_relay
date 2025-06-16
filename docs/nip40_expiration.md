# NIP-40 Expiration Timestamp Implementation Plan

## Overview
Implement NIP-40 to allow Nostr events to include an expiration timestamp. Events with an `expiration` tag should be considered expired after the specified time by clients and relays. Relays should not serve expired events and should drop incoming events that are already expired upon receipt.

## Current Behavior
- Events do not have an expiration mechanism.
- Relays store and serve events indefinitely unless explicitly deleted (e.g., via NIP-09).

## Desired Behavior
- Events can include an optional `["expiration", "<unix_timestamp_seconds>"]` tag.
- The relay **must** drop incoming events if their expiration timestamp is in the past (`event.created_at < expiration`).
- The relay **must not** send expired events to clients requesting them via `REQ` messages.
- The relay **should** add `40` to its NIP-11 `supported_nips` list.
- The relay **may** delete expired events from storage, but this is not required by the spec and will not be implemented initially. Only filtering on read/write is required.

## NIP-40 Specification Recap
- **Tag:** `["expiration", "<unix_timestamp_seconds>"]`
- **Relay Behavior:**
    - SHOULD drop incoming expired events.
    - SHOULD NOT send expired events to clients.
    - MAY delete expired events (but not required).
- **Client Behavior:**
    - SHOULD check relay support via NIP-11.
    - SHOULD NOT send events with expiration to unsupported relays.
    - SHOULD ignore received events that are expired.

## Implementation Context

### Key Files and Functions
- `src/middlewares.rs`: Register the new middleware.
- `src/middlewares/nip_40_expiration.rs` (New file): Implement the core NIP-40 logic.
- `src/nostr_database.rs`: Potentially modify `query` function or add a new filtered query mechanism to exclude expired events.
- `src/subscription_manager.rs`: Ensure subscription handling filters out expired events before sending.
- `src/main.rs` (or wherever NIP-11 info is generated): Add `40` to supported NIPs.

### Event Processing Flow
1. Incoming `EVENT` messages pass through the middleware chain.
2. The `Nip40Middleware` will intercept `EVENT` messages.
3. If an event has an `expiration` tag and is expired, the middleware drops it and sends a `NOTICE` or `OK(false)`.
4. If the event is valid or has no expiration, it passes to the next middleware/handler for storage.
5. Incoming `REQ` messages trigger event fetching.
6. The query logic (likely in `RelayDatabase::query` or `SubscriptionManager`) must filter out events where `current_time >= expiration_timestamp`.

## Implementation Steps

### 1. Create `Nip40Middleware`
- Create `src/middlewares/nip_40_expiration.rs`.
- Define `Nip40Middleware` struct (might not need any fields initially).
- Implement the `Middleware` trait for `Nip40Middleware`.
- In `process_inbound`:
    - Check if the message is `ClientMessage::Event`.
    - Parse the `expiration` tag, if present.
    - If expired (`expiration_timestamp <= current_unix_timestamp`), log the drop, send `OK(false, "event is expired")` or similar, and stop processing (`return Ok(())`).
    - If not expired or no tag, call `ctx.next().await`.

### 2. Register Middleware
- Add `mod nip_40_expiration;` and `pub use nip_40_expiration::Nip40Middleware;` to `src/middlewares.rs`.
- Instantiate and add `Nip40Middleware` to the middleware chain where `App` or the websocket server is configured (e.g., in `main.rs`).

### 3. Filter Expired Events on Query
- Modify the event query logic (likely in `RelayDatabase::query` or how `SubscriptionManager` uses it).
- When fetching events based on filters, add an additional check:
    - If an event has an `expiration` tag, ensure `expiration_timestamp > current_unix_timestamp`.
    - Exclude events that fail this check from the results sent to clients.
- Consider performance implications, especially if scanning many events.

### 4. Update Supported NIPs
- Locate the code that generates the NIP-11 relay information document.
- Add `40` to the list of supported NIP integer identifiers.

### 5. Add Tests
- **Unit Tests (`nip_40_expiration.rs`):**
    - Test `process_inbound` with:
        - Event with no expiration tag (should pass).
        - Event with future expiration tag (should pass).
        - Event with past expiration tag (should be dropped, return `OK(false)`).
        - Non-event message (should pass).
- **Integration Tests (e.g., in `groups.rs` or a dedicated NIP-40 test file):**
    - Test sending a future-expiring event (should be stored).
    - Test sending a past-expiring event (should be rejected/dropped).
    - Test subscribing (`REQ`) and verifying that:
        - Non-expired events are received.
        - Events that *were* valid but have *now* expired are *not* received.
        - Events created with past expiration are never received.
    - Test NIP-11 response includes `40`.

## Discoveries and Notes
- Initial implementation focuses on filtering, not physical deletion from storage.
- Filtering outgoing events (`REQ`) needs careful implementation in the query/subscription path, potentially outside the middleware itself.
- Ensure correct handling of timestamp parsing and comparison.
- Need to decide on the exact response for dropped expired events (`OK false` vs `NOTICE`). `OK false` seems more appropriate as the event itself is invalid *now*.