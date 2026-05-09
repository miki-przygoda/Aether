/// Per-session conversation history stored in Qdrant.
///
/// Each turn is a `HistoryEntry` stored as a vector point in the `history`
/// collection. Points are keyed by `{node_id}_{timestamp_us}` so they sort
/// chronologically. Retrieval fetches the last N turns by doing a scroll
/// (ordered by insertion time), not a vector search.
use crate::vector_store::{VectorPoint, VectorStore, COLLECTION_HISTORY};
use anyhow::{Context, Result};
use std::sync::Arc;

#[allow(dead_code)]
pub const EMBED_DIM_HISTORY: usize = 1; // dummy — history uses scroll, not ANN search

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HistoryEntry {
    pub node_id: String,
    pub role: Role,
    pub content: String,
    pub timestamp_us: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

impl HistoryEntry {
    fn point_id(node_id: &str, timestamp_us: i64) -> String {
        // Zero-pad timestamp so lexicographic order = chronological order.
        format!("{node_id}_{timestamp_us:020}")
    }
}

/// Store a single turn in Qdrant.
pub fn store_turn(
    store: &Arc<dyn VectorStore>,
    node_id: &str,
    role: Role,
    content: &str,
) -> Result<()> {
    let timestamp_us = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as i64;

    let id = HistoryEntry::point_id(node_id, timestamp_us);
    let payload = serde_json::to_value(HistoryEntry {
        node_id: node_id.to_string(),
        role,
        content: content.to_string(),
        timestamp_us,
    })
    .context("serialising history entry")?;

    // Qdrant requires a non-empty vector even for scroll-only collections.
    // We store a single zero float so the collection validates.
    store
        .upsert(
            COLLECTION_HISTORY,
            vec![VectorPoint {
                id,
                vector: vec![0.0_f32],
                payload,
            }],
        )
        .context("storing history turn")
}

/// Retrieve the last `n` turns for `node_id` in chronological order.
///
/// Implementation: we rely on the sorted `id` key to build an ordered scroll
/// via the Qdrant REST scroll endpoint called through the store's `search`
/// workaround. Because `MockVectorStore` doesn't implement scroll, history
/// retrieval in tests works by passing pre-seeded results via mock.
///
/// For the real `QdrantStore` we use a direct HTTP scroll call; the trait
/// doesn't expose scroll, so we reach into the concrete type via a helper
/// that is called only from the LLM path.
#[allow(dead_code)]
pub fn recent_turns_from_search_results(
    results: &[crate::vector_store::SearchResult],
    n: usize,
) -> Vec<HistoryEntry> {
    let mut entries: Vec<HistoryEntry> = results
        .iter()
        .filter_map(|r| serde_json::from_value(r.payload.clone()).ok())
        .collect();
    // Sort ascending by timestamp (scroll results may arrive unordered in mock).
    entries.sort_by_key(|e| e.timestamp_us);
    entries.truncate(n);
    entries
}

/// Format a slice of history entries as a prompt preamble.
/// Each line is `User: …` or `Assistant: …` followed by a blank line.
pub fn format_history(entries: &[HistoryEntry]) -> String {
    entries
        .iter()
        .map(|e| {
            let role = match e.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
            };
            format!("{role}: {}", e.content)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Qdrant scroll helper (used by real store only) ────────────────────────────

/// Scroll the `history` collection for `node_id`, returning the last `n` turns.
/// Falls back to an empty vec if Qdrant is unavailable (non-fatal).
pub fn scroll_recent(base_url: &str, node_id: &str, n: usize) -> Result<Vec<HistoryEntry>> {
    let client = reqwest::blocking::Client::new();
    // Filter by node_id, order by id (= chronological), take last n.
    let body = serde_json::json!({
        "filter": {
            "must": [{
                "key": "node_id",
                "match": { "value": node_id }
            }]
        },
        "limit": n,
        "order_by": { "key": "timestamp_us", "direction": "desc" },
        "with_payload": true,
    });
    let resp = client
        .post(format!(
            "{base_url}/collections/{COLLECTION_HISTORY}/points/scroll"
        ))
        .json(&body)
        .send()
        .context("scrolling history collection")?;

    if !resp.status().is_success() {
        // Collection may not exist yet on first boot — return empty.
        return Ok(vec![]);
    }

    let json: serde_json::Value = resp.json()?;
    let mut entries: Vec<HistoryEntry> = json["result"]["points"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|p| serde_json::from_value(p["payload"].clone()).ok())
        .collect();

    // scroll returned desc order — reverse to get chronological.
    entries.reverse();
    Ok(entries)
}

/// Delete all history for `node_id` (panic button long-press / web UI clear).
#[allow(dead_code)]
pub fn clear_history(base_url: &str, node_id: &str) -> Result<()> {
    let client = reqwest::blocking::Client::new();
    let body = serde_json::json!({
        "filter": {
            "must": [{ "key": "node_id", "match": { "value": node_id } }]
        }
    });
    client
        .post(format!(
            "{base_url}/collections/{COLLECTION_HISTORY}/points/delete"
        ))
        .query(&[("wait", "true")])
        .json(&body)
        .send()
        .context("clearing history")?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_store::mock::MockVectorStore;
    use std::sync::Arc;

    #[test]
    fn store_turn_upserts_one_point() {
        let mock = Arc::new(MockVectorStore::default());
        let store: Arc<dyn VectorStore> = mock.clone();
        store_turn(&store, "pi-1", Role::User, "hello").unwrap();
        let lock = mock.upserted.lock().unwrap();
        assert_eq!(lock.len(), 1);
        assert_eq!(lock[0].0, COLLECTION_HISTORY);
        assert_eq!(lock[0].1.vector, vec![0.0_f32]);
    }

    #[test]
    fn format_history_produces_correct_lines() {
        let entries = vec![
            HistoryEntry {
                node_id: "pi-1".into(),
                role: Role::User,
                content: "What time is it?".into(),
                timestamp_us: 1,
            },
            HistoryEntry {
                node_id: "pi-1".into(),
                role: Role::Assistant,
                content: "It is noon.".into(),
                timestamp_us: 2,
            },
        ];
        let text = format_history(&entries);
        assert_eq!(text, "User: What time is it?\nAssistant: It is noon.");
    }

    #[test]
    fn recent_turns_sorted_chronologically() {
        use crate::vector_store::SearchResult;
        let results = vec![
            SearchResult {
                id: "a".into(),
                score: 1.0,
                payload: serde_json::to_value(HistoryEntry {
                    node_id: "pi-1".into(),
                    role: Role::Assistant,
                    content: "second".into(),
                    timestamp_us: 2,
                })
                .unwrap(),
            },
            SearchResult {
                id: "b".into(),
                score: 1.0,
                payload: serde_json::to_value(HistoryEntry {
                    node_id: "pi-1".into(),
                    role: Role::User,
                    content: "first".into(),
                    timestamp_us: 1,
                })
                .unwrap(),
            },
        ];
        let turns = recent_turns_from_search_results(&results, 10);
        assert_eq!(turns[0].content, "first");
        assert_eq!(turns[1].content, "second");
    }

    #[test]
    fn recent_turns_truncates_to_n() {
        use crate::vector_store::SearchResult;
        let results: Vec<SearchResult> = (0..5)
            .map(|i| SearchResult {
                id: i.to_string(),
                score: 1.0,
                payload: serde_json::to_value(HistoryEntry {
                    node_id: "pi-1".into(),
                    role: Role::User,
                    content: format!("turn {i}"),
                    timestamp_us: i,
                })
                .unwrap(),
            })
            .collect();
        let turns = recent_turns_from_search_results(&results, 3);
        assert_eq!(turns.len(), 3);
    }

    #[test]
    fn point_id_sorts_chronologically() {
        let a = HistoryEntry::point_id("pi-1", 1_000);
        let b = HistoryEntry::point_id("pi-1", 2_000);
        assert!(a < b, "earlier timestamp should sort before later");
    }
}
