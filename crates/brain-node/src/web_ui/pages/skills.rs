use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let skills = state.skills.list();
    let skills_json: Vec<serde_json::Value> = skills
        .iter()
        .map(|s| {
            serde_json::json!({
                "action": s.action,
                "description": s.description,
                "example_phrases": s.example_phrases,
            })
        })
        .collect();
    render(
        &state,
        "skills.html",
        context! { active => "skills", skills => skills_json },
    )
}
