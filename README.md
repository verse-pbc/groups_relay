# Groups Relay

[![codecov](https://codecov.io/gh/verse-pbc/groups_relay/branch/main/graph/badge.svg)](https://codecov.io/gh/verse-pbc/groups_relay)

A Nostr relay server specialized for group chat functionality.

Implements [NIP-29: Relay-based Groups](https://github.com/nostr-protocol/nips/blob/master/29.md).

## Project Structure

This is a Rust workspace with these main crates:

- **groups_relay**: The main relay server implementing NIP-29 group chat functionality.
- **websocket_builder**: A middleware-based WebSocket framework.

## Key Features

### groups_relay

- **Groups**:
  - Support for managed and unmanaged groups
  - Group metadata management
  - Event metrics and monitoring

- **NIP-42 Auth**: Client authentication
- **NIP-70**: Protected events
- **Management UI**: Preact-based frontend for group administration

### websocket_builder

- Middleware pipeline for message processing
- Type-safe message conversion
- Connection state management
- Configurable channel sizing

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
