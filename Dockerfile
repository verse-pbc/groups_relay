ARG RUST_VERSION=1.86.0

FROM rust:${RUST_VERSION}-slim-bookworm AS rust-builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# Copy the entire workspace for building
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build all workspace binaries
RUN cargo build --release --workspace

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
COPY --from=rust-builder /usr/src/app/target/release/delete_event ./delete_event
COPY crates/groups_relay/config/settings.yml ./config/
COPY --from=frontend-builder /usr/src/app/frontend/dist ./frontend/dist

EXPOSE 8080

ENV NODE_ENV=production

CMD ["./groups_relay"]