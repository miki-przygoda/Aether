# Epic: Phase 5 — Setup & Personalisation Web UI

## Goal
A self-hosted web UI served directly from the brain node's Docker stack. Covers initial device setup, wake word training, STT voice personalisation, TTS configuration, LLM settings, and a skill tester. Accessible from any browser on the local network. No cloud, no app store.

## Stack
- **Backend:** Axum (Rust, Tokio-native — same async runtime as brain-node)
- **Frontend:** Server-rendered HTML via MiniJinja templates + HTMX for partial updates
- **Real-time:** Server-Sent Events (SSE) for training progress and node state
- **Audio (browser):** Web Audio API (vanilla JS) for recording wake word and voice samples
- **Styling:** minimal CSS (no framework — keep it lightweight and local-network fast)
- **Deployment:** `web-ui` Axum server embedded in the `brain-node` binary (same process, separate Axum router mounted at `/ui`)
- **Fine-tuning:** `finetuning` Python Docker service — used only during training jobs, never in the inference pipeline. Python is the bottleneck only here, which is acceptable since it runs offline.

## Architecture

The web UI runs as a sub-router inside `brain-node`. It shares direct access to:
- Session registry (live node states)
- `TtsSettings` store
- `ModelSettings` store
- Training pipeline (rustpotter trainer + Whisper fine-tune job runner)
- Qdrant client (document ingestion trigger)
- Pairing system (cert issuance)

No inter-process communication needed — the UI has direct access to all brain internals.

```
docker-compose.yml
├── brain-node (port 50051 gRPC + port 8080 HTTP)
│   ├── gRPC server — edge node transport
│   └── Axum HTTP server
│       ├── /ui/...        — web UI pages
│       ├── /api/...       — JSON REST API
│       └── /events/...    — SSE streams
├── ollama
├── qdrant
└── finetuning  ← Python container, setup/training only
    ├── whisper fine-tuning scripts (PyTorch)
    └── REST API called by brain-node training runner
        (never started during normal assistant operation)
```

## Pages & Routes

### Dashboard `GET /ui/`
Displays:
- Connected node cards: name, `NodeState` indicator dot, last query timestamp, wake word model version
- Brain status panel: uptime, active LLM models, GPU/CPU mode, Qdrant status
- Recent activity feed: last 10 queries with node ID, transcript snippet, skill dispatched, latency
- Real-time node state via SSE (`/events/nodes`) — indicator dots update without page refresh

### Nodes `GET /ui/nodes`
Lists all paired nodes with:
- Node ID, display name (editable), paired date, last seen
- Wake word model version and training date
- "Unpair" button (revokes cert, removes from session registry)

`GET /ui/nodes/pair` — pairing wizard:
1. Brain displays a short numeric confirmation code
2. Edge node runs `aether-edge setup` and shows matching code
3. User confirms match in UI → brain issues cert → node appears in list

### Wake Word Training `GET /ui/training/wake-word`
Multi-step wizard:

**Step 1 — Select nodes**
Checkbox list of paired nodes. Training produces a model deployed to selected nodes.

**Step 2 — Record samples**
- Prompt displayed: *"Say 'Hey Aether' clearly, then pause"*
- Record button triggers browser microphone via Web Audio API
- Waveform visualiser (canvas) shows audio level during recording
- Each sample: min 0.5s, max 3s; silence-trimmed automatically
- Target: 15 samples minimum, 30 recommended
- Each sample shown as a playback card (play / delete)
- Progress bar: `n / 15 samples recorded`
- `POST /api/training/wake-word/samples` — uploads each WAV chunk

**Step 3 — Review**
- Grid of sample cards, each playable
- Quality warnings: too short, clipped, background noise detected
- Option to re-record individual samples

**Step 4 — Train**
- "Start Training" button → `POST /api/training/wake-word/train`
- SSE stream (`/events/training/wake-word`) drives a progress bar
- Estimated time shown (30–120s depending on sample count)
- On completion: model accuracy score displayed

**Step 5 — Deploy**
- Summary: new model vs baseline accuracy comparison
- "Deploy to selected nodes" → `POST /api/training/wake-word/deploy`
- Brain pushes model file to each selected Pi via the existing mTLS gRPC connection
- Nodes hot-reload the model without restarting

