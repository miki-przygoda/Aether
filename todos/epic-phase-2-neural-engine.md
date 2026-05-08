# Epic: Phase 2 — Neural Engine (Docker)

## Goal
Deploy the brain node as a Docker Compose stack and connect the incoming audio stream to Whisper STT and Ollama LLM, returning a structured JSON action/response to the edge node. CPU by default; GPU acceleration as an opt-in profile.

## Acceptance Criteria
- [ ] `docker compose up` starts the full brain stack on a clean machine with no extra setup
- [ ] `docker compose --profile gpu up` enables CUDA acceleration for Whisper (requires `nvidia-container-toolkit`)
- [ ] Whisper transcribes a 5-second spoken sentence with >90% word accuracy
- [ ] Ollama returns valid `LlmResponse` JSON for every transcript (no free-form text leaks)
- [ ] End-to-end latency (audio received → JSON response) is under 3s on CPU, under 1.5s on GPU
- [ ] Skill router correctly dispatches `action` values to stub handlers
- [ ] Whisper model weights are mounted from `./models` volume (not baked into image)
- [ ] All code passes CI

## Tasks
- [ ] Write `Dockerfile` for `brain-node` (Rust binary + Piper/Whisper dependencies)
- [ ] Write `docker-compose.yml`: `brain-node` + `ollama` services, `./models` volume
- [ ] Add GPU compose profile (`deploy.resources.reservations` for `nvidia` device)
- [ ] Add first-run model download script for Whisper weights
- [ ] Add `whisper-rs` to `brain-node`; load model from volume path and transcribe PCM buffer
- [ ] Add Ollama HTTP client (`reqwest`); craft system prompt enforcing `LlmResponse` JSON schema
- [ ] Build skill router: match `action` field to handler trait objects
- [ ] Wire full pipeline: gRPC PCM in → Whisper → Ollama → JSON response out
- [ ] Add unit tests for JSON parsing, skill dispatch, and CPU/GPU config branching

## Done When
PR merged to master with CI green and a real voice query processed end-to-end via `docker compose up`.
