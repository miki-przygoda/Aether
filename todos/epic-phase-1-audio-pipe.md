# Epic: Phase 1 — Audio Pipe & Secure Transport

## Goal
Establish the full audio path from microphone capture on the edge node to raw PCM delivery at the brain node. Wake word detection uses rustpotter (pure Rust, no external account). All communication is secured via mTLS with a self-hosted CA. Multi-node support is built in from the start — not retrofitted.

## Stack
- **Wake Word:** `rustpotter` — pure Rust, trainable on user voice; baseline model shipped with repo
- **Audio I/O:** `cpal` (ALSA on Pi)
- **Initial Pairing:** wired (USB Ethernet / direct cable) — avoids mDNS-in-Docker on first setup
- **Discovery (ongoing):** `mdns-sd` + stored brain address — mDNS as fallback after first pairing
- **TLS:** `rcgen` (cert generation) + `rustls` (TLS runtime)
- **Transport:** `tonic` gRPC over mTLS, bidirectional streaming

## Acceptance Criteria
- [ ] `cpal` captures 16kHz mono PCM on the edge node without dropped frames
- [ ] rustpotter detects "Hey Aether" using the shipped baseline model with acceptable false-positive rate
- [ ] Wake word layer is abstracted behind a `WakeWordDetector` trait — backend is swappable
- [ ] Initial pairing works over a wired connection (USB Ethernet / direct cable) — no WiFi or mDNS required
- [ ] After pairing, brain's address is stored on the Pi; mDNS used as fallback if address changes
- [ ] Pairing ceremony generates a local CA on the brain, issues a unique client cert to each Pi
- [ ] Paired certs survive brain Docker restarts (stored in a named volume)
- [ ] On wake word: a mTLS gRPC bidirectional stream opens to the brain within 200ms
- [ ] Stream handshake includes `node_id` in gRPC metadata
- [ ] Brain handles at least 3 concurrent edge node sessions without interference
- [ ] Raw PCM chunks arrive in order and without corruption
- [ ] Edge node returns to idle listening state cleanly after stream closes
- [ ] All code passes CI (fmt, clippy, tests)

## Tasks

### Baseline Wake Word Model
- [ ] Generate synthetic "Hey Aether" samples via Kokoro TTS across all available voices
- [ ] Supplement with real samples from developer + family/friends (varied accents, environments)
- [ ] Train baseline rustpotter model and commit to `models/wake-word/hey-aether-baseline.rpw`
- [ ] Document the training process so users can retrain with their own voice

### Edge Node
- [ ] Add `cpal` to `edge-node`; capture mic input into a ring buffer (512-sample, 16kHz mono)
- [ ] Define `WakeWordDetector` trait in `aether-core`
- [ ] Implement `RustpotterDetector` in `edge-node` backed by the shipped baseline model
- [ ] Add `mdns-sd` to `edge-node`; discover `_aether._tcp.local` on boot, retry until found
- [ ] Load client cert from local storage (provisioned during pairing)
- [ ] Open mTLS gRPC stream with `node_id` in metadata on wake word trigger
- [ ] Cross-compile `edge-node` for Pi (`aarch64-unknown-linux-gnu` / `armv7-unknown-linux-gnueabihf`) via `cross-rs`
  - `Cross.toml` at workspace root; custom image installs `libasound2-dev` + `protobuf-compiler` for the target arch
  - CI job verifies the cross-compile succeeds on every PR
  - `scripts/deploy-edge.sh` copies the binary to the Pi via `AETHER_PI_HOST` env var (no hardcoded addresses)

### Brain Node
- [ ] Add `mdns-sd` to `brain-node`; advertise `_aether._tcp.local` (post-pairing discovery fallback)
- [ ] Add `rcgen` + `rustls`; implement wired pairing ceremony (`aether-brain pair` CLI subcommand)
  - Brain listens on wired interface during pairing; stores brain address in Pi config on completion
  - After pairing, Pi uses stored address first; falls back to mDNS if unreachable
- [x] Store CA + issued certs in Docker named volume (persist across restarts) — `compose.yml` + `aether-certs` named volume
- [ ] Define `aether.proto` gRPC service with `node_id` in stream metadata
- [ ] Implement session registry: `HashMap<NodeId, Session>` with async-safe access
- [ ] Implement `AudioStream` RPC: accept PCM chunks, route to session, return stub response

### Tests
- [ ] Unit test: `WakeWordDetector` trait mock — correct trigger/no-trigger behaviour
- [ ] Unit test: mDNS advertisement and discovery round-trip (loopback)
- [ ] Integration test: mock audio source → wake word trigger → mTLS stream open → PCM delivery
- [ ] Integration test: 3 concurrent mock edge nodes streaming simultaneously

## Done When
PR merged to master with CI green and manual test on Pi hardware: mTLS pairing confirmed, wake word triggers stream, PCM arrives at brain.
