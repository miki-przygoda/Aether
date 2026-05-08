# Epic: Phase 1 — Audio Pipe

## Goal
Establish the full audio path from microphone capture on the edge node to raw PCM delivery at the brain node, with wake word detection gating the stream.

## Acceptance Criteria
- [ ] `cpal` captures 16kHz mono PCM on the edge node without dropped frames
- [ ] Porcupine detects "Hey Aether" with acceptable false-positive rate in a quiet room
- [ ] On wake word: a gRPC bidirectional stream opens to the brain node within 200ms
- [ ] Raw PCM chunks arrive at the brain node in order and without corruption
- [ ] Edge node returns to idle listening state after stream closes
- [ ] All code passes CI (fmt, clippy, tests)

## Tasks
- [ ] Add `cpal` to `edge-node` and capture mic input into a ring buffer
- [ ] Integrate `pvporcupine` crate; wire up "Hey Aether" model file
- [ ] Add `tonic` to both crates; define `aether.proto` gRPC service
- [ ] Implement `AudioStream` RPC: edge streams PCM, brain echoes chunk count (stub)
- [ ] Add integration test: mock audio source → wake word trigger → stream open

## Done When
PR merged to master with CI green and manual test on Pi 4 hardware confirmed.
