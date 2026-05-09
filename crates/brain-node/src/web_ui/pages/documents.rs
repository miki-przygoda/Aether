use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let docs = list_documents(&state);
    let sessions = state.registry.snapshot().await;
    let node_ids: Vec<String> = sessions.iter().map(|s| s.node_id.clone()).collect();
    let qdrant_configured = state.rag.is_some();
    render(
        &state,
        "documents.html",
        context! {
            active => "documents",
            documents => docs,
            node_ids => node_ids,
            qdrant_configured => qdrant_configured,
        },
    )
}

fn list_documents(state: &AppState) -> Vec<serde_json::Value> {
    let Some(ref dir) = state.documents_dir else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file()
                && matches!(
                    e.path().extension().and_then(|x| x.to_str()),
                    Some("txt" | "md")
                )
        })
        .map(|e| {
            let meta = e.metadata().ok();
            serde_json::json!({
                "name": e.file_name().to_string_lossy(),
                "size_kb": meta.as_ref().map(|m| m.len() / 1024).unwrap_or(0),
                "status": "indexed",
            })
        })
        .collect()
}
