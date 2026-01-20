ARG RUST_VERSION=1.91.0

FROM rust:${RUST_VERSION}-slim-bookworm AS rust-builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    make \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Copy the project for building
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY src ./src
COPY benches ./benches

# Build all binaries (console feature disabled for stability testing)
# tokio_unstable needed for runtime metrics used by watchdog
# tokio_taskdump enables task dumps when deadlocks are detected (Linux only)
ENV RUSTFLAGS="--cfg tokio_unstable --cfg tokio_taskdump"
RUN cargo build --release --bins

# Install binaries from relay_builder
RUN cargo install --git https://github.com/verse-pbc/relay_builder \
    --bin export_import \
    --bin negentropy_sync \
    --bin nostr-lmdb-dump \
    --bin nostr-lmdb-integrity

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
    curl \
    iproute2 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy all pre-built binaries and default config
COPY --from=rust-builder /usr/src/app/target/release/groups_relay ./groups_relay
COPY --from=rust-builder /usr/src/app/target/release/delete_event ./delete_event
COPY --from=rust-builder /usr/src/app/target/release/add_original_relay ./add_original_relay
# console_dump requires console-dump feature, skipped for stability testing
# COPY --from=rust-builder /usr/src/app/target/release/console_dump ./console_dump
# Copy cargo-installed binaries
COPY --from=rust-builder /usr/local/cargo/bin/export_import ./export_import
COPY --from=rust-builder /usr/local/cargo/bin/negentropy_sync ./negentropy_sync
COPY --from=rust-builder /usr/local/cargo/bin/nostr-lmdb-dump ./nostr-lmdb-dump
COPY --from=rust-builder /usr/local/cargo/bin/nostr-lmdb-integrity ./nostr-lmdb-integrity
COPY config/settings.yml ./config/
COPY --from=frontend-builder /usr/src/app/frontend/dist ./frontend/dist

EXPOSE 8080
EXPOSE 6669

ENV NODE_ENV=production

CMD ["./groups_relay"]