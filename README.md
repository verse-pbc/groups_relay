# Nostr Group Relay

[![codecov](https://codecov.io/gh/verse-pbc/groups_relay/branch/main/graph/badge.svg)](https://codecov.io/gh/verse-pbc/groups_relay)

A Nostr relay implementation with NIP-29 group chat support, built using a middleware-based architecture and LMDB for storage.

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

MIT