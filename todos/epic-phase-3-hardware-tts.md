# Epic: Phase 3 — Hardware & TTS

## Goal
Close the loop: synthesise the LLM text response to audio and play it back on the edge node, while GPIO hardware reflects assistant state via LEDs and responds to the panic button. Auxiliary nodes mirror state via the Tokio broadcast channel.

## Acceptance Criteria
- [ ] Piper synthesises a response sentence and streams WAV back to the edge node over the existing gRPC connection
- [ ] Edge node plays WAV via ALSA without underruns
- [ ] LED state machine transitions correctly: Idle (green) → Processing (blue pulse) → Error (red flash)
- [ ] Panic button immediately kills audio playback and resets state to Idle
- [ ] Auxiliary node receives `NodeState` broadcast and updates its LED within 500ms
- [ ] All code passes CI

## Tasks
- [ ] Integrate Piper binary in `brain-node` container; stream WAV back over existing gRPC connection
- [ ] Add WAV playback to `edge-node` via `cpal` output stream
- [ ] Add `rppal` to `edge-node`; implement LED state machine driven by `NodeState`
- [ ] Wire GPIO interrupt for panic button → `tokio::sync::broadcast` kill signal
- [ ] Add `NodeState` broadcast channel to brain session registry; publish on every state change
- [ ] Implement auxiliary node subscriber: connects to edge node state endpoint, mirrors LED
- [ ] Write unit tests for LED state transitions and broadcast fan-out

## Done When
PR merged to master with CI green and full voice loop (speak → hear response + LED feedback + auxiliary mirror) verified on hardware.
