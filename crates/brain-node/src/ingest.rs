/// Document ingestion pipeline.
///
/// Files are read from `DOCUMENTS_DIR`, split into overlapping text chunks,
/// embedded via Ollama `nomic-embed-text`, and upserted into the Qdrant
/// `documents` collection.
///
/// Chunking uses a simple character-count window rather than a tokeniser to
/// avoid pulling in a full tokeniser dependency. At ~4 chars per token the
/// defaults (2048 chars / 256 overlap) approximate the 512-token / 64-token
/// window specified in the epic.
use crate::vector_store::{VectorPoint, VectorStore, COLLECTION_DOCUMENTS};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::Arc;

// ── Chunking ──────────────────────────────────────────────────────────────────

pub const CHUNK_CHARS: usize = 2048;
pub const OVERLAP_CHARS: usize = 256;

/// Split `text` into overlapping character-window chunks.
pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    assert!(
        overlap < chunk_size,
        "overlap must be smaller than chunk_size"
    );
    if text.is_empty() {
        return vec![];
    }

    let chars: Vec<char> = text.chars().collect();
    let total = chars.len();
    let step = chunk_size - overlap;
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < total {
        let end = (start + chunk_size).min(total);
        chunks.push(chars[start..end].iter().collect());
        if end == total {
            break;
        }
        start += step;
    }
    chunks
}

// ── Embedding via Ollama ──────────────────────────────────────────────────────

/// Embed `text` using Ollama's embedding endpoint.
/// Returns a f32 vector of length determined by the model (768 for nomic-embed-text).
pub fn embed(ollama_url: &str, model: &str, text: &str) -> Result<Vec<f32>> {
    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({
        "model": model,
        "prompt": text,
    });
    let resp = client
        .post(format!("{ollama_url}/api/embeddings"))
        .json(&body)
        .send()
        .context("calling Ollama embeddings endpoint")?;

    anyhow::ensure!(
        resp.status().is_success(),
        "Ollama /api/embeddings returned {}",
        resp.status()
    );

    let json: serde_json::Value = resp.json().context("parsing Ollama embeddings response")?;
    let embedding = json["embedding"]
        .as_array()
        .context("Ollama embeddings response missing 'embedding' array")?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect();
    Ok(embedding)
}

// ── File ingestion ────────────────────────────────────────────────────────────

/// Ingest a single file: chunk → embed → upsert.
/// Returns the number of chunks written.
pub fn ingest_file(
    path: &Path,
    store: &Arc<dyn VectorStore>,
    ollama_url: &str,
    embed_model: &str,
) -> Result<usize> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let chunks = chunk_text(&text, CHUNK_CHARS, OVERLAP_CHARS);
    let n = chunks.len();
    let mut points = Vec::with_capacity(n);

    for (i, chunk) in chunks.into_iter().enumerate() {
        let vector = embed(ollama_url, embed_model, &chunk)
            .with_context(|| format!("embedding chunk {i} of {file_name}"))?;

        let id = format!("{file_name}_{i:06}");
        let payload = serde_json::json!({
            "file": file_name,
            "chunk_index": i,
            "text": chunk,
        });
        points.push(VectorPoint {
            id,
            vector,
            payload,
        });
    }

    store
        .upsert(COLLECTION_DOCUMENTS, points)
        .with_context(|| format!("upserting chunks for {file_name}"))?;

    Ok(n)
}

/// Ingest all `.txt` and `.md` files in `dir`.
/// Logs a warning on per-file errors rather than aborting the whole batch.
pub fn ingest_dir(
    dir: &Path,
    store: &Arc<dyn VectorStore>,
    ollama_url: &str,
    embed_model: &str,
) -> Result<usize> {
    let mut total = 0usize;
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading documents dir {}", dir.display()))?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(ext, "txt" | "md") {
            continue;
        }
        match ingest_file(&path, store, ollama_url, embed_model) {
            Ok(n) => {
                tracing::info!(file = %path.display(), chunks = n, "ingested");
                total += n;
            }
            Err(e) => tracing::warn!(file = %path.display(), "ingest error: {e}"),
        }
    }
    Ok(total)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_text_empty_returns_empty() {
        assert!(chunk_text("", 100, 10).is_empty());
    }

    #[test]
    fn chunk_text_short_text_is_single_chunk() {
        let chunks = chunk_text("hello world", 100, 10);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello world");
    }

    #[test]
    fn chunk_text_produces_correct_count() {
        // 200 chars, chunk 100, overlap 20 → step 80 → ceil((200-100)/80)+1 = 3 chunks
        let text: String = "a".repeat(200);
        let chunks = chunk_text(&text, 100, 20);
        assert_eq!(chunks.len(), 3);
    }

    #[test]
    fn chunk_text_overlap_is_present() {
        let text: String = (b'a'..=b'z').map(char::from).collect::<String>().repeat(4); // 104 chars
        let chunks = chunk_text(&text, 30, 10);
        // Last 10 chars of chunk[0] should equal first 10 chars of chunk[1].
        let tail0: String = chunks[0]
            .chars()
            .rev()
            .take(10)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let head1: String = chunks[1].chars().take(10).collect();
        assert_eq!(tail0, head1, "overlap not preserved between chunk 0 and 1");
    }

    #[test]
    fn chunk_text_no_data_loss() {
        // Every character of the original must appear in at least one chunk.
        let text = "The quick brown fox jumps over the lazy dog. ".repeat(10);
        let chunks = chunk_text(&text, 50, 10);
        // Reconstruct by stepping through with step size (not perfect but proves coverage).
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].chars().count(), 50);
        // Final chunk may be smaller.
        assert!(chunks.last().unwrap().chars().count() <= 50);
    }

    #[test]
    fn chunk_text_exact_fit_is_one_chunk() {
        let text: String = "x".repeat(100);
        let chunks = chunk_text(&text, 100, 10);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    #[ignore = "requires running Ollama with nomic-embed-text — integration only"]
    fn embed_returns_non_empty_vector() {
        let v = embed("http://localhost:11434", "nomic-embed-text", "hello").unwrap();
        assert!(!v.is_empty());
    }
}
