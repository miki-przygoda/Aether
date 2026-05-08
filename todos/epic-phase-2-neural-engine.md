# Epic: Phase 2 — Neural Engine (Docker)

## Goal
Deploy the brain as a Docker Compose stack and wire the incoming PCM stream through Whisper STT, a two-tier LLM router, and Kokoro TTS — returning a WAV stream to the edge node. CPU by default; GPU as an opt-in compose profile.

## Stack
- **STT:** `whisper-rs` — medium model by default; dynamic fallback to distil-whisper-large-v3 on low confidence
- **Command Classifier:** Trie-based, pure Rust, runs on partial Whisper output in parallel — zero-latency fast path for known commands, no LLM call needed
- **LLM (Fast tier):** Ollama — Llama 3.2 3B; for queries not matched by the Trie
- **LLM (Deep tier):** Ollama — DeepSeek-R1-Distill 8B; for complex queries escalated from the fast tier
- **LLM output:** Grammar-constrained JSON (llama.cpp grammar via Ollama API) — guarantees schema validity
- **⚠️ STOP BEFORE IMPLEMENTING DEEP TIER:** Grammar constraints + DeepSeek thinking tokens may conflict. See implementation note at bottom of this epic.
- **TTS:** Kokoro-82M via `ort` (ONNX Runtime Rust bindings) — no Python runtime required
- **Deployment:** Docker Compose; `docker compose up` (CPU) / `docker compose --profile gpu up` (GPU)

## Acceptance Criteria
- [ ] `docker compose up` starts the full brain stack on a clean machine with no extra steps
- [ ] `docker compose --profile gpu up` enables CUDA for Whisper and Ollama (requires `nvidia-container-toolkit`)
- [ ] Whisper medium transcribes a 5-second spoken sentence with >90% word accuracy on CPU
- [ ] Dynamic fallback: if Whisper medium confidence < 0.75, re-runs with distil-large-v3 automatically
- [ ] Trie classifier matches known command patterns from partial Whisper output (streaming) before full transcript arrives
- [ ] Trie-matched commands are dispatched directly — no Ollama API call made
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

### Command Classifier (Trie)
- [ ] Define `CommandTrie` in `aether-core` crate: prefix tree over tokenised command phrases
- [ ] Seed initial command set: play/pause music, set timer, lights on/off, weather, volume up/down
- [ ] Wire classifier to Whisper streaming output — evaluate partial transcript on each new token
- [ ] On Trie match: emit `TrieAction` directly to skill router, cancel pending LLM call if any
- [ ] On no match after full transcript: pass to LLM fast tier
- [ ] Unit test: correct match / no-match / partial-match behaviour across command set

### LLM
- [ ] Add `reqwest` to `brain-node`; implement Ollama HTTP client
- [ ] Define `LlmResponse` JSON grammar file for llama.cpp grammar-constrained generation
- [ ] Implement fast tier: send transcript to Llama 3.2 3B with grammar constraint
- [ ] Implement escalation: if fast tier returns `action: "unknown"` or malformed JSON → retry with deep tier
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

---

## ⚠️ Implementation Note — Deep Tier (Stop & Discuss Before Proceeding)

Before implementing the DeepSeek-R1-Distill integration, stop and discuss with the user. The core tension:

- Grammar-constrained generation forces immediate JSON output
- DeepSeek-R1 emits `<think>...</think>` chain-of-thought tokens before its answer
- These two behaviours are likely incompatible with a strict grammar applied from token 0

Options to evaluate at the time:
1. **Ollama `format: json`** — softer constraint, no grammar file, works with thinking tokens (strip them post-generation)
2. **Grammar applied after thinking block** — if Ollama supports grammar activation mid-stream (unclear at time of writing)
3. **Switch deep tier to Qwen2.5-7B** — no thinking tokens, grammar constraints work cleanly, strong instruction following
4. **Retry loop** — no grammar, validate JSON post-generation, retry on failure (simplest but adds latency)

Pick the approach with the user based on what Ollama supports at implementation time.
