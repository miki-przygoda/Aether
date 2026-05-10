use crate::web_ui::{load_paired_nodes, load_wizard_state, render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let wizard = load_wizard_state(&state.config_dir);
    let paired = load_paired_nodes(&state.config_dir);

    let brain_ip = local_ip_address::local_ip()
        .ok()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "<brain-ip>".to_string());

    let pair_port = 50052u16;
    let grpc_port = 50051u16;

    let paired_nodes: Vec<serde_json::Value> = paired
        .iter()
        .map(|p| serde_json::json!({ "node_id": p.node_id }))
        .collect();

    render(
        &state,
        "setup.html",
        context! {
            active => "setup",
            stage => wizard.stage,
            stage_index => wizard.stage.index(),
            target_node_id => wizard.target_node_id,
            wake_model_path => wizard.wake_model_path,
            paired_nodes => paired_nodes,
            brain_ip => brain_ip,
            pair_port => pair_port,
            grpc_port => grpc_port,
            config_dir => state.config_dir.to_string_lossy().to_string(),
        },
    )
}