### Voice Personalisation `GET /ui/training/voice`
Whisper fine-tuning on one or more users' voices. Supports multiple profiles per household. Fine-tuning runs in the `finetuning` Python container — Python is only active during this wizard, never during normal assistant operation.

**Step 1 — User profiles**
- List of existing voice profiles (name + trained date)
- "Add user" button → enter name → begins recording wizard for that user
- Multiple profiles can be trained; Whisper selects the closest match at inference time (or a specific profile can be locked per node)

**Step 2 — Introduction (per user)**
- Explanation: what this does, estimated time (~10 min with GPU), what improves
- "Begin recording" button

**Step 3 — Read-aloud prompts (per user)**
- 25 varied prompts shown one at a time (phoneme diversity + command patterns)
- Example prompts:
  - "Set a timer for fifteen minutes"
  - "What's the weather like outside today?"
  - "Play some focus music in the background"
  - "Turn off the office lights and close the blinds"
- Record each prompt via Web Audio API; waveform shown during recording
- Skip button for prompts the user finds unnatural
- `POST /api/training/voice/samples` — uploads `{ user_id, transcript, wav }` tuple

**Step 4 — Fine-tune**
- "Start Fine-tuning" → `POST /api/training/voice/train` (triggers `finetuning` container job)
- SSE progress stream (`/events/training/voice`)
- Long job — show estimated remaining time, allow navigating away (job continues in background)
- Notification badge on nav when complete

**Step 5 — Activate**
- Before/after word error rate comparison on held-out samples (per user)
- Toggle: "Use personalised model" / "Revert to base model"
- Per-node assignment: choose which voice profile a given Pi defaults to

### TTS Settings `GET /ui/settings/tts`
- **Voice selector** — dropdown of available Kokoro voices with name and a short preview clip
- **Speed** — slider 0.5× to 2.0× (default 1.0×); live preview updates on release
- **Pitch** — slider −6 to +6 semitones (default 0); live preview updates on release
- **Preview** — textarea + "Play" button; `POST /api/settings/tts/preview` returns audio played inline
- **Scope** — toggle: apply globally or per-node (per-node overrides global)
- Save button → `POST /api/settings/tts`; settings persisted to Docker volume

### Model Settings `GET /ui/settings/models`
- **LLM routing mode** — radio: Fast only / Deep only / Auto (default)
  - Auto: fast tier default; deep tier triggered by query complexity classifier
- **Whisper mode** — radio: Medium only / Dynamic (default) / Large only
  - Dynamic: show confidence threshold slider (default 0.75)
- **Active models** — shows which Ollama models are pulled; "Pull" / "Remove" buttons per model
- **GPU mode** — read-only indicator (CPU / GPU); links to Docker compose docs if CPU
- Save → `POST /api/settings/models`

### Skills `GET /ui/skills`
- Table of registered skills: action name, description, example trigger phrases
- **Skill tester**: text input → "Test" button → `POST /api/skills/test`
  - Shows: matched skill name, raw LLM JSON response, which tier was used, latency
- Future hook: "Add custom skill" (Phase 6+)

### Documents `GET /ui/documents`
- File list from `./documents` volume with index status (indexed / pending / error)
- Upload button → drag-and-drop file upload → `POST /api/documents`
- "Re-index all" button → `POST /api/documents/ingest`
- SSE progress for active ingestion jobs (`/events/documents/ingest`)
- "Clear history" button per node → `DELETE /api/history/:node_id`

## API Routes

