FROM rust:1.80.1-slim-bookworm as builder

WORKDIR /usr/src/app
COPY . .

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /usr/src/app/target/release/groups_relay ./groups_relay
COPY --from=builder /usr/src/app/config ./config
COPY --from=builder /usr/src/app/frontend/dist ./frontend/dist

ENV RUST_LOG=info

EXPOSE 8080

CMD ["./groups_relay"]