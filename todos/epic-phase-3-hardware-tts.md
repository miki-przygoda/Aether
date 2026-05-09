# Epic: Phase 3 — Hardware Feedback & Multi-Node State

## Goal
Wire the GPIO hardware on the edge node to the assistant's state machine (LEDs, panic button), and connect auxiliary nodes so they mirror primary node state in real time via the brain's broadcast channel.

## Stack
- **GPIO:** `rppal` (Pi GPIO/I2C/PWM)
- **State broadcast:** `tokio::sync::broadcast` channel on the brain session registry
- **Auxiliary sync:** edge node exposes a lightweight state SSE endpoint; auxiliary nodes subscribe
- **Device discovery:** `device_discovery` module in `edge-node` — ALSA scan + I2C bus probe + HAT EEPROM read

## Acceptance Criteria
- [x] LED state machine transitions correctly driven by `NodeState`:
  - `Idle` → solid green
  - `Processing` → pulsing blue (PWM)
  - `Error` / DND → flashing red
- [x] Panic button GPIO interrupt immediately kills active audio playback and resets to `Idle`
- [x] Kill signal propagates via `tokio::sync::broadcast` — all active tasks on the edge node receive it
- [x] Auxiliary node (Pi 3B+) receives `NodeState` updates and mirrors LED within 500ms
- [x] Brain session registry publishes `NodeState` changes on every transition
- [x] TTS speed and pitch settings (from Phase 5 web UI) are respected during Kokoro synthesis
- [x] All code passes CI

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
- [x] Wire `SIGUSR1` handler in `main.rs` to call `discover()` and log the updated report
- [x] Call `discover()` at startup and log detected devices before entering the wake-word loop
- [ ] Use discovered input device (or `DeviceConfig::audio_input` override) to select the cpal device instead of always using the system default

### LED State Machine
- [x] Add `rppal` to `edge-node` (`gpio` feature flag — compile with `--features gpio` on Pi)
- [x] Define GPIO pin assignments as env vars (`AETHER_LED_RED/GREEN/BLUE_PIN`) — exact values in `private/CLAUDE.md`
- [x] Implement `LedController`: drives 3-colour LED via GPIO output pins; pulsing/flashing via background tasks
- [x] Wire `LedController` to `NodeState` receiver — update LED on every state change

### Panic Button
- [x] Register GPIO interrupt on panic button pin via `rppal` (`AETHER_PANIC_BUTTON_PIN` env var)
- [x] On interrupt: publish kill signal to `tokio::sync::broadcast::Sender<KillSignal>`
- [x] Audio playback task and gRPC stream task both subscribe and abort on signal
- [x] Reset `NodeState` to `Idle` after kill

### Brain State Broadcast
- [x] Add `tokio::sync::broadcast::Sender<NodeStateEvent>` to session registry
- [x] Publish `NodeStateEvent { node_id, state }` on every session state transition
- [x] Expose internal broadcast channel to web UI (Phase 5) for real-time dashboard updates

### Auxiliary Node Sync
- [x] Add lightweight HTTP SSE endpoint to `edge-node`: `GET /state/events` streams `NodeState` as SSE (axum, port configurable via `AETHER_STATE_PORT`)
- [x] Implement auxiliary mode in `edge-node` binary: `auxiliary --target <url>` subcommand
- [x] Auxiliary mode: connects to target edge node SSE, drives its own LEDs to mirror state

### TTS Settings Integration
- [x] Add `TtsSettings` struct to `aether-core::types`: speed (f32), voice (String)
- [x] Load `TtsSettings` from brain config (env: `TTS_SPEED`, `TTS_VOICE`); apply during Kokoro synthesis
- [ ] Persist settings to Docker volume so they survive restarts (deferred to Phase 5 web UI)

### Tests
- [x] Unit tests: `device_discovery` — registry lookups, ALSA parser, HAT parser, discover() smoke (in `device_discovery.rs`)
- [x] Unit test: `LedController` state machine — `state_to_pattern()` maps all `NodeState` variants correctly (`gpio.rs`)
- [x] Unit test: kill signal fan-out — multiple subscribers all receive on panic button press (`kill_signal.rs`)
- [x] Unit test: `NodeStateEvent` broadcast — 3 subscribers each receive correct events in order (`session.rs`)
- [x] Unit test: SSE data round-trips through JSON for all `NodeState` variants (`state_server.rs`)

## Done When
PR merged to master with CI green and full hardware loop verified: speak → Kokoro response + LED transitions + panic button kill + auxiliary mirror all working on real hardware.
