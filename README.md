# Groups Relay

[![codecov](https://codecov.io/gh/verse-pbc/groups_relay/branch/main/graph/badge.svg)](https://codecov.io/gh/verse-pbc/groups_relay)

A Nostr relay server specialized for group chat functionality.

Implements [NIP-29: Relay-based Groups](https://github.com/nostr-protocol/nips/blob/master/29.md).

## Architecture

Groups Relay is built on top of modular Rust libraries:

```
groups_relay (NIP-29 implementation)
    ↓ depends on
nostr_relay_builder (Nostr protocol handling)
    ↓ depends on  
websocket_builder (WebSocket transport)
```

**Dependencies:**
- [nostr_relay_builder](https://github.com/verse-pbc/nostr_relay_builder) - Nostr relay framework
- [websocket_builder](https://github.com/verse-pbc/websocket_builder) - WebSocket middleware framework

## Features

- **NIP-29 Groups**: Managed and unmanaged groups with metadata, roles, and permissions
- **Join System**: Join requests and invite codes with role-based access control
- **Protocol Support**: Inherits NIPs 09 (deletion), 40 (expiration), 42 (auth), 70 (protected events)
- **Management UI**: Preact-based frontend for group administration
- **Multi-tenant**: Subdomain-based data isolation

## Development

### Prerequisites

- Rust 1.86 or later
- Node.js 20+ (for frontend)
- Docker (optional)

### Quick Start

Build and test:
```bash
cargo build
cargo test
```

Run the relay:
```bash
cargo run
```

Run with Docker:
```bash
docker compose up --build
```

### Frontend Development

```bash
cd frontend
npm install
npm run dev
```

## Configuration

Default configuration in `config/settings.yml`. Environment-specific overrides supported via `config/settings.local.yml`.

## License

[AGPL](LICENSE)