FROM node:20-slim as frontend-builder

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
COPY groups_relay ./groups_relay
COPY config/settings.yml ./config/
COPY --from=frontend-builder /usr/src/app/frontend/dist ./frontend/dist

EXPOSE 8080

ENV NODE_ENV=production

CMD ["./groups_relay"]