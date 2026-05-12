use crate::web_ui::{load_paired_nodes, render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn list_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let sessions = state.registry.snapshot().await;
    let paired = load_paired_nodes(&state.config_dir);

    let mut nodes: Vec<serde_json::Value> = paired
        .into_iter()
        .map(|p| {
            let live = sessions.iter().find(|s| s.node_id == p.node_id);
            serde_json::json!({
                "node_id": p.node_id,
                "state": live.map(|s| format!("{:?}", s.state)).unwrap_or_else(|| "Offline".to_string()),
                "online": live.is_some(),
                "paired_at": p.paired_at,
            })
        })
        .collect();

    for s in &sessions {
        if !nodes.iter().any(|n| n["node_id"] == s.node_id) {
            nodes.push(serde_json::json!({
                "node_id": s.node_id,
                "state": format!("{:?}", s.state),
                "online": true,
                "paired_at": null,
            }));
        }
    }

    render(
        &state,
        "nodes.html",
        context! { active => "nodes", nodes => nodes },
    )
}

pub async fn pair_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    render(&state, "nodes_pair.html", context! { active => "nodes" })
}
