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
  (ARM SBC)     │                     ┌──────────────────────────────────────┐
                ├── mTLS gRPC ──────► │   Brain Node (Docker Compose)        │
  Edge Node 2 ──┤   (local network)   │                                      │
  (ARM SBC)     │                     │  ┌──────────┐  ┌─────────┐           │
                │   WAV stream ◄───── │  │brain-node│  │ ollama  │           │
  Edge Node N ──┘                     │  │  Rust    │  │  LLM    │           │
                                      │  │  + Web   │  └─────────┘           │
  Browser ───── HTTP :8080 ─────────► │  │  UI      │  ┌─────────┐           │
  (local LAN)                         │  └──────────┘  │ qdrant  │           │
                                      │                 │  RAG    │           │
                                      │                 └─────────┘           │
                                      └──────────────────────────────────────┘
```

Edge nodes discover the brain automatically on the local network via mDNS — no accounts, no manual configuration. All inter-node traffic is encrypted with mutual TLS using a self-hosted certificate authority established during a one-time pairing ceremony.

The web UI is served directly from the brain node binary on port 8080 — no separate service, full access to all brain internals.

---

## Repository Layout

```
Aether/
├── crates/
│   ├── aether-core/   — shared types and traits (LlmResponse, NodeState, …)
│   ├── brain-node/    — Docker-deployed inference server (STT · LLM · TTS · Web UI)
│   └── edge-node/     — ARM SBC binary (wake word · audio capture · gRPC client)
├── finetuning/        — Python service for Whisper voice personalisation (on-demand only)
├── proto/             — gRPC service definition (aether.proto)
├── scripts/           — model download, wake word training, edge deploy helpers
└── todos/             — phase epics and task tracking
```

---

## Tech Stack

| Layer                  | Technology                                      |
|:-----------------------|:------------------------------------------------|
| **Language**           | Rust (edition 2021, async via Tokio)            |
| **Audio I/O**          | `cpal` (ALSA / CoreAudio)                       |
| **Wake Word**          | `rustpotter` — pure Rust, trainable on your voice |
| **Discovery**          | `mdns-sd` (zero-config local network)           |
| **Networking**         | `tonic` (gRPC) over mTLS                        |
| **TLS**                | `rustls` + `rcgen` (self-hosted CA)             |
| **STT**                | Whisper.cpp via `whisper-rs`                    |
| **LLM**                | Ollama (Llama 3.2 3B)                           |
| **TTS**                | Kokoro-82M via `ort` (ONNX Runtime)             |
| **Memory / RAG**       | Qdrant (local vector DB, Docker Compose service)|
| **Embeddings**         | `nomic-embed-text` via Ollama                   |
| **Web UI**             | Axum + MiniJinja (server-rendered, SSE)         |
| **GPIO / Hardware**    | `rppal` (I2C, PWM, GPIO)                        |
| **Brain Deployment**   | Docker Compose (CPU default, GPU opt-in)        |
| **Cross-compilation**  | `cross-rs`                                      |

---

## How It Works

1. **Idle** — Edge node listens locally for the wake word using a small on-device rustpotter model. No audio leaves the device.
2. **Activation** — Wake word detected; a mTLS gRPC stream opens to the brain node (discovered automatically via mDNS).
3. **Transcription** — Audio chunks are streamed to Whisper for speech-to-text. A confidence-based fallback automatically escalates to a larger model when needed.
4. **Routing** — The transcript is first matched against a zero-latency command trie. On a match, the skill is dispatched directly — no LLM call. On no match, the transcript goes to Ollama.
5. **Inference** — Ollama retrieves relevant document context from Qdrant (RAG) and recent conversation history before calling the LLM. The model responds with structured JSON describing an action or reply.
6. **Synthesis** — Response text is converted to speech via Kokoro TTS and streamed back.
7. **Playback & Action** — Edge node plays the audio and executes any GPIO actions (LED state, etc.).

---

## Getting Started

### Brain Node (any machine with Docker)

```bash
# Download Whisper and Kokoro model weights
./scripts/download-models.sh

# Start the full stack (CPU)
docker compose up

