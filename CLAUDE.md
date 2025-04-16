# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands
- Build: `cargo build`
- Run: `cargo run`
- Test all: `cargo test`
- Test single: `cargo test test_name`
- Test specific crate: `cargo test -p crates_name`
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Frontend dev: `cd frontend && npm run dev`
- Frontend build: `cd frontend && npm run build`

## Code Style Guidelines
- Rust: Follow standard Rust conventions (use rustfmt & clippy)
- Use error handling with anyhow/thiserror/snafu for propagation
- Type safety: Prefer strong typing with proper error types
- Use async/await with Tokio for async operations
- Imports: Group standard library, external crates, and internal modules
- Naming: snake_case for variables/functions, CamelCase for types
- TypeScript: Use type annotations for all exports
- Structure code with modules and clear separation of concerns
- Use middleware pattern for request processing
- Test all public functionality

## Important Notes
- Always import Nostr types from `nostr_sdk::prelude::*` 
- Never import directly from `nostr` package
- This is a Nostr groups relay implementing NIP-29 group management

## NIP-29 Implementation
This project implements the [NIP-29 Relay-Based Groups](https://github.com/nostr-protocol/nips/blob/master/29.md) specification with these key components:

### Key Event Kinds
- Group management (9000-9020): Add/remove users, edit metadata, delete events
- User events (9021-9022): Join/leave requests
- Group metadata (39000): Define name, picture, description, visibility
- Group state (39001-39003): Admins list, members list, supported roles

### Group Features
- Public/private visibility: Control read access
- Open/closed access: Auto-accept vs. manual approval for joins
- Broadcast mode: Only admins can post content
- Role-based access control: Admin/member privileges

### Tag Types
- `h` tag: Required for group identification
- `d` tag: Used for addressable event kinds in the 39xxx range