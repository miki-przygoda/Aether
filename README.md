# Aether

**Local-First, Privacy-Centric Smart Assistant — built in Rust.**

Aether is an open-source distributed smart speaker system that keeps all AI processing on your own hardware. No cloud APIs. No telemetry. No data leaving your network.

---

## Why Aether?

Commercial smart speakers trade convenience for privacy. Aether takes a different approach: split the work between low-power edge nodes (always-on listening) and a Dockerised brain node (heavy AI inference), connected over an encrypted local network. The result is Alexa-like responsiveness with full data sovereignty — and no accounts, subscriptions, or external services required.

---

## Architecture

```
  Edge Node 1 ──┐
  (ARM SBC)     │                     ┌─────────────────────────────┐
                ├── mTLS gRPC ──────► │   Brain Node (Docker)       │
  Edge Node 2 ──┤   (local network)   │  ┌──────────┐ ┌─────────┐   │
  (ARM SBC)     │                     │  │brain-node│ │ ollama  │   │
                │   WAV stream ◄───── │  │  Rust    │ │  LLM    │   │
  Edge Node N ──┘                     │  └──────────┘ └─────────┘   │
                                      └─────────────────────────────┘
```

Edge nodes discover the brain automatically on the local network via mDNS — no accounts, no manual configuration. All traffic is encrypted with mutual TLS using a self-hosted certificate authority established during a one-time pairing ceremony.

---

## Repository Layout

```
crates/
├── aether-core/   — shared types and traits (LlmResponse, NodeState, …)
├── brain-node/    — Docker-deployed inference server (STT · LLM · TTS)
└── edge-node/     — ARM SBC binary (wake word · audio capture · gRPC client)
```

---

## Tech Stack

| Layer                 | Technology                               |
|:----------------------|:-----------------------------------------|
| **Language**          | Rust                                     |
| **Audio I/O**         | `cpal` (ALSA / PulseAudio)               |
| **Wake Word**         | Porcupine (local, on-device)             |
| **Discovery**         | `mdns-sd` (zero-config local network)    |
| **Networking**        | `tonic` (gRPC) over mTLS                 |
| **TLS**               | `rustls` + `rcgen` (self-hosted CA)      |
| **STT**               | Whisper.cpp via `whisper-rs`             |
| **LLM**               | Ollama (Llama 3.2 / Mistral Nemo)        |
| **TTS**               | Piper (fast) or Kokoro-82M (natural)     |
| **GPIO / Hardware**   | `rppal` (I2C, PWM, GPIO)                 |
| **Brain Deployment**  | Docker Compose (CPU default, GPU opt-in) |
| **Cross-compilation** | `cross-rs`                               |

---

## How It Works

1. **Idle** — Edge node listens locally for the wake word using a small on-device model. No audio leaves the device.
2. **Activation** — Wake word detected; a mTLS gRPC stream opens to the brain node (discovered automatically via mDNS).
3. **Transcription** — Audio chunks are streamed to Whisper for speech-to-text.
4. **Inference** — The transcript is sent to Ollama. The LLM responds with structured JSON describing an action or reply.
5. **Synthesis** — Response text is converted to speech via TTS and streamed back.
6. **Playback & Action** — Edge node plays the audio and executes any GPIO actions (LEDs, buttons, etc.).

---

## Getting Started

### Brain Node (any machine with Docker)

```bash
# CPU (works everywhere)
docker compose up

# GPU-accelerated (requires nvidia-container-toolkit)
docker compose --profile gpu up
```

### Edge Node (ARM SBC)

```bash
# First boot — discovers brain via mDNS and runs pairing ceremony
aether-edge setup
```

That's it. No config files, no IP addresses, no accounts.

---

## Features

- **100% local inference** — Whisper, Ollama, and TTS all run on your own hardware.
- **Wake word privacy** — Detection runs entirely on the edge node; nothing is streamed until you speak the wake word.
- **Zero-config discovery** — Edge nodes find the brain via mDNS automatically on the local network.
- **Self-hosted encryption** — mTLS with a local CA. One pairing ceremony per node; no external services.
- **Multi-node** — Connect as many edge nodes as you want; each runs independently on the brain.
- **Hardware feedback** — LED status indicators reflect assistant state (idle, processing, error).
- **Physical panic button** — Hardware interrupt to immediately kill all active audio.
- **CPU or GPU** — Brain runs on CPU by default; GPU acceleration is a single flag.

---

## Roadmap

### Phase 1 — Audio Pipe & Secure Transport
- `cpal` mic capture on edge node
- Porcupine wake word detection
- mDNS brain discovery + mTLS pairing ceremony
- gRPC bidirectional streaming (multi-node from day one)

### Phase 2 — Neural Engine (Docker)
- Brain node as a Docker Compose stack (CPU default, GPU opt-in)
- Whisper STT + Ollama LLM integration
- Skill router: parse LLM JSON output into actions vs. responses

### Phase 3 — Hardware & Voice
- GPIO LED status indicators and panic button
- Piper TTS streamed back to edge node
- Auxiliary node state mirroring

### Phase 4 — Memory & Scaling
- Local vector DB (Qdrant) in the Docker stack
- Document-grounded memory via RAG
- Persistent conversation context across sessions

---

## Privacy & Security

- **No external APIs.** STT, LLM, and TTS run entirely on your own hardware.
- **No telemetry.** Nothing is sent to any vendor.
- **No accounts.** Discovery is via mDNS; encryption via a self-hosted certificate authority.
- **Encrypted transport.** All inter-node traffic is mutual TLS — both sides authenticate.
- **Hardware mute.** Physical microphone kill switch wired directly to the edge node.

---

## License

[MIT](LICENSE)
