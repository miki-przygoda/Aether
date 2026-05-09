# syntax=docker/dockerfile:1
#
# Brain-node image: Rust binary + whisper.cpp + espeak-ng + ONNX Runtime.
#
# Build arg:
#   ORT_VERSION — ONNX Runtime version to bundle (matches ort crate rc target)

ARG ORT_VERSION=1.20.0

# ── Stage 1: download ONNX Runtime shared library ─────────────────────────────
FROM debian:bookworm-slim AS ort-download
ARG ORT_VERSION
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL \
    "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz" \
    | tar xz --strip-components=2 -C /usr/local/lib \
    --wildcards "*/lib/libonnxruntime.so*" "*/lib/libonnxruntime_providers_shared.so"

# ── Stage 2: build brain-node binary ─────────────────────────────────────────
FROM rust:1.85-slim-bookworm AS builder

# cmake + build-essential compile whisper.cpp (whisper-rs build.rs).
# protobuf-compiler generates gRPC stubs.
RUN apt-get update && apt-get install -y --no-install-recommends \
        cmake \
        build-essential \
        protobuf-compiler \
        pkg-config \
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

# ── Stage 3: runtime ─────────────────────────────────────────────────────────
FROM debian:bookworm-slim

# espeak-ng is invoked at runtime by KokoroTts to phonemize text.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        espeak-ng \
    && rm -rf /var/lib/apt/lists/*

# ONNX Runtime shared library — loaded at runtime via ort load-dynamic feature.
COPY --from=ort-download /usr/local/lib/ /usr/local/lib/
RUN ldconfig

COPY --from=builder /brain-node /usr/local/bin/brain-node

EXPOSE 50051 50052 8080

VOLUME ["/data/certs", "/data/config", "/models"]

ENV BRAIN_GRPC_PORT=50051 \
    BRAIN_PAIR_PORT=50052 \
    BRAIN_WEB_PORT=8080 \
    BRAIN_CERTS_DIR=/data/certs \
    BRAIN_CONFIG_DIR=/data/config \
    WHISPER_MODEL_PATH=/models/whisper/ggml-medium.bin \
    WHISPER_FALLBACK_MODEL_PATH=/models/whisper/large-v3-turbo.bin \
    WHISPER_CONFIDENCE_THRESHOLD=0.75 \
    KOKORO_MODEL_PATH=/models/tts/kokoro-82m.onnx \
    OLLAMA_BASE_URL=http://ollama:11434 \
    LLM_FAST_MODEL=llama3.2:3b \
    ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so

ENTRYPOINT ["brain-node"]
# Default: run the mTLS gRPC server.
# For pairing: docker compose run --rm brain-node pair
CMD ["serve"]
