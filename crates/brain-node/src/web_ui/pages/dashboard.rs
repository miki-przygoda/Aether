use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn handler(State(state): State<AppState>) -> AppResult<Html<String>> {
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
    render(
        &state,
        "dashboard.html",
        context! { active => "dashboard", nodes => nodes },
    )
}
