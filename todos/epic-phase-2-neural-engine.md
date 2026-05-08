# Epic: Phase 2 — Neural Engine (Docker)

## Status

**IN PROGRESS** — branch `epic-phase-2-neural-engine`

| PR | Scope | State |
|----|-------|-------|
| PR 1 | STT (`whisper-rs`, confidence fallback, gRPC wiring) | merged to branch |
| PR 2 | CommandTrie (`aether-core`, trie dispatch in gRPC handler) | merged to branch |
| PR 3 | LLM fast tier (Ollama / Llama 3.2 3B) | next |
| PR 4 | Skill Router | pending |
| PR 5 | TTS (Kokoro ONNX) + edge-node WAV playback | pending |
| PR 6 | Docker Compose + auto model download | pending |

**Decision (2026-05-09):** Deep tier (DeepSeek-R1-Distill 8B) is **skipped**. Fast tier (Llama 3.2 3B) only. Docker will auto-download all models on first start.

---

## Goal
Deploy the brain as a Docker Compose stack and wire the incoming PCM stream through Whisper STT, a command trie classifier, an Ollama LLM, and Kokoro TTS — returning a WAV stream to the edge node. CPU by default; GPU as an opt-in compose profile.

## Stack
- **STT:** `whisper-rs` — medium model by default; dynamic fallback to distil-whisper-large-v3 on low confidence ✅
- **Command Classifier:** Trie-based, pure Rust — zero-latency fast path for known commands, no LLM call needed ✅
- **LLM (Fast tier):** Ollama — Llama 3.2 3B; for queries not matched by the Trie
- **~~LLM (Deep tier):~~** ~~DeepSeek-R1-Distill 8B~~ — **SKIPPED**
- **TTS:** Kokoro-82M via `ort` (ONNX Runtime Rust bindings) — no Python runtime required
- **Deployment:** Docker Compose; `docker compose up` (CPU) / `docker compose --profile gpu up` (GPU); models downloaded automatically on first start

## Acceptance Criteria
- [ ] `docker compose up` starts the full brain stack on a clean machine with no extra steps
- [ ] `docker compose --profile gpu up` enables CUDA for Whisper and Ollama (requires `nvidia-container-toolkit`)
- [x] Whisper medium transcribes audio and returns text + confidence score
- [x] Dynamic fallback: if Whisper medium confidence < 0.75, re-runs with distil-large-v3 automatically
- [x] Trie classifier matches known command patterns and returns `ClassifyResult` (Match / Partial / NoMatch)
- [x] Trie-matched commands are dispatched directly as `SkillAction` — no Ollama API call made
- [ ] Fast tier (Llama 3.2 3B) responds in under 1s on GPU, under 6s on CPU
- [ ] Fast tier always returns valid `LlmResponse` JSON
- [ ] Skill router correctly dispatches `action` field to registered handlers
- [ ] Kokoro synthesises a TTS response and streams WAV chunks back over the existing gRPC connection
- [ ] Edge node plays back WAV without underruns
- [ ] Whisper model weights loaded from mounted `./models` volume (not baked into image)
- [ ] All code passes CI

## Tasks

### Docker
- [ ] Write `Dockerfile` for `brain-node` (Rust binary + ONNX Runtime + Whisper.cpp deps)
- [ ] Write `docker-compose.yml`: `brain-node` + `ollama` services, `./models` volume
- [ ] Add GPU compose profile: `deploy.resources.reservations` for `nvidia` device on both services
- [ ] Write first-run model download script: pulls Whisper medium + distil-large-v3 GGUF into `./models` automatically
- [ ] Write Ollama model pull script: `llama3.2:3b` pulled on first start (entrypoint script)

### STT ✅
- [x] Add `whisper-rs` to `brain-node`; load model path from env var (`WHISPER_MODEL_PATH`)
- [x] Implement `transcribe(pcm: &[f32]) -> TranscriptResult` returning text + confidence score
- [x] Implement dynamic fallback: if confidence < threshold, re-run with large model
- [x] Expose confidence threshold as env var (`WHISPER_CONFIDENCE_THRESHOLD`)

### Command Classifier (Trie) ✅
- [x] Define `CommandTrie` in `aether-core` crate: prefix tree over tokenised command phrases
- [x] Seed initial command set: play/pause/stop music, set timer, lights on/off, weather, volume up/down (13 phrases)
- [x] `classify()` supports partial-match detection for streaming evaluation — awaits streaming STT token callbacks
- [x] On Trie match: send `SkillAction` to edge node directly, LLM call skipped
- [x] On no match after full transcript: stub logs "LLM path not yet implemented" (wired in PR 3)
- [x] Unit tests: 16 tests covering exact match, embedded match, case insensitivity, partial prefix, punctuation, NoMatch

### LLM
- [ ] Add `reqwest` to `brain-node`; implement Ollama HTTP client
- [ ] Implement fast tier: send transcript to Llama 3.2 3B with `format: "json"` and schema system prompt
- [ ] Parse and validate `LlmResponse` JSON; handle malformed output gracefully
- [ ] Expose model name as env var (`LLM_FAST_MODEL`, default `llama3.2:3b`)
- [ ] Replace `// TODO: LLM path` stub in `grpc.rs` with real fast-tier call

### TTS
- [ ] Export Kokoro-82M to ONNX; commit model to `./models/tts/kokoro-82m.onnx`
- [ ] Add `ort` crate to `brain-node`; load Kokoro ONNX model
- [ ] Implement `synthesise(text: &str, settings: TtsSettings) -> impl Stream<Item = Vec<u8>>`
- [ ] Wire WAV stream back to edge node over the existing gRPC connection

### Skill Router
- [ ] Define `Skill` trait: `fn matches(&self, action: &str) -> bool` + `fn execute(&self, params: Value) -> Result<()>`
- [ ] Implement stub skills: `UnknownSkill`, `RespondSkill` (TTS only), `TimerSkill` (stub)
- [ ] Register skills in a `SkillRegistry`; dispatch on `LlmResponse.action`

### Tests
- [x] Unit tests: `bytes_to_f32le` roundtrip + `#[ignore]` real-model smoke test
- [x] Integration test: MockStt → `TranscriptUpdate` delivered to edge (PR 1)
- [x] Integration test: trie match → `SkillAction{action:"play_music"}` delivered to edge (PR 2)
- [x] Unit tests: 16 trie classify tests (match / partial / no-match / punctuation / case)
- [ ] Unit test: LLM JSON schema validation (fast-tier output always valid `LlmResponse`)
- [ ] Unit test: skill router dispatch — correct skill selected for each action string
- [ ] Integration test: PCM in → Whisper → Ollama → WAV stream out (end-to-end, mocked models for CI)

## Done When
PR merged to master with CI green and a real voice query processed end-to-end via `docker compose up` — heard as a Kokoro voice response on the Pi.
