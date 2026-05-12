use crate::web_ui::{render, AppResult, AppState};
use axum::{extract::State, response::Html};
use minijinja::context;

pub async fn tts_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let settings = state.tts_settings.read().await.clone();
    render(
        &state,
        "settings_tts.html",
        context! {
            active => "settings_tts",
            speed => settings.speed,
            voice => settings.voice,
            tts_enabled => state.tts.is_some(),
        },
    )
}

pub async fn models_handler(State(state): State<AppState>) -> AppResult<Html<String>> {
    let settings = state.model_settings.read().await.clone();
    let update = state.ollama_update.read().await.clone();
    let ollama_models = fetch_ollama_models(&state.ollama_url);
    render(
        &state,
        "settings_models.html",
        context! {
            active => "settings_models",
            settings => serde_json::to_value(&settings).unwrap_or_default(),
            ollama_models => ollama_models,
            ollama_update => serde_json::to_value(&update).unwrap_or_default(),
        },
    )
}

fn fetch_ollama_models(ollama_url: &str) -> Vec<serde_json::Value> {
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(format!("{ollama_url}/api/tags"))
        .timeout(std::time::Duration::from_secs(5))
        .send();
    match resp {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .ok()
            .and_then(|v| v["models"].as_array().cloned())
            .unwrap_or_default(),
        _ => vec![],
    }
}
