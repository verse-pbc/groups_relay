# Nostr NIP-29 Proxy

A middleware-based Nostr relay proxy that adds NIP-29 (and other NIPs) support to any existing Nostr relay. While this implementation uses strfry as the backing relay, the proxy can work with any Nostr relay implementation.

## Overview

This project implements a WebSocket proxy that sits between Nostr clients and a relay, allowing us to add functionality to any relay without modifying its codebase. The proxy intercepts WebSocket communication and implements additional Nostr NIPs that may not be supported by the backing relay.

### Implemented NIPs (Work in Progress)

The proxy is working towards implementing the following Nostr Implementation Possibilities (NIPs):

- **NIP-29**: Partial implementation of group chat functionality
- **NIP-42**: Authentication of clients to relays (both read and write operations)
- **NIP-70**: Basic relay-defined content moderation

### Architecture

The proxy uses a flexible middleware architecture (built on top of the `websocket_builder` crate) that allows for:
- Easy addition of new NIPs through middleware components
- Per-connection state management
- Message interception and modification
- Forwarding to the backing relay

## Running the Project

```bash
cargo run
```

You can monitor the group state changes through the web interface hosted at the root path (`/`). The web interface provides visibility into:
- Active groups
- Group state changes

### Local development

```bash
docker compose up --build
```

This will spawn a relay just for the docker session, it will be emptied once the compose server is stopped.

A webpage is available to inspect and interact with the relay groups at `/`.

## Configuration

The proxy can be configured to connect to any backing relay. By default, it connects to a local strfry instance at `localhost:7777`, but this can be modified through configuration or command line arguments.

## Status

This is a work in progress. While some core functionality works, there are significant limitations and ongoing work:

- Group membership validation is still being refined
- Performance optimizations are needed for large groups
- Additional NIPs are planned for implementation
- The web interface is basic and needs enhancement
- More comprehensive testing is needed

## Development

The project is structured around middleware components that can be composed to add functionality. Each NIP is implemented as a separate middleware, making it easy to add or remove features.

Key components:
- `middlewares/`: Contains implementations of different NIPs
- `websocket_builder/`: The core WebSocket handling framework
- `group.rs`: NIP-29 group management logic
- `main.rs`: Server setup and configuration

## License

MIT License