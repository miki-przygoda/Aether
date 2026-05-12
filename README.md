# Aether

**Local-first, privacy-centric smart assistant — built in Rust.**

Aether is an open-source smart speaker system that keeps every piece of AI processing on your own hardware. No cloud APIs, no telemetry, no accounts, no data leaving your network. Wake word detection, speech-to-text, language model inference, and text-to-speech all run locally — on devices you own and control.

---

## How It Works

Aether splits the workload between two roles:

- **Edge node** — a low-power ARM board (e.g. Raspberry Pi) that sits in the room. It listens for the wake word locally and streams audio to the brain only after activation. Nothing leaves the device until you speak.
- **Brain node** — a more powerful machine running Docker that handles the heavy lifting: Whisper for transcription, Ollama for the language model, and Kokoro for speech synthesis.

Edge nodes discover the brain automatically over your local network via mDNS. All traffic between them is encrypted with mutual TLS using a self-hosted certificate authority created during a one-time pairing ceremony.

```
  Edge Node ────────────────────────────────────────────────────────────┐
  (ARM SBC)                                                             │
                                                                        │
    Microphone → wake word detection (on-device, always private)        │
                                                                        │
    Wake word detected → mTLS gRPC stream ───────────────────────────►  │
                                                                        │  Brain Node
                         WAV response ◄───────────────────────────────  │  (Docker Compose)
                                                                        │
    Speaker ← playback                                                  │   Whisper  ·  Ollama  ·  Kokoro
                                                                        │   Qdrant (RAG + history)
  Browser ──── http://<brain>:8080/ui/ ──────────────────────────────►  │   Web UI
```

The brain serves a self-hosted web UI at port 8080 — use it from any browser on your local network to pair nodes, train your wake word, manage documents, and configure every setting.

---

## Getting Started

### What You Need

- A machine to run the brain (any x86-64 or ARM64 machine with Docker and at least 8 GB RAM)
- A Raspberry Pi or similar ARM single-board computer for the edge node
- A USB microphone and speaker for the edge node

---

### 1. Launch the Brain

Clone the repository and download the AI model weights, then start the brain with Docker Compose.

```bash
git clone https://github.com/miki-przygoda/Aether
cd Aether

# Download Whisper and Kokoro model weights (~2 GB)
./scripts/download-models.sh

# Start the full brain stack
docker compose up
```

If your machine has an NVIDIA GPU and the `nvidia-container-toolkit` installed, enable GPU acceleration:

```bash
docker compose -f compose.yml -f compose.gpu.yml up
```

Once running, open the web UI in a browser on the same network:

```
http://<brain-machine-ip>:8080/ui/
```

A setup wizard will guide you through the remaining steps.

---

### 2. Install the Edge Node on Your Pi

The deploy script cross-compiles `edge-node` for ARM on your development machine and copies the binary to the Pi over SSH. Before running it, set the two environment variables it needs:

| Variable | Description | Example |
|:---|:---|:---|
| `AETHER_PI_HOST` | SSH target in `user@host` form | `pi@raspberrypi.local` |
| `AETHER_PI_ARCH` | Cross-compile target (optional) | `aarch64-unknown-linux-gnu` *(default)* |

The easiest way is to create a `.env` file in the repo root so you never have to type them again:

```bash
# .env  (already in .gitignore — safe to store here)
AETHER_PI_HOST=pi@raspberrypi.local
AETHER_PI_ARCH=aarch64-unknown-linux-gnu
```

Then deploy:

```bash
source .env && ./scripts/deploy-edge.sh
```

The script builds the binary with `cross` if it is not already present, then copies it to `/usr/local/bin/edge-node` on the Pi via `scp`. SSH key-based authentication is assumed — if you have not set up an SSH key for your Pi yet, run `ssh-copy-id pi@raspberrypi.local` first.

To run the edge node automatically on boot, add a systemd unit on the Pi after deploying:

```bash
sudo tee /etc/systemd/system/aether-edge.service > /dev/null <<'EOF'
[Unit]
Description=Aether edge node
After=network.target

[Service]
ExecStart=/usr/local/bin/edge-node run --model-path /home/pi/.config/aether/hey-aether.rpw
Restart=on-failure
Environment=AETHER_MODEL_PATH=/home/pi/.config/aether/hey-aether.rpw
User=pi

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload && sudo systemctl enable --now aether-edge
```

---

### 3. Pair the Pi with the Brain

Run the pairing command on the Pi. You will need a wired (Ethernet) connection to the brain for this step — the pairing port is not exposed over Wi-Fi by default.

```bash
edge-node pair \
  --brain-addr <brain-ip>:50052 \
  --node-id living-room-pi
```

