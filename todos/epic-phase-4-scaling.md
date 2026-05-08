# Epic: Phase 4 — Memory & RAG

## Goal
Add persistent memory via Qdrant in the Docker Compose stack. The deep LLM tier uses retrieved document context for grounded answers. Conversation history persists across sessions per node.

## Stack
- **Vector DB:** Qdrant (Docker Compose service)
- **Embeddings:** local embedding model (e.g. `nomic-embed-text` via Ollama)
- **RAG routing:** deep LLM tier queries Qdrant before calling DeepSeek
- **History:** per-session conversation turns stored in Qdrant, retrieved on each request

## Acceptance Criteria
- [ ] Qdrant service starts automatically with `docker compose up`
- [ ] Brain node indexes a local document corpus into Qdrant on demand (via CLI or web UI trigger)
- [ ] Deep tier LLM queries include top-K retrieved context chunks above the user query
- [ ] RAG-grounded answer demonstrably uses document content (not hallucinated)
- [ ] Conversation history persists across brain restarts for each `node_id`
- [ ] Last N turns retrieved and injected into prompt on every request (N configurable)
- [ ] Multiple concurrent node sessions have fully isolated conversation histories
- [ ] All code passes CI

## Tasks

### Docker
- [ ] Add `qdrant` service to `docker-compose.yml` with a named data volume
- [ ] Add `nomic-embed-text` to the Ollama model pull script

### Qdrant Client
- [ ] Add `qdrant-client` crate to `brain-node`
- [ ] Define two Qdrant collections: `documents` (user corpus) and `history` (conversation turns)
- [ ] Implement `VectorStore` trait: `upsert`, `search(query, top_k)`, `delete`

### Document Ingestion
- [ ] Implement document ingestion pipeline:
  - Read files from a watched `./documents` volume mount
  - Chunk text (512-token sliding window, 64-token overlap)
  - Embed each chunk via Ollama `nomic-embed-text`
  - Upsert into `documents` collection with file path + chunk index metadata
- [ ] Expose ingestion trigger via CLI (`aether-brain ingest`) and web UI (Phase 5)
- [ ] Watch `./documents` for changes and re-index modified files automatically

### RAG Integration
- [ ] In deep tier LLM path: embed user query → search `documents` collection → inject top-3 chunks into prompt
- [ ] Prompt template: `[Context]\n{chunks}\n\n[User]\n{query}`
- [ ] If no relevant chunks found (score < threshold): proceed without context, log miss

### Conversation History
- [ ] On each turn: store `{ node_id, role, content, timestamp }` in `history` collection
- [ ] On each request: retrieve last N turns for `node_id`, prepend to prompt as chat history
- [ ] N configurable via env var (`HISTORY_TURNS`, default 10)
- [ ] Expose history clear endpoint (used by web UI and panic button long-press)

### Tests
- [ ] Unit test: document chunking — correct chunk sizes and overlap
- [ ] Unit test: RAG prompt assembly — chunks injected in correct position
- [ ] Unit test: history retrieval — returns last N turns in chronological order
- [ ] Integration test: two simultaneous sessions with independent histories — no cross-contamination

## Done When
PR merged to master with CI green and a RAG-grounded document query plus persistent conversation history verified end-to-end.
