# Epic: Phase 2 — Neural Engine

## Goal
Connect the incoming audio stream to Whisper STT and pipe the transcript through Ollama, returning a structured JSON action/response to the edge node.

## Acceptance Criteria
- [ ] Whisper transcribes a 5-second spoken sentence with >90% word accuracy on the brain node
- [ ] Ollama returns valid `LlmResponse` JSON for every transcript (no free-form text leaks)
- [ ] End-to-end latency (audio received → JSON response) is under 3s on local hardware
- [ ] Skill router correctly dispatches `action` values to stub handlers
- [ ] All code passes CI

## Tasks
- [ ] Add `whisper-rs` to `brain-node`; load model and transcribe a PCM buffer
- [ ] Add Ollama HTTP client (reqwest); craft system prompt enforcing JSON output
- [ ] Define `LlmResponse` schema in `shared::types` (already stubbed)
- [ ] Build skill router: match `action` string to handler trait objects
- [ ] Wire full pipeline: gRPC PCM in → Whisper → Ollama → JSON response out
- [ ] Add unit tests for JSON parsing and skill dispatch

## Done When
PR merged to master with CI green and a real voice query processed end-to-end on local hardware.
