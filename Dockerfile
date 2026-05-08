# syntax=docker/dockerfile:1
# ── Stage 1: build ────────────────────────────────────────────────────────────
FROM rust:1.85-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY . .

# BuildKit cache mounts keep the Cargo registry and compiled dep artifacts
# between builds.  First build is slow; subsequent builds recompile only
# changed crates.  The binary is copied out before the cache mount closes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release -p brain-node \
 && cp target/release/brain-node /brain-node

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /brain-node /usr/local/bin/brain-node

EXPOSE 50051 50052

VOLUME ["/data/certs"]

ENV BRAIN_GRPC_PORT=50051 \
    BRAIN_PAIR_PORT=50052 \
    BRAIN_CERTS_DIR=/data/certs

ENTRYPOINT ["brain-node"]
# Default: run the mTLS gRPC server.
# For pairing: docker compose run --rm brain-node pair
CMD ["serve"]
