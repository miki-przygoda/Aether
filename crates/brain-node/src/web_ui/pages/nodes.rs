use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn list_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let sessions = state.registry.snapshot().await;
    let nodes: Vec<serde_json::Value> = sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "node_id": s.node_id,
                "state": format!("{:?}", s.state),
            })
        })
        .collect();
    render(&state, "nodes.html", context! { active => "nodes", nodes => nodes })
}

pub async fn pair_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    render(
        &state,
        "nodes_pair.html",
        context! { active => "nodes" },
    )
}
