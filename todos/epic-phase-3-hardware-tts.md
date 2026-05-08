# Epic: Phase 3 — Hardware Feedback & Multi-Node State

## Goal
Wire the GPIO hardware on the edge node to the assistant's state machine (LEDs, panic button), and connect auxiliary nodes so they mirror primary node state in real time via the brain's broadcast channel.

## Stack
- **GPIO:** `rppal` (Pi GPIO/I2C/PWM)
- **State broadcast:** `tokio::sync::broadcast` channel on the brain session registry
- **Auxiliary sync:** edge node exposes a lightweight state SSE endpoint; auxiliary nodes subscribe

## Acceptance Criteria
- [ ] LED state machine transitions correctly driven by `NodeState`:
  - `Idle` → solid green
  - `Processing` → pulsing blue (PWM)
  - `Error` / DND → flashing red
- [ ] Panic button GPIO interrupt immediately kills active audio playback and resets to `Idle`
- [ ] Kill signal propagates via `tokio::sync::broadcast` — all active tasks on the edge node receive it
- [ ] Auxiliary node (Pi 3B+) receives `NodeState` updates and mirrors LED within 500ms
- [ ] Brain session registry publishes `NodeState` changes on every transition
- [ ] TTS speed and pitch settings (from Phase 5 web UI) are respected during Kokoro synthesis
- [ ] All code passes CI

## Tasks

### LED State Machine
- [ ] Add `rppal` to `edge-node`
- [ ] Define GPIO pin assignments as constants (documented in `private/CLAUDE.md`)
- [ ] Implement `LedController`: drives 3-colour LED via PWM for pulsing blue, solid/flash for others
- [ ] Wire `LedController` to `NodeState` receiver — update LED on every state change

### Panic Button
- [ ] Register GPIO interrupt on panic button pin via `rppal`
- [ ] On interrupt: publish kill signal to `tokio::sync::broadcast::Sender<KillSignal>`
- [ ] Audio playback task and gRPC stream task both subscribe and abort on signal
- [ ] Reset `NodeState` to `Idle` after kill

### Brain State Broadcast
- [ ] Add `tokio::sync::broadcast::Sender<NodeStateEvent>` to session registry
- [ ] Publish `NodeStateEvent { node_id, state }` on every session state transition
- [ ] Expose internal broadcast channel to web UI (Phase 5) for real-time dashboard updates

### Auxiliary Node Sync
- [ ] Add lightweight HTTP SSE endpoint to `edge-node`: `GET /state/events` streams `NodeState` as SSE
- [ ] Implement auxiliary mode in `edge-node` binary: `--mode auxiliary --target <node_id>`
- [ ] Auxiliary mode: connects to target edge node SSE, drives its own LEDs to mirror state

### TTS Settings Integration
- [ ] Add `TtsSettings` struct to `shared::types`: speed (f32), pitch (f32), voice (String)
- [ ] Load `TtsSettings` from brain config (file or env); apply during Kokoro synthesis
- [ ] Persist settings to Docker volume so they survive restarts

### Tests
- [ ] Unit test: `LedController` state machine — correct PWM values per `NodeState`
- [ ] Unit test: kill signal fan-out — multiple subscribers all receive on panic button press
- [ ] Unit test: `NodeStateEvent` broadcast — 3 subscribers each receive correct events in order
- [ ] Integration test: auxiliary node SSE subscription receives state changes within 500ms

## Done When
PR merged to master with CI green and full hardware loop verified: speak → Kokoro response + LED transitions + panic button kill + auxiliary mirror all working on real hardware.
