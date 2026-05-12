use crate::web_ui::{load_paired_nodes, load_wizard_state, render, AppResult, AppState, WizardStage};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let sessions = state.registry.snapshot().await;
    let paired = load_paired_nodes(&state.config_dir);
    let wizard = load_wizard_state(&state.config_dir);
    let setup_complete = wizard.stage == WizardStage::Complete;
    let update = state.ollama_update.read().await.clone();

    let mut nodes: Vec<serde_json::Value> = paired
        .into_iter()
        .map(|p| {
            let live = sessions.iter().find(|s| s.node_id == p.node_id);
            serde_json::json!({
                "node_id": p.node_id,
                "state": live.map(|s| format!("{:?}", s.state)).unwrap_or_else(|| "Offline".to_string()),
                "online": live.is_some(),
            })
        })
        .collect();

    for s in &sessions {
        if !nodes.iter().any(|n| n["node_id"] == s.node_id) {
            nodes.push(serde_json::json!({
                "node_id": s.node_id,
                "state": format!("{:?}", s.state),
                "online": true,
            }));
        }
    }

    render(
        &state,
        "dashboard.html",
        context! {
            active => "dashboard",
            nodes => nodes,
            setup_complete => setup_complete,
            ollama_update_available => update.update_available,
            ollama_latest => update.latest_version.unwrap_or_default(),
        },
    )
}
