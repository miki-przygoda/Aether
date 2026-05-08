# Aether

**Local-First, Privacy-Centric Smart Assistant — built in Rust.**

Aether is an open-source distributed smart speaker system that keeps all AI processing on your own hardware. No cloud APIs. No telemetry. No data leaving your network.

---

## Why Aether?

Commercial smart speakers trade convenience for privacy. Aether takes a different approach: split the work between low-power edge nodes (always-on listening) and a more capable local compute node (heavy AI inference), connected over an encrypted local network. The result is Alexa-like responsiveness with full data sovereignty.

---

## Architecture

```
┌─────────────────────┐        gRPC / Tailscale VPN         ┌──────────────────────┐
│     Edge Node(s)    │ ──────────────────────────────────► │     Brain Node       │
│  (ARM SBC)          │                                     │  (x86 PC + GPU)      │
│                     │                                     │                      │
│  • Wake word detect │ ◄────────────── WAV stream ───────  │  • Whisper STT       │
│  • Audio capture    │                                     │  • Ollama LLM        │
│  • GPIO / LEDs      │                                     │  • Piper / Kokoro TTS│
│  • Audio playback   │                                     │                      │
└─────────────────────┘                                     └──────────────────────┘
```

All nodes communicate exclusively over a private, encrypted overlay network. No public IPs or ports are exposed.

---

## Tech Stack

| Layer                 | Technology                                     |
|:----------------------|:-----------------------------------------------|
| **Language**          | Rust                                           |
| **Audio I/O**         | `cpal` (ALSA / PulseAudio)                     |
| **Wake Word**         | Porcupine (local, on-device)                   |
| **Networking**        | `tonic` (gRPC) over Tailscale                  |
| **STT**               | Whisper.cpp via `whisper-rs` (GPU-accelerated) |
| **LLM**               | Ollama (Llama 3.2 / Mistral Nemo)              |
| **TTS**               | Piper (fast) or Kokoro-82M (natural)           |
| **GPIO / Hardware**   | `rppal` (I2C, PWM, GPIO)                       |
| **Cross-compilation** | `cross-rs`                                     |

---

## How It Works

1. **Idle** — Edge node listens locally for the wake word using a small on-device model. No audio leaves the device.
2. **Activation** — Wake word detected; a gRPC stream opens to the Brain node.
3. **Transcription** — Audio chunks are streamed to Whisper for GPU-accelerated speech-to-text.
4. **Inference** — The transcript is sent to Ollama. The LLM responds with structured JSON describing an action or reply.
5. **Synthesis** — Response text is converted to speech via TTS and streamed back.
6. **Playback & Action** — Edge node plays the audio and executes any GPIO actions (LEDs, buttons, etc.).

---

## Features

- **100% local inference** — Whisper, Ollama, and TTS all run on your own hardware.
- **Wake word privacy** — Detection runs entirely on the edge node; nothing is streamed until you speak the wake word.
- **Hardware feedback** — LED status indicators reflect assistant state (idle, processing, error).
- **Physical panic button** — Hardware interrupt to immediately kill all active audio.
- **Multi-node** — Auxiliary nodes can act as room sensors or remote status indicators.
- **Encrypted networking** — Tailscale overlay network; no open ports.

---

## Roadmap

### Phase 1 — Audio Pipe
- `cpal` mic capture on edge node
- Porcupine wake word detection
- gRPC server on brain node to receive PCM streams

### Phase 2 — Neural Engine
- Ollama + Whisper integration on brain node
- Skill router: parse LLM JSON output into actions vs. responses

### Phase 3 — Hardware & Voice
- GPIO LED status indicators and panic button
- Piper TTS for low-latency voice responses

### Phase 4 — Scaling
- Satellite node support (multi-room)
- Local vector DB for document-grounded memory (Qdrant)

---

## Privacy & Security

- **No external APIs.** STT, LLM, and TTS run entirely on-premises.
- **No telemetry.** Nothing is sent to any vendor.
- **Encrypted transport.** All inter-node traffic runs over Tailscale.
- **Hardware mute.** Physical microphone kill switch wired directly to the edge node.

---

## License

[MIT](LICENSE)
