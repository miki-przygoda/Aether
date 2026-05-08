# Epic: Phase 3 — Hardware Feedback & Multi-Node State

## Goal
Wire the GPIO hardware on the edge node to the assistant's state machine (LEDs, panic button), and connect auxiliary nodes so they mirror primary node state in real time via the brain's broadcast channel.

## Stack
- **GPIO:** `rppal` (Pi GPIO/I2C/PWM)
- **State broadcast:** `tokio::sync::broadcast` channel on the brain session registry
- **Auxiliary sync:** edge node exposes a lightweight state SSE endpoint; auxiliary nodes subscribe
- **Device discovery:** `device_discovery` module in `edge-node` — ALSA scan + I2C bus probe + HAT EEPROM read

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

### Device Discovery
The `device_discovery` module skeleton (types, registry, pure parsers, tests) is
already implemented in `edge-node/src/device_discovery.rs`.  The remaining Phase 3
work is wiring in the actual I2C bus scan and the USB hotplug watcher.

**Peripheral categories and their detection mechanism:**

| Category                                    | Detection                                                       | Notes                                                                  |
|---------------------------------------------|-----------------------------------------------------------------|------------------------------------------------------------------------|
| USB mic/speaker                             | ALSA auto-enumerates via udev; read `/proc/asound/cards`        | Works out of the box — no app code needed beyond scanning              |
| I2C mic arrays (ES7210, AC108)              | `rppal::i2c` bus scan; match against `KNOWN_I2C_CHIPS` registry | Probe each 7-bit address 0x08–0x77; treat ACK as present               |
| I2C codecs (WM8960, TLV320)                 | Same I2C scan                                                   | One chip handles mic in + speaker out                                  |
| I2S MEMS mics (INMP441, ICS-43432, SPH0645) | No I2C — appear as ALSA cards once devicetree overlay loaded    | Enable overlay in `/boot/config.txt`; after that, ALSA scan finds them |
| Pi HATs with EEPROM                         | OS reads EEPROM at boot, loads DT overlay automatically         | Read identity from `/proc/device-tree/hat/vendor` + `/product`         |
| GPIO (buttons, LEDs)                        | No detection — pin numbers come from config                     | Assignments documented in `private/CLAUDE.md`                          |
| SPI displays                                | Requires DT config; not auto-detectable                         | Out of scope for audio path                                            |

- [ ] Wire `rppal::i2c` I2C bus scan into `device_discovery::discover()` — iterate `/dev/i2c-*`, probe each address in `KNOWN_I2C_CHIPS`, populate `DiscoveredDevices::i2c_chips`
- [ ] Add `inotify` watch on `/dev/snd/` for USB audio hotplug; re-call `scan_alsa_cards()` on change and log newly appeared/disappeared cards
- [ ] Wire `SIGUSR1` handler in `main.rs` to call `discover()` and log the updated report
- [ ] Call `discover()` at startup and log detected devices before entering the wake-word loop
- [ ] Use discovered input device (or `DeviceConfig::audio_input` override) to select the cpal device instead of always using the system default

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
- [ ] Add `TtsSettings` struct to `aether-core::types`: speed (f32), pitch (f32), voice (String)
- [ ] Load `TtsSettings` from brain config (file or env); apply during Kokoro synthesis
- [ ] Persist settings to Docker volume so they survive restarts

### Tests
- [x] Unit tests: `device_discovery` — registry lookups, ALSA parser, HAT parser, discover() smoke (in `device_discovery.rs`)
- [ ] Unit test: `LedController` state machine — correct PWM values per `NodeState`
- [ ] Unit test: kill signal fan-out — multiple subscribers all receive on panic button press
- [ ] Unit test: `NodeStateEvent` broadcast — 3 subscribers each receive correct events in order
- [ ] Integration test: auxiliary node SSE subscription receives state changes within 500ms

## Done When
PR merged to master with CI green and full hardware loop verified: speak → Kokoro response + LED transitions + panic button kill + auxiliary mirror all working on real hardware.
