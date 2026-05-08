# Epic: Phase 1 — Audio Pipe & Secure Transport

## Goal
Establish the full audio path from microphone capture on the edge node to raw PCM delivery at the brain node, with wake word detection gating the stream. All communication is encrypted via mTLS using a self-hosted CA — no external services required.

## Acceptance Criteria
- [ ] `cpal` captures 16kHz mono PCM on the edge node without dropped frames
- [ ] Porcupine detects "Hey Aether" with acceptable false-positive rate in a quiet room
- [ ] Edge node discovers the brain automatically via mDNS on the local network
- [ ] Pairing ceremony generates a local CA on the brain and issues a client cert to the Pi
- [ ] On wake word: a mTLS gRPC bidirectional stream opens to the brain within 200ms
- [ ] Stream handshake includes `node_id` in metadata
- [ ] Brain handles at least 2 concurrent edge node sessions without interference
- [ ] Raw PCM chunks arrive in order and without corruption
- [ ] Edge node returns to idle listening state after stream closes
- [ ] All code passes CI (fmt, clippy, tests)

## Tasks
- [ ] Add `cpal` to `edge-node`; capture mic input into a ring buffer
- [ ] Integrate `pvporcupine` crate; wire up "Hey Aether" model file
- [ ] Add `mdns-sd` to both crates; brain advertises `_aether._tcp.local`, edge discovers on boot
- [ ] Add `rcgen` + `rustls` to both crates; implement pairing ceremony (`aether-brain pair` / `aether-edge setup`)
- [ ] Define `aether.proto` gRPC service with `node_id` in stream metadata
- [ ] Add `tonic` (TLS) to both crates; implement `AudioStream` RPC
- [ ] Implement session registry in brain-node: `HashMap<NodeId, Session>`
- [ ] Add integration test: mock audio source → wake word trigger → mTLS stream open → PCM delivery

## Done When
PR merged to master with CI green and manual test on Pi hardware with mTLS pairing confirmed.
