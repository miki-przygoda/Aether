use crate::skills::SkillConfig;
use crate::web_ui::{json_error, save_skill_config, AppState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

pub async fn get(State(state): State<AppState>) -> Json<SkillConfig> {
    Json(state.skill_config.read().await.clone())
}

pub async fn save(
    State(state): State<AppState>,
    Json(body): Json<SkillConfig>,
) -> Result<Json<SkillConfig>, (StatusCode, Json<serde_json::Value>)> {
    *state.skill_config.write().await = body.clone();
    save_skill_config(&state.config_dir, &body);
    Ok(Json(body))
}

#[derive(Deserialize)]
pub struct LocationQuery {
    pub q: String,
}

/// Proxies to the Open-Meteo geocoding API.  The response is forwarded
/// verbatim so the frontend can render the result list.
pub async fn location_search(
    State(state): State<AppState>,
    Query(q): Query<LocationQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let base = "https://geocoding-api.open-meteo.com";
    let resp = state
        .http_client
        .get(format!("{base}/v1/search"))
        .query(&[("name", q.q.as_str()), ("count", "8"), ("language", "en")])
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, json_error(e.to_string())))?;

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, json_error(e.to_string())))?;

    Ok(Json(json))
}
