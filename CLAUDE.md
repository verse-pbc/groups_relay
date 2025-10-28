# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Repository Overview

A NIP-29 relay implementation with group/chat functionality, built on the relay_builder framework. Features a Preact-based web UI with Cashu wallet integration.

## Architecture

### Core Components
- **Groups System** (`src/groups.rs`, `src/group.rs`) - Manages NIP-29 groups with role-based permissions
- **Validation Middleware** (`src/validation_middleware.rs`) - Groups-specific event validation
- **Groups Event Processor** (`src/groups_event_processor.rs`) - Handles group-related event processing
- **Database** - Uses nostr-lmdb with scoped storage for multi-tenant support
- **Frontend** - Preact app with NDK wallet integration for Cashu payments

### Key Event Kinds
- `9000-9009`: Group management events (add/remove users, metadata, roles)
- `9021-9022`: Join/leave requests
- `39000-39003`: Group metadata (replaceable events)
- `10009`: Simple lists

## Development Commands

### Testing
```bash
# Run all tests (unit + integration)
just test

# Run specific test by name
just test-name <test_name>

# NIP-29 flow test (interactive)
just test-nip29

# NIP-29 flow test (automated)
just test-nip29-auto

# Test coverage with HTML report
just coverage
```

### Building & Running
```bash
# Run in debug mode
just run

# Run with debug logging
just run-debug

# Build release version
just build-release

# Run benchmarks
just bench
```

### Code Quality
```bash
# Format code
just fmt

# Run clippy linter
just clippy

# Run all checks (format, clippy, tests)
just check
```

### Frontend Development
```bash
cd frontend
npm install
npm run dev     # Development server with hot reload
npm run build   # Production build
```

## Testing Individual Components

```bash
# Test specific middleware
cargo test --test validation_middleware_test

# Test with debug output
cargo test <test_name> -- --nocapture

# Test with specific log level
RUST_LOG=debug cargo test <test_name> -- --nocapture

# Run integration tests only
cargo nextest run --test '*' --all-features
```

## Configuration

The relay uses a config directory (default: `config/`) with TOML files:
- `relay_url`: WebSocket URL for the relay
- `local_addr`: Local address to bind
- `db_path`: Database directory path
- `max_limit`: Maximum event limit per subscription
- `max_subscriptions`: Maximum concurrent subscriptions

Override with CLI args:
```bash
cargo run -- --config-dir config --relay-url ws://localhost:8080 --local_addr 127.0.0.1:8080
```

## Performance Profiling

```bash
# Run benchmarks
cargo bench --workspace

# Generate flamegraph (requires cargo-flamegraph)
./scripts/run_flamegraph.sh

# Performance test script
./scripts/groups_relay_performance_test.sh
```

## Docker Support

```bash
# Build Docker image
docker build -t groups_relay .

# Run with docker-compose
docker compose up --build

# Tag and push images
./scripts/tag_image.sh <version>
./scripts/tag_latest_as_stable.sh
```

## Debugging Tips

### WebSocket Issues
- Enable debug logging: `RUST_LOG=debug,groups_relay=trace`
- Use `./scripts/diagnose_network.sh` for network diagnostics
- Check browser console for connection errors

### NIP-29 Flow Testing
- Use `./scripts/flow29.sh` for comprehensive group operation testing
- Interactive mode allows step-by-step verification
- Automated mode for CI/CD pipelines

### Database Issues
- Database utilities included in Docker image:
  - `nostr-lmdb-dump`: Export database contents
  - `nostr-lmdb-integrity`: Check database integrity
  - `export_import`: Export/import relay data
  - `negentropy_sync`: Sync between relays

### Async Runtime Debugging with tokio-console

The relay is instrumented with tokio-console for debugging async issues.

**Local Development:**
```bash
# Build with console feature
cargo run --features console

# In another terminal
tokio-console http://localhost:6669
```

**Production/Remote:**
```bash
# Port 6669 is already exposed
ssh communities
source ~/.cargo/env
tokio-console http://localhost:6669
```

**Quick Diagnostics:**
```bash
# Automated snapshot capture
./scripts/diagnose_tokio_console.sh
```

**Common Issues:**
- **Lost Wakers**: Tasks cancelled before running (check for timeout issues)
- **High Poll Counts**: Tasks spinning or busy-waiting
- **Stuck Tasks**: Long-running tasks blocking the runtime

See `docs/debugging_async_issues.md` for comprehensive debugging guide.

**Key Views:**
- `t` - Tasks view (shows all async tasks)
- `r` - Resources view (mutexes, semaphores)
- `Enter` - Inspect selected task details

## Frontend Architecture

### NDK Wallet Integration
- Uses `@nostr-dev-kit/ndk-wallet` for Cashu wallet functionality
- Supports nutzaps and mint management
- Local storage with localforage for persistence

### State Management
- Event subscriptions managed through NDK
- Group state synchronized with relay
- Optimistic UI updates with rollback on failure

## Common Patterns

### Scoped Storage
Groups use scoped storage with `(Scope, group_id)` keys for multi-tenant support:
```rust
let scoped_key = (scope.clone(), group_id);
groups.insert(scoped_key, group);
```

### Event Processing Flow
1. ValidationMiddleware validates group events
2. GroupsEventProcessor handles group-specific logic
3. Storage layer persists to nostr-lmdb
4. Frontend receives updates via WebSocket subscription

### Testing Patterns
- Use `test_utils.rs` for common test fixtures
- Integration tests use real WebSocket connections
- Unit tests mock database interactions

## Included Binaries

- `groups_relay`: Main relay server
- `add_original_relay`: Utility to add original relay tags to events
- `delete_event`: Remove specific events from database