# Epic: Phase 4 — Memory & Scaling

## Goal
Add persistent memory via a local vector DB (Qdrant) in the Docker Compose stack and support retrieval-augmented generation (RAG) so the assistant can answer questions grounded in local documents.

## Acceptance Criteria
- [ ] Qdrant service added to `docker-compose.yml`; starts automatically with the brain stack
- [ ] Brain node indexes at least one local document corpus into Qdrant on demand
- [ ] LLM can answer questions grounded in indexed documents (RAG-augmented prompt)
- [ ] Conversation context persists across sessions (stored in and retrieved from Qdrant)
- [ ] Brain handles N concurrent edge node sessions with independent conversation histories
- [ ] All code passes CI

## Tasks
- [ ] Add `qdrant` service to `docker-compose.yml` with a named data volume
- [ ] Add `qdrant-client` crate to `brain-node`
- [ ] Build document ingestion pipeline: file chunking, embedding (local model), upsert to Qdrant
- [ ] Modify Ollama prompt builder to inject retrieved context chunks above the user query
- [ ] Implement per-session conversation history: store turns in Qdrant, retrieve last N on each request
- [ ] Refactor session registry to support full concurrent histories per `node_id`
- [ ] Integration test: two simultaneous mock streams with independent conversation contexts

## Done When
PR merged to master with CI green and a RAG-grounded query (document Q&A) plus persistent context verified end-to-end.
