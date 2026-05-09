use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn wake_word_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let training = state.wake_training.lock().await;
    let sessions = state.registry.snapshot().await;
    let node_ids: Vec<String> = sessions.iter().map(|s| s.node_id.clone()).collect();
    let samples_json: Vec<serde_json::Value> = training
        .samples
        .iter()
        .map(|s| serde_json::to_value(s).unwrap_or_default())
        .collect();
    let status_json = serde_json::to_value(&training.status).unwrap_or_default();
    render(
        &state,
        "training_wake_word.html",
        context! {
            active => "training_wakeword",
            node_ids => node_ids,
            samples => samples_json,
            status => status_json,
            sample_count => training.samples.len(),
        },
    )
}

pub async fn voice_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let training = state.voice_training.lock().await;
    let users_json: Vec<serde_json::Value> = training
        .users
        .iter()
        .map(|u| serde_json::to_value(u).unwrap_or_default())
        .collect();
    let finetuning_available = state.finetuning_url.is_some();
    render(
        &state,
        "training_voice.html",
        context! {
            active => "training_voice",
            users => users_json,
            finetuning_available => finetuning_available,
        },
    )
}