```
# Nodes
GET    /api/nodes                          → list paired nodes
POST   /api/nodes/pair                     → confirm pairing code
DELETE /api/nodes/:id                      → unpair node
PATCH  /api/nodes/:id                      → update display name

# Wake word training
POST   /api/training/wake-word/samples     → upload WAV sample
GET    /api/training/wake-word/samples     → list samples
DELETE /api/training/wake-word/samples/:id → delete sample
POST   /api/training/wake-word/train       → start training job
POST   /api/training/wake-word/deploy      → push model to nodes

# Voice personalisation
GET    /api/training/voice/users           → list voice profiles
POST   /api/training/voice/users           → create new user profile
DELETE /api/training/voice/users/:id       → delete user profile
POST   /api/training/voice/samples         → upload { user_id, transcript, wav }
POST   /api/training/voice/train           → start Whisper fine-tune (triggers finetuning container)
POST   /api/training/voice/activate        → switch to personalised model
POST   /api/training/voice/revert          → revert to base model
PATCH  /api/nodes/:id/voice-profile        → assign voice profile to a node

# TTS
GET    /api/settings/tts                   → current TTS settings
POST   /api/settings/tts                   → update TTS settings
POST   /api/settings/tts/preview           → synthesise preview (returns audio/wav)

# Models
GET    /api/settings/models                → current model settings
POST   /api/settings/models                → update model settings
POST   /api/settings/models/:name/pull     → pull Ollama model
DELETE /api/settings/models/:name          → remove Ollama model

# Skills
GET    /api/skills                         → list registered skills
POST   /api/skills/test                    → test query against skill router

# Documents
GET    /api/documents                      → list documents + index status
POST   /api/documents                      → upload document
POST   /api/documents/ingest               → trigger full re-index
DELETE /api/history/:node_id               → clear conversation history for node

# SSE streams
GET    /events/nodes                       → real-time NodeState changes
GET    /events/training/wake-word          → wake word training progress
GET    /events/training/voice              → voice fine-tune progress
GET    /events/documents/ingest            → document ingestion progress
```

## Acceptance Criteria
- [ ] Web UI accessible at `http://<brain-host>:8080/ui/` from any browser on the local network
- [ ] Dashboard shows live node state (SSE updates without page refresh)
- [ ] Wake word training wizard: records 15+ samples, trains rustpotter model, deploys to Pi — all in browser
- [ ] Deployed wake word model is hot-reloaded on the Pi without service restart
- [ ] Voice personalisation: supports multiple user profiles; each records 25 prompts; fine-tune runs in Python container; per-node profile assignment works
- [ ] Python `finetuning` container only starts when a training job is triggered — not running during normal use
- [ ] TTS preview plays audio in-browser within 2s of clicking "Play"
- [ ] TTS settings persist across brain Docker restarts
- [ ] Model settings correctly switch Whisper and LLM tier behaviour
- [ ] Skill tester returns correct skill match and LLM JSON for any test query
- [ ] Document upload + ingestion triggers Qdrant indexing with progress feedback
- [ ] All API endpoints return appropriate errors with human-readable messages
- [ ] UI is usable on mobile (PWA-friendly layout, no horizontal scroll)
- [ ] All code passes CI

## Tasks

### Setup
- [ ] Add `web-ui` module to `brain-node`
- [ ] Add Axum, MiniJinja, tower-http to `brain-node/Cargo.toml`
- [ ] Mount Axum router at `/ui` and `/api` inside brain-node binary
- [ ] Serve static assets (CSS, minimal JS) from embedded bytes (`include_str!` / `rust-embed`)

### Pages
- [ ] Dashboard page + SSE node state stream
- [ ] Nodes list + pairing wizard
- [ ] Wake word training wizard (5 steps, Web Audio API recording)
- [ ] Voice personalisation wizard (4 steps)
- [ ] TTS settings page + live preview
- [ ] Model settings page
- [ ] Skills page + tester
- [ ] Documents page + ingestion progress

### API
- [ ] Implement all routes listed above
- [ ] Shared error type → JSON `{ "error": "..." }` responses

### Training Pipeline
- [ ] rustpotter training runner: takes sample WAVs → produces `.rpw` model file (pure Rust, runs in brain-node)
- [ ] Write `finetuning/` Python service: Dockerfile (PyTorch + Whisper), REST API wrapper around fine-tune script
- [ ] Whisper fine-tune runner in brain-node: calls `finetuning` container REST API, streams progress back via SSE
- [ ] `finetuning` container: `docker compose run` style (starts on demand, exits when job completes)
- [ ] Multi-user model management: per-user GGUF stored in `./models/voice/<user_id>/`; Whisper loads correct model per session

### Tests
- [ ] Unit test: each API route returns correct status and body shape
- [ ] Unit test: SSE stream emits correct events on state changes
- [ ] Integration test: full wake word training flow (mock samples → train → deploy)

## Done When
PR merged to master with CI green and full web UI verified in browser: pairing, wake word training, TTS preview, model settings, and skill tester all functional.
