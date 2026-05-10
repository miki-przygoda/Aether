use crate::web_ui::{json_error, load_paired_nodes, register_paired_node, remove_paired_node, AppState};
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
    pub online: bool,
    pub paired_at: Option<String>,
}

pub async fn list(State(state): State<AppState>) -> Json<Vec<NodeInfo>> {
    let sessions = state.registry.snapshot().await;
    let paired = load_paired_nodes(&state.config_dir);

    // Start from the persistent registry so offline nodes always appear.
    let mut nodes: Vec<NodeInfo> = paired
        .into_iter()
        .map(|p| {
            let live = sessions.iter().find(|s| s.node_id == p.node_id);
            NodeInfo {
                node_id: p.node_id,
                state: live
                    .map(|s| format!("{:?}", s.state))
                    .unwrap_or_else(|| "Offline".to_string()),
                online: live.is_some(),
                paired_at: Some(p.paired_at),
            }
        })
        .collect();

    // Include any live session that somehow isn't in the registry.
    for s in &sessions {
        if !nodes.iter().any(|n| n.node_id == s.node_id) {
            nodes.push(NodeInfo {
                node_id: s.node_id.clone(),
                state: format!("{:?}", s.state),
                online: true,
                paired_at: None,
            });
        }
    }

    Json(nodes)
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

    register_paired_node(&state.config_dir, &body.node_id);
    tracing::info!(node_id = %body.node_id, "client cert issued via web UI");

    Ok(Json(serde_json::json!({
        "node_id": body.node_id,
        "client_key_pem": issued.key_pem,
        "client_cert_pem": issued.cert_pem,
    })))
}

pub async fn unpair(Path(node_id): Path<String>, State(state): State<AppState>) -> StatusCode {
    state.registry.unregister(&node_id).await;
    remove_paired_node(&state.config_dir, &node_id);
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
