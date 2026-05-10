use crate::web_ui::{
    json_error, load_paired_nodes, load_wizard_state, save_wizard_state, AppState, WizardStage,
};
use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct SetupStatus {
    pub stage: WizardStage,
    pub stage_index: usize,
    pub target_node_id: Option<String>,
    pub wake_model_path: Option<String>,
    pub brain_healthy: bool,
    pub node_paired: bool,
    pub wake_model_ready: bool,
    pub node_online: bool,
    pub complete: bool,
}

pub async fn get_status(State(state): State<AppState>) -> Json<SetupStatus> {
    let wizard = load_wizard_state(&state.config_dir);
    let paired = load_paired_nodes(&state.config_dir);
    let sessions = state.registry.snapshot().await;

    let brain_healthy = check_brain_health(&state).await;

    let node_paired = wizard
        .target_node_id
        .as_ref()
        .map(|id| paired.iter().any(|p| &p.node_id == id))
        .unwrap_or(!paired.is_empty());

    let wake_model_ready = wizard
        .wake_model_path
        .as_ref()
        .map(|p| std::path::Path::new(p).exists())
        .unwrap_or(false);

    let node_online = wizard
        .target_node_id
        .as_ref()
        .map(|id| sessions.iter().any(|s| &s.node_id == id))
        .unwrap_or(false);

    let complete = wizard.stage == WizardStage::Complete;
    let stage_index = wizard.stage.index();

    Json(SetupStatus {
        stage: wizard.stage,
        stage_index,
        target_node_id: wizard.target_node_id,
        wake_model_path: wizard.wake_model_path,
        brain_healthy,
        node_paired,
        wake_model_ready,
        node_online,
        complete,
    })
}

async fn check_brain_health(state: &AppState) -> bool {
    let certs_ok = state.certs_dir.join("ca.pem").exists();
    let ollama_ok = reqwest::Client::new()
        .get(format!("{}/api/tags", state.ollama_url))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);
    certs_ok && ollama_ok
}

pub async fn advance_stage(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let mut wizard = load_wizard_state(&state.config_dir);
    match wizard.stage.next() {
        Some(next) => {
            wizard.stage = next;
            save_wizard_state(&state.config_dir, &wizard);
            Ok(Json(serde_json::json!({ "stage": wizard.stage })))
        }
        None => Err((
            StatusCode::BAD_REQUEST,
            json_error("wizard already complete"),
        )),
    }
}

pub async fn set_target_node(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let node_id = body["node_id"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, json_error("node_id required")))?
        .to_string();
    let mut wizard = load_wizard_state(&state.config_dir);
    wizard.target_node_id = Some(node_id.clone());
    save_wizard_state(&state.config_dir, &wizard);
    Ok(Json(serde_json::json!({ "node_id": node_id })))
}

pub async fn set_wake_model(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let path = body["path"]
        .as_str()
        .ok_or_else(|| (StatusCode::BAD_REQUEST, json_error("path required")))?
        .to_string();
    let mut wizard = load_wizard_state(&state.config_dir);
    wizard.wake_model_path = Some(path.clone());
    save_wizard_state(&state.config_dir, &wizard);
    Ok(Json(serde_json::json!({ "path": path })))
}

pub async fn reset(State(state): State<AppState>) -> StatusCode {
    save_wizard_state(&state.config_dir, &Default::default());
    StatusCode::NO_CONTENT
}