# GPU-accelerated (requires nvidia-container-toolkit)
docker compose -f compose.yml -f compose.gpu.yml up
```

Once running, the **web configuration UI** is available at:

```
http://<brain-host>:8080/ui/
```

Use it to pair edge nodes, train your wake word, configure TTS, manage documents, and test skills — all from any browser on the local network.

### Edge Node (ARM SBC)

```bash
# Cross-compile for the target (from dev machine)
./scripts/deploy-edge.sh

# First boot on the Pi — discovers brain via mDNS and pairs
edge-node pair --brain-addr <brain-ip>:50052 --node-id my-pi
```

No config files, no IP addresses stored manually, no accounts.

---

## Features

- **100% local inference** — Whisper, Ollama, and TTS all run on your own hardware.
- **Wake word privacy** — Detection runs entirely on the edge node; nothing is streamed until you speak the wake word.
- **Trainable wake word** — Record samples in-browser and train a custom `rustpotter` model from the web UI. Deploys to all edge nodes over the existing mTLS connection — no restart required.
- **Voice personalisation** — Fine-tune Whisper on your voice via the web UI. Supports multiple per-household user profiles.
- **Document-grounded memory** — Drop `.txt` or `.md` files into `./documents/`; Qdrant indexes them for RAG. Answers are grounded in your documents, not hallucinated.
- **Persistent conversation history** — Per-node conversation context stored in Qdrant and injected into every request. Persists across brain restarts.
- **Zero-config discovery** — Edge nodes find the brain via mDNS automatically on the local network.
- **Self-hosted encryption** — mTLS with a local CA. One pairing ceremony per node; no external services.
- **Multi-node** — Connect as many edge nodes as you want; each has a fully isolated session and conversation history.
- **Web configuration UI** — Manage everything from a browser: nodes, TTS settings, model settings, skill tester, document ingestion.
- **Hardware feedback** — LED status indicators reflect assistant state (idle, processing, error).
- **Physical panic button** — Hardware interrupt to immediately kill all active audio.
- **CPU or GPU** — Brain runs on CPU by default; GPU acceleration is a one-flag compose override.

---

## Web UI

The brain node serves a self-hosted configuration interface at `http://<brain-host>:8080/ui/`.

| Page | Description |
|:-----|:------------|
| **Dashboard** | Live node status via SSE — connection state updates in real time without a page refresh |
| **Nodes** | Manage paired edge nodes; run the pairing wizard to add new ones |
| **Wake Word Training** | Record samples in-browser, train a custom model, deploy to nodes (hot-reload) |
| **Voice Personalisation** | Record prompts per user and fine-tune Whisper on your voice |
| **TTS Settings** | Adjust voice, speed, and pitch; play a preview in-browser |
| **Model Settings** | Configure Whisper mode, LLM routing, pull/remove Ollama models |
| **Skills** | Browse registered skills and test any query against the skill router |
| **Documents** | Upload documents and trigger Qdrant ingestion with live progress |

---

## Roadmap

All five phases are shipped. The system is fully functional end-to-end.

| Phase | Description | Status |
|:------|:------------|:-------|
| **1 — Audio Pipe & Secure Transport** | `cpal` capture, rustpotter wake word, mDNS discovery, mTLS pairing, gRPC streaming | ✅ Shipped |
| **2 — Neural Engine** | Docker Compose brain stack, Whisper STT, Ollama LLM, Kokoro TTS, skill router | ✅ Shipped |
| **3 — Hardware Feedback** | GPIO LEDs, panic button, auxiliary node state mirroring | ✅ Shipped |
| **4 — Memory & RAG** | Qdrant vector DB, document ingestion, RAG-grounded answers, conversation history | ✅ Shipped |
| **5 — Web Configuration UI** | Self-hosted Axum UI, wake word training wizard, voice personalisation, TTS/model settings | ✅ Shipped |

---

## Privacy & Security

- **No external APIs.** STT, LLM, and TTS run entirely on your own hardware.
- **No telemetry.** Nothing is sent to any vendor.
- **No accounts.** Discovery is via mDNS; encryption via a self-hosted certificate authority.
- **Encrypted transport.** All inter-node traffic is mutual TLS — both sides authenticate with certificates you issued.
- **Hardware mute.** Physical microphone kill switch wired directly to the edge node.
- **Local web UI.** The configuration interface is HTTP on the local network only — not exposed to the internet.

---

## License

[MIT](LICENSE)
