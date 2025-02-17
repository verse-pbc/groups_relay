ARG RUST_VERSION=1.84.0

FROM rust:${RUST_VERSION}-slim-bookworm AS rust-builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Copy only the files needed for dependency caching
COPY Cargo.toml Cargo.lock ./
COPY crates/groups_relay/Cargo.toml ./crates/groups_relay/
COPY crates/websocket_builder/Cargo.toml ./crates/websocket_builder/

# Create dummy source files for dependency caching
RUN mkdir -p crates/groups_relay/src && \
    echo "fn main() {}" > crates/groups_relay/src/main.rs && \
    mkdir -p crates/websocket_builder/src && \
    echo "fn main() {}" > crates/websocket_builder/src/lib.rs

# Build dependencies only
RUN cargo build --release --package groups_relay

# Remove the dummy source files
RUN rm -rf crates/groups_relay/src crates/websocket_builder/src

# Copy the actual source code
COPY crates/groups_relay/src ./crates/groups_relay/src
COPY crates/websocket_builder/src ./crates/websocket_builder/src

# Build the actual binary
RUN cargo build --release --package groups_relay

FROM node:20-slim AS frontend-builder

WORKDIR /usr/src/app/frontend

RUN apt-get update && apt-get install -y \
    python3 \
    make \
    g++ \
    && rm -rf /var/lib/apt/lists/*

COPY frontend/package*.json ./
COPY frontend/pnpm-lock.yaml ./

RUN npm install -g pnpm && pnpm install

COPY frontend/src ./src
COPY frontend/index.html ./
COPY frontend/vite.config.mts ./
COPY frontend/tsconfig.json ./
COPY frontend/postcss.config.cjs ./
COPY frontend/tailwind.config.js ./

ENV NODE_ENV=production
RUN pnpm run build

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy pre-built artifacts and default config
COPY --from=rust-builder /usr/src/app/target/release/groups_relay ./groups_relay
COPY crates/groups_relay/config/settings.yml ./config/
COPY --from=frontend-builder /usr/src/app/frontend/dist ./frontend/dist

EXPOSE 8080

ENV NODE_ENV=production

CMD ["./groups_relay"]