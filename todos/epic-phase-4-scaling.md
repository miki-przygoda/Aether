# Epic: Phase 4 — Memory & RAG

## Goal
Add persistent memory via Qdrant in the Docker Compose stack. The deep LLM tier uses retrieved document context for grounded answers. Conversation history persists across sessions per node.

## Stack
- **Vector DB:** Qdrant (Docker Compose service)
- **Embeddings:** local embedding model (e.g. `nomic-embed-text` via Ollama)
- **RAG routing:** deep LLM tier queries Qdrant before calling DeepSeek
- **History:** per-session conversation turns stored in Qdrant, retrieved on each request

## Acceptance Criteria
- [x] Qdrant service starts automatically with `docker compose up`
- [x] Brain node indexes a local document corpus into Qdrant on demand (via CLI or web UI trigger)
- [x] Deep tier LLM queries include top-K retrieved context chunks above the user query
- [x] RAG-grounded answer demonstrably uses document content (not hallucinated)
- [x] Conversation history persists across brain restarts for each `node_id`
- [x] Last N turns retrieved and injected into prompt on every request (N configurable)
- [x] Multiple concurrent node sessions have fully isolated conversation histories
- [x] All code passes CI

## Tasks

### Docker
- [x] Add `qdrant` service to `docker-compose.yml` with a named data volume
- [x] Add `nomic-embed-text` to the Ollama model pull script

### Qdrant Client
- [x] Add `qdrant-client` crate to `brain-node`
- [x] Define two Qdrant collections: `documents` (user corpus) and `history` (conversation turns)
- [x] Implement `VectorStore` trait: `upsert`, `search(query, top_k)`, `delete`

### Document Ingestion
- [x] Implement document ingestion pipeline:
  - Read files from a watched `./documents` volume mount
  - Chunk text (2048-char sliding window, 256-char overlap ≈ 512/64 tokens)
  - Embed each chunk via Ollama `nomic-embed-text`
  - Upsert into `documents` collection with file path + chunk index metadata
- [x] Expose ingestion trigger via CLI (startup ingest via `--documents-dir`); web UI trigger deferred to Phase 5
- [ ] Watch `./documents` for changes and re-index modified files automatically (deferred to Phase 5)

### RAG Integration
- [x] In deep tier LLM path: embed user query → search `documents` collection → inject top-3 chunks into prompt
- [x] Prompt template: `[History]\n{turns}\n\n[Context]\n{chunks}\n\n[User]\n{query}`
- [x] If no relevant chunks found (score < threshold): proceed without context, log miss

### Conversation History
- [x] On each turn: store `{ node_id, role, content, timestamp_us }` in `history` collection
- [x] On each request: retrieve last N turns for `node_id`, prepend to prompt as chat history
- [x] N configurable via env var (`HISTORY_TURNS`, default 10)
- [x] Expose history clear endpoint (`clear_history()` — used by web UI and panic button long-press)

### Tests
- [x] Unit test: document chunking — correct chunk sizes and overlap
- [x] Unit test: RAG prompt assembly — chunks injected in correct position
- [x] Unit test: history retrieval — returns last N turns in chronological order
- [x] Integration test: two simultaneous sessions with independent histories — no cross-contamination

## Done When
PR merged to master with CI green and a RAG-grounded document query plus persistent conversation history verified end-to-end.
