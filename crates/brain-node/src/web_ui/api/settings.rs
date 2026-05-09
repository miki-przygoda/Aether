use crate::web_ui::{json_error, AppState, ModelSettings};
use aether_core::TtsSettings;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

// ── TTS settings ──────────────────────────────────────────────────────────────

pub async fn get_tts(State(state): State<AppState>) -> Json<TtsSettings> {
    Json(state.tts_settings.read().await.clone())
}

#[derive(Deserialize)]
pub struct SaveTtsBody {
    pub speed: f32,
    pub voice: String,
}

pub async fn save_tts(
    State(state): State<AppState>,
    Json(body): Json<SaveTtsBody>,
) -> Result<Json<TtsSettings>, (StatusCode, Json<serde_json::Value>)> {
    let speed = body.speed.clamp(0.5, 2.0);
    let settings = TtsSettings { speed, voice: body.voice };
    *state.tts_settings.write().await = settings.clone();
    persist_tts(&state, &settings)?;
    Ok(Json(settings))
}

fn persist_tts(
    state: &AppState,
    settings: &TtsSettings,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let path = state.config_dir.join("tts_settings.json");
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;
    std::fs::write(&path, json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))
}

#[derive(Deserialize)]
pub struct TtsPreviewBody {
    pub text: String,
    pub speed: Option<f32>,
    pub voice: Option<String>,
}

pub async fn tts_preview(
    State(state): State<AppState>,
    Json(body): Json<TtsPreviewBody>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let tts = state.tts.clone().ok_or_else(|| {
        (StatusCode::SERVICE_UNAVAILABLE, json_error("TTS not configured"))
    })?;
    let current = state.tts_settings.read().await.clone();
    let settings = TtsSettings {
        speed: body.speed.unwrap_or(current.speed).clamp(0.5, 2.0),
        voice: body.voice.unwrap_or(current.voice),
    };
    let text = body.text.clone();
    let wav = tokio::task::spawn_blocking(move || tts.synthesise(&text, &settings))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    Ok(([(header::CONTENT_TYPE, "audio/wav")], wav))
}

// ── Model settings ────────────────────────────────────────────────────────────

pub async fn get_models(State(state): State<AppState>) -> Json<ModelSettings> {
    Json(state.model_settings.read().await.clone())
}

pub async fn save_models(
    State(state): State<AppState>,
    Json(body): Json<ModelSettings>,
) -> Result<Json<ModelSettings>, (StatusCode, Json<serde_json::Value>)> {
    let settings = body;
    *state.model_settings.write().await = settings.clone();
    let path = state.config_dir.join("model_settings.json");
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;
    std::fs::write(&path, json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;
    Ok(Json(settings))
}

pub async fn pull_model(
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ollama_url = state.ollama_url.clone();
    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;
        let resp = client
            .post(format!("{ollama_url}/api/pull"))
            .json(&serde_json::json!({ "name": name }))
            .send()?;
        anyhow::ensure!(resp.status().is_success(), "Ollama pull returned {}", resp.status());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?
    .map_err(|e| (StatusCode::BAD_GATEWAY, json_error(e.to_string())))?;

    Ok(Json(serde_json::json!({ "status": "pulled" })))
}

pub async fn remove_model(
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let ollama_url = state.ollama_url.clone();
    tokio::task::spawn_blocking(move || {
        let client = reqwest::blocking::Client::new();
        let resp = client
            .delete(format!("{ollama_url}/api/delete"))
            .json(&serde_json::json!({ "name": name }))
            .send()?;
        anyhow::ensure!(resp.status().is_success(), "Ollama delete returned {}", resp.status());
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?
    .map_err(|e| (StatusCode::BAD_GATEWAY, json_error(e.to_string())))?;

    Ok(Json(serde_json::json!({ "status": "deleted" })))
}
