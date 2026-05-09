/// Abstraction over a vector database — allows mock injection in tests.
use anyhow::Result;

#[derive(Debug, Clone)]
pub struct VectorPoint {
    /// Stable identifier for this chunk (UUID or hash of file+offset).
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    #[allow(dead_code)]
    pub id: String,
    pub score: f32,
    pub payload: serde_json::Value,
}

pub trait VectorStore: Send + Sync {
    fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()>;
    fn search(&self, collection: &str, query: Vec<f32>, top_k: usize) -> Result<Vec<SearchResult>>;
    #[allow(dead_code)]
    fn delete(&self, collection: &str, id: &str) -> Result<()>;
    fn ensure_collection(&self, collection: &str, vector_size: usize) -> Result<()>;
}

// ── Qdrant REST client ────────────────────────────────────────────────────────

pub struct QdrantStore {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl QdrantStore {
    pub fn new(base_url: &str) -> Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

impl VectorStore for QdrantStore {
    fn ensure_collection(&self, collection: &str, vector_size: usize) -> Result<()> {
        // Check if collection already exists.
        let resp = self
            .client
            .get(self.url(&format!("/collections/{collection}")))
            .send()?;
        if resp.status().is_success() {
            return Ok(());
        }

        // Create collection with cosine distance (standard for text embeddings).
        let body = serde_json::json!({
            "vectors": {
                "size": vector_size,
                "distance": "Cosine"
            }
        });
        let resp = self
            .client
            .put(self.url(&format!("/collections/{collection}")))
            .json(&body)
            .send()?;
        anyhow::ensure!(
            resp.status().is_success(),
            "Qdrant create_collection failed: {}",
            resp.status()
        );
        Ok(())
    }

    fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()> {
        let qdrant_points: Vec<serde_json::Value> = points
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "vector": p.vector,
                    "payload": p.payload,
                })
            })
            .collect();
        let body = serde_json::json!({ "points": qdrant_points });
        let resp = self
            .client
            .put(self.url(&format!("/collections/{collection}/points")))
            .query(&[("wait", "true")])
            .json(&body)
            .send()?;
        anyhow::ensure!(
            resp.status().is_success(),
            "Qdrant upsert failed: {}",
            resp.status()
        );
        Ok(())
    }

    fn search(&self, collection: &str, query: Vec<f32>, top_k: usize) -> Result<Vec<SearchResult>> {
        let body = serde_json::json!({
            "vector": query,
            "limit": top_k,
            "with_payload": true,
        });
        let resp = self
            .client
            .post(self.url(&format!("/collections/{collection}/points/search")))
            .json(&body)
            .send()?;
        anyhow::ensure!(
            resp.status().is_success(),
            "Qdrant search failed: {}",
            resp.status()
        );
        let body: serde_json::Value = resp.json()?;
        let results = body["result"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|r| SearchResult {
                id: r["id"].as_str().unwrap_or("").to_string(),
                score: r["score"].as_f64().unwrap_or(0.0) as f32,
                payload: r["payload"].clone(),
            })
            .collect();
        Ok(results)
    }

    fn delete(&self, collection: &str, id: &str) -> Result<()> {
        let body = serde_json::json!({
            "points": [id]
        });
        let resp = self
            .client
            .post(self.url(&format!("/collections/{collection}/points/delete")))
            .query(&[("wait", "true")])
            .json(&body)
            .send()?;
        anyhow::ensure!(
            resp.status().is_success(),
            "Qdrant delete failed: {}",
            resp.status()
        );
        Ok(())
    }
}

// ── Collection name constants ─────────────────────────────────────────────────

pub const COLLECTION_DOCUMENTS: &str = "documents";
pub const COLLECTION_HISTORY: &str = "history";

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MockVectorStore {
        pub upserted: Mutex<Vec<(String, VectorPoint)>>,
        pub searches: Mutex<Vec<(String, Vec<f32>)>>,
        pub search_results: Vec<SearchResult>,
    }

    impl MockVectorStore {
        pub fn with_results(results: Vec<SearchResult>) -> Self {
            Self {
                search_results: results,
                ..Default::default()
            }
        }
    }

    impl VectorStore for MockVectorStore {
        fn ensure_collection(&self, _collection: &str, _vector_size: usize) -> Result<()> {
            Ok(())
        }
        fn upsert(&self, collection: &str, points: Vec<VectorPoint>) -> Result<()> {
            let mut lock = self.upserted.lock().unwrap();
            for p in points {
                lock.push((collection.to_string(), p));
            }
            Ok(())
        }
        fn search(
            &self,
            collection: &str,
            query: Vec<f32>,
            _top_k: usize,
        ) -> Result<Vec<SearchResult>> {
            self.searches
                .lock()
                .unwrap()
                .push((collection.to_string(), query));
            Ok(self.search_results.clone())
        }
        fn delete(&self, _collection: &str, _id: &str) -> Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mock::MockVectorStore;

    #[test]
    fn mock_upsert_records_points() {
        let store = MockVectorStore::default();
        store
            .upsert(
                COLLECTION_DOCUMENTS,
                vec![VectorPoint {
                    id: "abc".into(),
                    vector: vec![0.1, 0.2],
                    payload: serde_json::json!({ "text": "hello" }),
                }],
            )
            .unwrap();
        let lock = store.upserted.lock().unwrap();
        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].1.id, "abc");
    }

    #[test]
    fn mock_search_returns_preset_results() {
        let store = MockVectorStore::with_results(vec![SearchResult {
            id: "x".into(),
            score: 0.9,
            payload: serde_json::json!({ "text": "chunk content" }),
        }]);
        let results = store.search(COLLECTION_DOCUMENTS, vec![0.1], 3).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "x");
        assert!((results[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn mock_delete_does_not_panic() {
        let store = MockVectorStore::default();
        store.delete(COLLECTION_DOCUMENTS, "xyz").unwrap();
    }
}
