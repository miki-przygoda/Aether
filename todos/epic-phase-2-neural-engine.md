# Epic: Phase 2 — Neural Engine (Docker)

## Goal
Deploy the brain as a Docker Compose stack and wire the incoming PCM stream through Whisper STT, a two-tier LLM router, and Kokoro TTS — returning a WAV stream to the edge node. CPU by default; GPU as an opt-in compose profile.

## Stack
- **STT:** `whisper-rs` — medium model by default; dynamic fallback to distil-whisper-large-v3 on low confidence
- **LLM (Fast tier):** Ollama — Llama 3.2 3B; for quick commands and simple queries
- **LLM (Deep tier):** Ollama — DeepSeek-R1-Distill 8B; for complex queries and reasoning tasks
- **LLM output:** Grammar-constrained JSON (llama.cpp grammar via Ollama API) — guarantees schema validity
- **TTS:** Kokoro-82M via `ort` (ONNX Runtime Rust bindings) — no Python runtime required
- **Deployment:** Docker Compose; `docker compose up` (CPU) / `docker compose --profile gpu up` (GPU)

## Acceptance Criteria
- [ ] `docker compose up` starts the full brain stack on a clean machine with no extra steps
- [ ] `docker compose --profile gpu up` enables CUDA for Whisper and Ollama (requires `nvidia-container-toolkit`)
- [ ] Whisper medium transcribes a 5-second spoken sentence with >90% word accuracy on CPU
- [ ] Dynamic fallback: if Whisper medium confidence < 0.75, re-runs with distil-large-v3 automatically
- [ ] Fast tier (Llama 3.2 3B) responds in under 1s on GPU, under 6s on CPU
- [ ] Deep tier (DeepSeek-R1-Distill 8B) responds in under 3s on GPU (thinking tokens stripped before routing)
- [ ] Both LLM tiers always return valid `LlmResponse` JSON — schema enforced via grammar constraint
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
- [ ] Write first-run model download script: pulls Whisper medium + distil-large-v3 GGUF into `./models`
- [ ] Write Ollama model pull script: `llama3.2:3b` and `deepseek-r1:8b` pulled on first start

### STT
- [ ] Add `whisper-rs` to `brain-node`; load model path from env var (`WHISPER_MODEL_PATH`)
- [ ] Implement `transcribe(pcm: &[f32]) -> TranscriptResult` returning text + confidence score
- [ ] Implement dynamic fallback: if confidence < threshold (configurable, default 0.75), re-run with large model
- [ ] Expose confidence threshold as env var (`WHISPER_CONFIDENCE_THRESHOLD`)

### LLM
- [ ] Add `reqwest` to `brain-node`; implement Ollama HTTP client
- [ ] Define `LlmResponse` JSON grammar file for llama.cpp grammar-constrained generation
- [ ] Implement two-tier router: classify query complexity → select fast or deep model
  - Fast: single-step commands, lookups, timers
  - Deep: multi-step reasoning, document Q&A, ambiguous context
- [ ] Strip DeepSeek thinking tokens (`<think>...</think>`) before parsing JSON response
- [ ] Expose model names as env vars (`LLM_FAST_MODEL`, `LLM_DEEP_MODEL`)

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
- [ ] Unit test: Whisper transcription confidence fallback logic
- [ ] Unit test: LLM JSON schema validation (grammar-constrained output always valid)
- [ ] Unit test: thinking token stripping for DeepSeek responses
- [ ] Unit test: skill router dispatch — correct skill selected for each action string
- [ ] Integration test: PCM in → Whisper → Ollama → WAV stream out (end-to-end, mocked models for CI)

## Done When
PR merged to master with CI green and a real voice query processed end-to-end via `docker compose up` — heard as a Kokoro voice response on the Pi.
