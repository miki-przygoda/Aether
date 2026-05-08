# Epic: Phase 1 — Audio Pipe & Secure Transport

**Status: MERGED** — PR #3 merged to master 2026-05-09, CI green (fmt · clippy · test · cross-compile aarch64 + armv7).

> **Outstanding:** Baseline wake word model (TTS sample generation, real recordings, rustpotter training) and manual Pi hardware verification are deferred — tracked below. Everything else is shipped.

---

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
- [x] `cpal` captures 16kHz mono PCM on the edge node without dropped frames
- [ ] rustpotter detects "Hey Aether" using the shipped baseline model with acceptable false-positive rate — **blocked on wake word model training**
- [x] Wake word layer is abstracted behind a `WakeWordDetector` trait — backend is swappable
- [x] Initial pairing works over a wired connection (USB Ethernet / direct cable) — no WiFi or mDNS required
- [x] After pairing, brain's address is stored on the Pi; mDNS used as fallback if address changes
- [x] Pairing ceremony generates a local CA on the brain, issues a unique client cert to each Pi
- [x] Paired certs survive brain Docker restarts (stored in a named volume)
- [x] On wake word: a mTLS gRPC bidirectional stream opens to the brain within 200ms
- [x] Stream handshake includes `node_id` in gRPC metadata
- [x] Brain handles at least 3 concurrent edge node sessions without interference
- [x] Raw PCM chunks arrive in order and without corruption
- [x] Edge node returns to idle listening state cleanly after stream closes
- [x] All code passes CI (fmt, clippy, tests)

## Tasks

### Baseline Wake Word Model
- [ ] Generate synthetic "Hey Aether" samples via Kokoro TTS across all available voices
- [ ] Supplement with real samples from developer + family/friends (varied accents, environments)
- [ ] Train baseline rustpotter model and commit to `models/wake-word/hey-aether-baseline.rpw`
- [ ] Document the training process so users can retrain with their own voice

### Edge Node
- [x] Add `cpal` to `edge-node`; capture mic input into a ring buffer (512-sample, 16kHz mono)
- [x] Define `WakeWordDetector` trait in `aether-core`
- [x] Implement `RustpotterDetector` in `edge-node` backed by the shipped baseline model (behind `--features wake-word`)
- [x] Add `mdns-sd` to `edge-node`; discover `_aether._tcp.local` on boot, retry until found
- [x] Load client cert from local storage (provisioned during pairing)
- [x] Open mTLS gRPC stream with `node_id` in metadata on wake word trigger
- [x] Cross-compile `edge-node` for Pi (`aarch64-unknown-linux-gnu` / `armv7-unknown-linux-gnueabihf`) via `cross-rs`
  - `Cross.toml` at workspace root; downloads protoc 25.3 + installs `libasound2-dev` for the target arch
  - CI matrix job verifies both targets build on every PR
  - `scripts/deploy-edge.sh` copies the binary to the Pi via `AETHER_PI_HOST` env var

### Brain Node
- [x] Add `mdns-sd` to `brain-node`; advertise `_aether._tcp.local` (post-pairing discovery fallback)
- [x] Add `rcgen` + `rustls`; implement wired pairing ceremony (`brain-node pair` CLI subcommand)
  - Brain listens on wired interface during pairing; stores brain address in Pi config on completion
  - After pairing, Pi uses stored address first; falls back to mDNS if unreachable
- [x] Store CA + issued certs in Docker named volume (persist across restarts) — `compose.yml` + `aether-certs` named volume
- [x] Define `aether.proto` gRPC service with `node_id` in stream metadata
- [x] Implement session registry: `HashMap<NodeId, Session>` with async-safe access
- [x] Implement `AudioStream` RPC: accept PCM chunks, route to session, return stub response

### Tests
- [x] Unit test: `WakeWordDetector` trait mock — correct trigger/no-trigger behaviour (`aether-core/src/wake_word.rs`)
- [x] Unit test: mDNS advertisement and discovery round-trip — `#[ignore]`; requires multicast on active NIC, run with `cargo test -- --ignored` (`brain-node/src/mdns_adv.rs`)
- [x] Integration test: mock audio source → wake word trigger → mTLS stream open → PCM delivery (`edge-node/src/integration_tests.rs` + `brain-node/src/integration_tests.rs::mtls_audio_stream_handshake_and_pcm_delivery`)
- [x] Integration test: 3 concurrent mock edge nodes streaming simultaneously (`brain-node/src/integration_tests.rs`)

## Done When
PR #3 merged to master 2026-05-09 with CI green. ✓

Manual Pi hardware test (mTLS pairing confirmed, wake word triggers stream, PCM arrives at brain) pending wake word model training.
