# Nostr Groups Frontend

This is the frontend for the Nostr Groups application, built with Preact and TypeScript.

## Development

### Prerequisites

- Node.js (v16 or later)
- pnpm (v7 or later)

### Setup

1. Install dependencies:

```bash
pnpm install
```

2. Configure environment variables:

The application uses environment variables to configure the WebSocket connection to the backend server. In development mode, these are set in `.env.development`.

- `VITE_WEBSOCKET_URL`: The URL of the WebSocket server (e.g., `ws://0.0.0.0:8080`)

### Running in Development Mode

```bash
pnpm run dev
```

This will start the development server, typically on port 5173.

### Building for Production

```bash
pnpm run build
```

This will create a production build in the `dist` directory.

## Connecting to the Backend

When running the frontend in development mode, it will connect to the WebSocket server specified in the `.env.development` file. By default, this is set to `ws://0.0.0.0:8080`.

If you're running the backend on a different host or port, you'll need to update the `VITE_WEBSOCKET_URL` environment variable accordingly.

**Important**: Make sure the backend server is running and accessible from your browser. If you're running the backend in Docker, you may need to expose the port and ensure it's accessible from your local network.

## Authentication

The application supports two methods of authentication:

1. **Nostr Extension**: If you have a Nostr browser extension installed (like Alby or nos2x), you can use it to authenticate.
2. **Private Key**: You can enter your Nostr private key directly. The key should be a valid 64-character hex string or an nsec value.

## Troubleshooting

### Connection Issues

If you're having trouble connecting to the backend:

1. Make sure the backend server is running
2. Check that the WebSocket URL in `.env.development` matches the backend server's address
3. Look for any CORS errors in the browser console
4. Try using `0.0.0.0` instead of `localhost` in the WebSocket URL

### Authentication Issues

If you're having trouble authenticating:

1. Make sure you're using a valid private key (64-character hex string or nsec value)
2. Check the browser console for any error messages
3. Ensure the backend server is properly configured to accept WebSocket connections