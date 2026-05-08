# Epic: Phase 1 — Audio Pipe & Secure Transport

## Goal
Establish the full audio path from microphone capture on the edge node to raw PCM delivery at the brain node. Wake word detection uses rustpotter (pure Rust, no external account). All communication is secured via mTLS with a self-hosted CA. Multi-node support is built in from the start — not retrofitted.

## Stack
- **Wake Word:** `rustpotter` — pure Rust, trainable on user voice
- **Audio I/O:** `cpal` (ALSA on Pi)
- **Discovery:** `mdns-sd` — brain advertises `_aether._tcp.local`
- **TLS:** `rcgen` (cert generation) + `rustls` (TLS runtime)
- **Transport:** `tonic` gRPC over mTLS, bidirectional streaming

## Acceptance Criteria
- [ ] `cpal` captures 16kHz mono PCM on the edge node without dropped frames
- [ ] rustpotter detects "Hey Aether" using the shipped baseline model with acceptable false-positive rate
- [ ] Wake word layer is abstracted behind a `WakeWordDetector` trait — backend is swappable
- [ ] Edge node discovers the brain automatically via mDNS on boot — no IP config required
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
- [ ] Record a diverse set of "Hey Aether" samples (varied speakers, environments)
- [ ] Train baseline rustpotter model and commit to `models/wake-word/hey-aether-baseline.rpw`
- [ ] Document the training process so users can retrain with their own voice

### Edge Node
- [ ] Add `cpal` to `edge-node`; capture mic input into a ring buffer (512-sample, 16kHz mono)
- [ ] Define `WakeWordDetector` trait in `shared`
- [ ] Implement `RustpotterDetector` in `edge-node` backed by the shipped baseline model
- [ ] Add `mdns-sd` to `edge-node`; discover `_aether._tcp.local` on boot, retry until found
- [ ] Load client cert from local storage (provisioned during pairing)
- [ ] Open mTLS gRPC stream with `node_id` in metadata on wake word trigger

### Brain Node
- [ ] Add `mdns-sd` to `brain-node`; advertise `_aether._tcp.local`
- [ ] Add `rcgen` + `rustls`; implement pairing ceremony (`aether-brain pair` CLI subcommand)
- [ ] Store CA + issued certs in Docker named volume (persist across restarts)
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
