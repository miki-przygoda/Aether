# Epic: Phase 4 — Memory & Scaling

## Goal
Add persistent memory via a local vector DB and support multi-room satellite nodes.

## Acceptance Criteria
- [ ] Qdrant running on the brain node indexes at least one local document corpus
- [ ] LLM can answer questions grounded in indexed documents (retrieval-augmented generation)
- [ ] A second edge node can be added to the Tailscale mesh and handled concurrently
- [ ] Conversation context persists across sessions (stored in Qdrant)
- [ ] All code passes CI

## Tasks
- [ ] Deploy Qdrant on brain node; add `qdrant-client` crate
- [ ] Build document ingestion pipeline: chunking, embedding (via local model), upsert
- [ ] Modify LLM prompt to include retrieved context snippets
- [ ] Refactor brain node to handle multiple concurrent gRPC streams (one per edge node)
- [ ] Add per-session conversation history stored and retrieved from Qdrant
- [ ] Integration test: two simultaneous mock streams processed correctly

## Done When
PR merged to master with CI green and RAG-grounded query plus multi-node concurrency verified on hardware.
