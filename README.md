# Groups Relay

[![codecov](https://codecov.io/gh/verse-pbc/groups_relay/branch/main/graph/badge.svg)](https://codecov.io/gh/verse-pbc/groups_relay)

[NIP-29: Relay-based Groups](https://github.com/nostr-protocol/nips/blob/master/29.md) implementation.

## Implementation Status

- ✅ All event kinds (9000-9009, 9021-9022, 39000-39003)  
- ✅ Group types (public/private, open/closed, broadcast)
- ✅ Moderation actions and role-based permissions
- ✅ Join requests and invite codes
- ❌ Timeline references (not implemented)

Also supports NIPs 09, 40, 42, 70.

## Quick Start

```bash
cargo run
# or
docker compose up --build
```

Web UI at `http://localhost:8080`

## Development

```bash
cargo test
cargo fmt
cargo clippy
```

Built on [relay_builder](https://github.com/verse-pbc/relay_builder) and [websocket_builder](https://github.com/verse-pbc/websocket_builder).

## Included Utilities

The Docker image includes these utility binaries from relay_builder:

- **export_import** - Export/import relay data
- **negentropy_sync** - Relay-to-relay synchronization using Negentropy protocol
- **nostr-lmdb-dump** - Dump LMDB database contents
- **nostr-lmdb-integrity** - Check LMDB database integrity

## License

[AGPL](LICENSE)