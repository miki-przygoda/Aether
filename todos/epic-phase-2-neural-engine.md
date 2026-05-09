# Epic: Phase 2 — Neural Engine (Docker)

## Status

**IN PROGRESS** — branch `epic-phase-2-neural-engine`

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
- **Deployment:** Docker Compose; `docker compose up` (CPU) / `docker compose -f compose.yml -f compose.gpu.yml up` (GPU); models downloaded automatically on first start

## Acceptance Criteria
- [x] `docker compose up` starts the full brain stack on a clean machine (after running download-models.sh)
- [x] `docker compose -f compose.yml -f compose.gpu.yml up` enables CUDA for Whisper and Ollama (requires `nvidia-container-toolkit`)
- [x] Whisper medium transcribes audio and returns text + confidence score
- [x] Dynamic fallback: if Whisper medium confidence < 0.75, re-runs with distil-large-v3 automatically
- [x] Trie classifier matches known command patterns and returns `ClassifyResult` (Match / Partial / NoMatch)
- [x] Trie-matched commands are dispatched directly as `SkillAction` — no Ollama API call made
- [ ] Fast tier (Llama 3.2 3B) responds in under 1s on GPU, under 6s on CPU
- [ ] Fast tier always returns valid `LlmResponse` JSON
- [x] Skill router correctly dispatches `action` field to registered handlers
- [x] Kokoro synthesises a TTS response and streams WAV chunks back over the existing gRPC connection
- [x] Edge node plays back WAV without underruns
- [x] Whisper model weights loaded from mounted `./models` volume (not baked into image)
- [ ] All code passes CI

## Tasks

### Docker ✅
- [x] Write `Dockerfile` for `brain-node` (Rust binary + ONNX Runtime + Whisper.cpp deps + espeak-ng)
- [x] Write `compose.yml`: `brain-node` + `ollama` services, `./models` volume, Ollama auto-pulls `llama3.2:3b`
- [x] Add GPU override: `compose.gpu.yml` with `deploy.resources.reservations` for nvidia on both services
- [x] Write model download script: `scripts/download-models.sh` — Whisper + Kokoro into `./models/`

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
- [x] On no match after full transcript: stub logs "LLM path not yet implemented"
- [x] Unit tests: 16 tests covering exact match, embedded match, case insensitivity, partial prefix, punctuation, NoMatch

### LLM ✅
- [x] Add `reqwest` to `brain-node`; implement Ollama HTTP client
- [x] Implement fast tier: send transcript to Llama 3.2 3B with `format: "json"` and schema system prompt
- [x] Parse and validate `LlmResponse` JSON; handle malformed output gracefully
- [x] Expose model name as env var (`LLM_FAST_MODEL`, default `llama3.2:3b`)
- [x] Replace `// TODO: LLM path` stub in `grpc.rs` with real fast-tier call

### TTS ✅
- [x] Add `ort` crate to `brain-node`; load Kokoro-82M ONNX model via `ort load-dynamic`
- [x] Implement `synthesise(text: &str) -> Result<Vec<u8>>` — espeak-ng phonemization + ONNX inference + WAV encoding
- [x] Wire WAV stream back to edge node over the existing gRPC connection (TtsChunk payload)
- [x] Edge node receives TtsChunk, decodes WAV, plays via cpal (with linear-interp resample)

### Skill Router ✅
- [x] Define `Skill` trait: `fn handle(&self, params: &Value) -> SkillResult`
- [x] Implement stub skills: `UnknownSkill`, `RespondSkill`, `TimerSkill`, and 7 others
- [x] Register skills in `SkillRegistry`; dispatch on `LlmResponse.action`

### Tests
- [x] Unit tests: `bytes_to_f32le` roundtrip + `#[ignore]` real-model smoke test
- [x] Integration test: MockStt → `TranscriptUpdate` delivered to edge
- [x] Integration test: trie match → `SkillAction{action:"play_music"}` delivered to edge
- [x] Unit tests: 16 trie classify tests (match / partial / no-match / punctuation / case)
- [x] Unit test: LLM JSON schema validation (fast-tier output always valid `LlmResponse`)
- [x] Unit test: skill router dispatch — correct skill selected for each action string
- [x] Integration test: PCM in → Whisper → Ollama → WAV stream out (end-to-end, mocked models for CI)

## Done When
PR merged to master with CI green and a real voice query processed end-to-end via `docker compose up` — heard as a Kokoro voice response on the Pi.
