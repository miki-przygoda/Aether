# Epic: Phase 3 — Hardware & TTS

## Goal
Close the loop: convert the LLM text response to audio and play it back on the edge node, while GPIO hardware reflects assistant state via LEDs and responds to the panic button.

## Acceptance Criteria
- [ ] Piper synthesises a response sentence and streams WAV back to edge node
- [ ] Edge node plays WAV via ALSA without underruns
- [ ] LED state machine transitions correctly: Idle (green) → Processing (blue pulse) → Error (red flash)
- [ ] Panic button immediately kills audio playback and resets state to Idle
- [ ] Auxiliary node mirrors edge node state within 500ms
- [ ] All code passes CI

## Tasks
- [ ] Integrate Piper binary in `brain-node`; stream WAV over existing gRPC connection
- [ ] Add WAV playback to `edge-node` via `cpal` output stream
- [ ] Add `rppal` to `edge-node`; implement LED state machine
- [ ] Wire GPIO interrupt for panic button → `tokio::sync::broadcast` kill signal
- [ ] Implement auxiliary node state-sync HTTP endpoint on `edge-node`
- [ ] Write unit tests for LED state transitions

## Done When
PR merged to master with CI green and full voice loop (speak → hear response + LED feedback) verified on hardware.
