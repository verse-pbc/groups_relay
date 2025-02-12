# Groups Relay

A Nostr relay implementation specialized for group chat functionality.

## Project Structure

This is a Rust workspace containing two crates:

- **groups_relay**: The main relay server implementation with group chat functionality
- **websocket_builder**: A generic, middleware-based WebSocket framework

## Development

### Prerequisites

- Rust 1.75 or later
- Docker (optional, for containerized development)

### Building

Build all workspace crates:
```bash
cargo build --workspace
```

Run tests across all crates:
```bash
cargo test --workspace
```

### Running the Relay

From the workspace root:
```bash
cargo run -p groups_relay
```

## Key Features

- **NIP-29 Group Chat**: Implementation of decentralized groups including:
  - Public/private group visibility
  - Open/closed membership
  - Role-based permissions
  - Group metadata management
  - Event deletion
  - Invite system
- **NIP-42 Auth**: Client authentication handling
- **NIP-70 Moderation**: Content moderation tools
- **Middleware Architecture**: Modular implementation of NIPs through middleware components
- **State Management**: Storage and management of group memberships and events
- **LMDB Storage**: Persistent event and group data storage

## Quick Start

```bash
# Run with default config
cargo run

# Docker setup
docker compose up --build
```

Configure using command line args:
```bash
cargo run -- \
  --local-addr 0.0.0.0:8080 \
  --auth-url https://your-auth.com
```

## Core Components

Key middleware flow:
1. Connection authentication (NIP-42)
2. Message validation
3. Group membership checks (NIP-29)
4. Event storage
5. Content moderation (NIP-70)

## Development

Built using:
- `axum` for WebSocket handling
- `nostr-sdk` for Nostr protocol primitives
- `websocket_builder` middleware framework
- `lmdb` for persistent storage

Project structure:
- `/middlewares` - NIP implementations
- `/websocket_builder` - Core WebSocket framework
- `/groups` - Group state management

## License

AGPL
