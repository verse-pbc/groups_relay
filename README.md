# Groups Relay

[![codecov](https://codecov.io/gh/verse-pbc/groups_relay/branch/main/graph/badge.svg)](https://codecov.io/gh/verse-pbc/groups_relay)

A Nostr relay server specialized for group chat functionality.

Implements [NIP-29: Relay-based Groups](https://github.com/nostr-protocol/nips/blob/master/29.md).

## Project Structure

This is a Rust workspace with three main crates:

- **websocket_builder**: A low-level middleware-based WebSocket framework for building protocol servers.
- **nostr_relay_builder**: A framework for building custom Nostr relays with pluggable business logic.
- **groups_relay**: The main relay server implementing NIP-29 group chat functionality.

## Architecture

The project follows a layered architecture:

```
groups_relay (NIP-29 implementation)
    ↓ uses
nostr_relay_builder (Nostr protocol handling)
    ↓ uses
websocket_builder (WebSocket transport)
```

## Key Features

### groups_relay

- **NIP-29 Groups**:
  - Support for managed and unmanaged groups
  - Group metadata management
  - Member roles and permissions
  - Join requests and invitations
  
- **Built on nostr_relay_builder**:
  - Inherits all protocol support (NIPs 09, 40, 42, 70)
  - Custom `GroupsRelayProcessor` for group-specific rules

- **Management UI**: Preact-based frontend for group administration

### nostr_relay_builder

- **Pluggable event processing** via `EventProcessor` trait
- **Protocol middlewares**: NIPs 09, 40, 42, 70
- **Multi-tenant support** via subdomain isolation
- **Database abstraction** with LMDB backend

### websocket_builder

- **Middleware pipeline** for bidirectional message processing
- **Type-safe message conversion**
- **Connection state management**
- **Backpressure handling**

## Development

### Prerequisites

- Rust 1.84 or later
- Docker (optional)

### Quick Start

Build and test:
```bash
cargo build --workspace
cargo test --workspace
```

Run the relay:
```bash
cargo run -p groups_relay
```

Run with Docker:
```bash
docker compose up --build
```

## License

[AGPL](LICENSE)