The brain will prompt you to approve the pairing request in the terminal. Accept it. The Pi receives a signed client certificate and stores its configuration in `~/.config/aether/`. After this point, the Pi connects to the brain automatically over mTLS — no IP addresses or credentials need to be set manually again.

---

### 4. Train Your Wake Word

In the web UI, go to **Wake Word Training**. Record at least five short samples of yourself saying the wake phrase. The trainer automatically trims silence, generates phoneme-anchored reference samples via TTS, and builds a rustpotter model. Hit **Deploy** — the model is pushed to all connected edge nodes over the existing encrypted connection without a restart.

---

### 5. Start Listening

On the Pi, start the edge node:

```bash
edge-node run \
  --model-path ~/.config/aether/hey-aether.rpw
```

The Pi will begin listening for the wake word immediately. Speak the phrase, ask a question, and Aether will respond through the speaker.

If you installed the systemd unit in step 2, the edge node starts automatically on boot and restarts on failure — no manual intervention needed.

---

## Features

**AI and voice**
- Trainable wake word — record samples in-browser and deploy a custom model to all nodes over the encrypted connection, with no restart required.
- On-device wake word detection — no audio leaves the edge node until activation.
- Whisper speech-to-text with automatic quality-tier fallback based on confidence score.
- Ollama-powered language model (Llama 3.2 by default; any Ollama-compatible model works).
- Kokoro TTS — natural-sounding speech synthesis running entirely on your hardware.
- Document-grounded memory — drop `.txt` or `.md` files into `./documents/`; they are embedded and indexed into Qdrant so answers are grounded in your own content.
- Persistent conversation history per node, stored in Qdrant and injected into every request.

**Hardware and reliability**
- LED status indicators mirror assistant state (idle, listening, processing, error) in real time.
- Physical panic button — hardware interrupt that immediately kills any active audio stream.
- Auxiliary node mode — additional boards can mirror a primary node's LED state without running a microphone or wake word detector.
- Multi-node — connect as many edge nodes as you want; each has a fully independent session and conversation history.
- CPU by default, GPU-accelerated with a single compose flag.

**Configuration and management**
- Self-hosted web UI served directly from the brain node — no separate service, no external dependencies.
- Real-time dashboard via server-sent events — node state updates without a page refresh.
- Model management — pull, configure, or remove Ollama models from the browser.
- Skill tester — send arbitrary queries against the skill router and inspect routing decisions from the web UI.

---

## Privacy

Aether's privacy guarantees are enforced at the software level, not as a policy.

| What could leak                     | What Aether does                                                                                                                                                             |
|:------------------------------------|:-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Always-on microphone audio          | Wake word detection runs entirely on the edge node. Audio is only streamed after activation — the brain never receives idle microphone data.                                 |
| Voice data sent to vendors          | All STT (Whisper), LLM (Ollama), and TTS (Kokoro) run on your own hardware. No external API calls are made.                                                                  |
| Traffic intercepted on your network | All edge-to-brain traffic is encrypted with mutual TLS. Both sides authenticate with certificates issued by a local CA you control.                                          |
| Metadata and telemetry              | There is no telemetry. No analytics, no crash reporting, no usage data is collected or transmitted anywhere.                                                                 |
| External account requirements       | There are no accounts. Discovery uses mDNS. Encryption uses a self-hosted certificate authority. Nothing requires an internet connection after model weights are downloaded. |

The web configuration UI is served on HTTP on your local network and is not intended to be exposed to the internet. For deployments where the brain machine is reachable from outside the network, put the UI behind a reverse proxy with authentication.

---

## Tech Stack

| Layer             | Technology                                      |
|:------------------|:------------------------------------------------|
| Language          | Rust 2021, async via Tokio                      |
| Wake word         | `rustpotter` — pure Rust, trained on your voice |
| STT               | Whisper.cpp via `whisper-rs`                    |
| LLM               | Ollama (Llama 3.2 3B default)                   |
| TTS               | Kokoro-82M via ONNX Runtime                     |
| Audio I/O         | `cpal` (ALSA on Linux, CoreAudio on macOS)      |
| Networking        | `tonic` gRPC over mTLS (`rustls` + `rcgen`)     |
| Discovery         | `mdns-sd`                                       |
| Vector DB         | Qdrant                                          |
| Embeddings        | `nomic-embed-text` via Ollama                   |
| Web UI            | Axum + MiniJinja, server-sent events            |
| GPIO              | `rppal`                                         |
| Brain deployment  | Docker Compose                                  |
| Cross-compilation | `cross-rs`                                      |

---

## License

[MIT](LICENSE)
