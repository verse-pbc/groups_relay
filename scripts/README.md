# Groups Relay Test Scripts

This directory contains test scripts for the groups_relay implementation of NIP-29 (Relay-based Groups).

## Main Test Script

### `flow29_test.sh`
The unified comprehensive test script that covers all NIP-29 functionality with 28 test steps.

**Usage:**
```bash
# Interactive mode (press Enter between steps)
./flow29_test.sh manual

# Automated mode (runs without pausing)
./flow29_test.sh auto

# Server-only mode (for manual testing)
./flow29_test.sh server-only

# Test-only mode (assumes server is already running)
./flow29_test.sh test-only
```

**Features tested:**
- Group creation and state management (kinds 9007, 39000-39003)
- Metadata editing with public/private and open/closed settings
- Invite codes and join requests
- Role-based permissions (admin, moderator, member, custom roles)
- Timeline references
- Multiple event kinds (chat messages, articles)
- Moderation actions with deletion verification
- User leaving with access verification
- Late publication prevention
- Group deletion with verification

## Helper Scripts

### `run_nip29_test.sh`
Wrapper script for running tests with AI analysis. This script:
1. Starts the server
2. Runs the comprehensive test
3. Outputs formatted logs for Claude analysis

**Usage with Claude:**
```bash
./run_nip29_test.sh | claude /project:analyze-nip29
# or via just:
just test-nip29-analyze | claude /project:analyze-nip29
```

## Just Commands

The easiest way to run tests is using the `just` command runner:

```bash
# Run comprehensive test interactively
just test-nip29

# Run comprehensive test automatically
just test-nip29-auto

# Run test with AI analysis
just test-nip29-analyze | claude /project:analyze-nip29
```

## Other Scripts

- `groups_relay_performance_test.sh` - Performance testing script
- `run_flamegraph.sh` - Generate flamegraphs for performance analysis
- `tag_latest_as_stable.sh` - Docker image tagging utility

## Test Data

- `example_prd.txt` - Example Product Requirements Document
- `nostr_lmdb_scoped_api_prd.txt` - PRD for scoped LMDB API

## Troubleshooting

### Test Hangs
If the test hangs, it's likely due to `nak` detecting it's not running in a terminal:
1. The unified test script handles this with proper timeout settings
2. Try running in manual mode: `just test-nip29`
3. Check if the server is running properly

### Server Won't Start
- Check if port 8080 is already in use
- Ensure you have the correct configuration in `crates/groups_relay/config/`
- Check server logs: `test_server.log`

### Authentication Errors
Initial authentication errors are normal - `nak` automatically retries with NIP-42 authentication after the initial connection.

### Missing Dependencies
- Install `nak`: `cargo install nak`
- Install `jq`: Required for JSON parsing in tests