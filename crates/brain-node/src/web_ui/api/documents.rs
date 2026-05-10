use crate::web_ui::{json_error, AppState, ProgressEvent};
use axum::{
    extract::{Multipart, Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;

#[derive(Serialize)]
pub struct DocumentInfo {
    pub name: String,
    pub size_kb: u64,
    pub status: String,
}

pub async fn list(State(state): State<AppState>) -> Json<Vec<DocumentInfo>> {
    let docs = list_from_dir(&state);
    Json(docs)
}

fn list_from_dir(state: &AppState) -> Vec<DocumentInfo> {
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
            DocumentInfo {
                name: e.file_name().to_string_lossy().to_string(),
                size_kb: meta.as_ref().map(|m| m.len() / 1024).unwrap_or(0),
                status: "indexed".to_string(),
            }
        })
        .collect()
}

pub async fn upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let dir = state.documents_dir.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json_error("DOCUMENTS_DIR not configured"),
        )
    })?;

    let mut uploaded = vec![];
    while let Ok(Some(field)) = multipart.next_field().await {
        let filename = field.file_name().unwrap_or("upload.txt").to_string();
        let data = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, json_error(e.to_string())))?;
        let safe_name: String = filename
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '.' || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dest = dir.join(&safe_name);
        std::fs::write(&dest, &data)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;
        uploaded.push(safe_name);
    }

    Ok(Json(serde_json::json!({ "uploaded": uploaded })))
}

pub async fn trigger_ingest(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let rag = state.rag.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json_error("Qdrant not configured"),
        )
    })?;
    let dir = state.documents_dir.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json_error("DOCUMENTS_DIR not configured"),
        )
    })?;
    let tx = state.ingest_progress_tx.clone();

    tokio::task::spawn_blocking(move || {
        let _ = tx.send(ProgressEvent {
            percent: 0,
            message: "Starting ingestion…".to_string(),
            ..Default::default()
        });
        match crate::ingest::ingest_dir(&dir, &rag.store, &rag.embed_url, &rag.embed_model) {
            Ok(n) => {
                let _ = tx.send(ProgressEvent {
                    percent: 100,
                    message: format!("Ingested {n} chunks"),
                    done: true,
                    ..Default::default()
                });
            }
            Err(e) => {
                let _ = tx.send(ProgressEvent {
                    percent: 0,
                    message: format!("Error: {e}"),
                    done: true,
                    error: true,
                });
            }
        }
    });

    Ok(Json(serde_json::json!({ "status": "ingestion started" })))
}

pub async fn clear_history(
    Path(node_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let rag = state.rag.clone().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json_error("Qdrant not configured"),
        )
    })?;
    tokio::task::spawn_blocking(move || crate::history::clear_history(&rag.qdrant_url, &node_id));
    Ok(StatusCode::NO_CONTENT)
}
