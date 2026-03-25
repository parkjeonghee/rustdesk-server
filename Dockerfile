# Stage 1: Build
FROM rust:1.85-bookworm AS builder

WORKDIR /build
COPY . .
RUN cargo build --release

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends libssl3 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/hbbs /usr/bin/hbbs
COPY --from=builder /build/target/release/hbbr /usr/bin/hbbr
COPY --from=builder /build/target/release/rustdesk-utils /usr/bin/rustdesk-utils

EXPOSE 21115 21116 21116/udp 21117 21118 21119 21120

WORKDIR /data
VOLUME /data

CMD ["hbbs"]
