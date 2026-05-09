use crate::web_ui::{json_error, AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct NodeInfo {
    pub node_id: String,
    pub state: String,
}

pub async fn list(State(state): State<AppState>) -> Json<Vec<NodeInfo>> {
    let sessions = state.registry.snapshot().await;
    Json(
        sessions
            .iter()
            .map(|s| NodeInfo {
                node_id: s.node_id.clone(),
                state: format!("{:?}", s.state),
            })
            .collect(),
    )
}

#[derive(Deserialize)]
pub struct PairConfirm {
    pub node_id: String,
}

pub async fn confirm_pair(
    State(state): State<AppState>,
    Json(body): Json<PairConfirm>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let local_ip = local_ip_address::local_ip()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;
    crate::pair::ensure_certs(&state.certs_dir, local_ip)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    let ca_key_pem = std::fs::read_to_string(state.certs_dir.join("ca-key.pem"))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;
    let ca_key = rcgen::KeyPair::from_pem(&ca_key_pem)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    let issued = crate::pair::issue_client_cert(&body.node_id, &ca_key)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    tracing::info!(node_id = %body.node_id, "client cert issued via web UI");

    Ok(Json(serde_json::json!({
        "node_id": body.node_id,
        "client_key_pem": issued.key_pem,
        "client_cert_pem": issued.cert_pem,
    })))
}

pub async fn unpair(Path(node_id): Path<String>, State(state): State<AppState>) -> StatusCode {
    state.registry.unregister(&node_id).await;
    tracing::info!(node_id = %node_id, "node unpaired via web UI");
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct RenameBody {
    #[allow(dead_code)]
    pub display_name: String,
}

pub async fn rename(
    Path(_node_id): Path<String>,
    State(_state): State<AppState>,
    Json(_body): Json<RenameBody>,
) -> StatusCode {
    StatusCode::NO_CONTENT
}
