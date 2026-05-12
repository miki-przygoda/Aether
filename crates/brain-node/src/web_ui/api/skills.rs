use crate::skills::SkillContext;
use crate::web_ui::{json_error, AppState};
use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;

pub async fn list(State(state): State<AppState>) -> Json<Vec<crate::skills::SkillInfo>> {
    Json(state.skills.list())
}

#[derive(Deserialize)]
pub struct TestBody {
    pub query: String,
}

pub async fn test(
    State(state): State<AppState>,
    Json(body): Json<TestBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    use aether_core::trie::ClassifyResult;

    let cfg = state.skill_config.read().await.clone();
    let ctx = SkillContext {
        node_id: "skill-tester",
        http_client: &state.http_client,
        config: &cfg,
        registry: &state.registry,
    };

    match state.trie.classify(&body.query) {
        ClassifyResult::Match(action) => {
            let action_str = action.as_str().to_string();
            let params = serde_json::Value::Object(Default::default());
            let result = state.skills.dispatch(&action_str, &params, &ctx).await;
            Ok(Json(serde_json::json!({
                "matched": "trie",
                "action": action_str,
                "spoken_reply": result.spoken_reply,
                "params": params,
                "latency_source": "trie",
            })))
        }
        _ => {
            let Some(llm) = state.llm.clone() else {
                return Err((StatusCode::BAD_GATEWAY, json_error("LLM not configured".to_string())));
            };
            let query = body.query.clone();
            let resp = tokio::task::spawn_blocking(move || llm.ask(&query))
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?
                .map_err(|e| (StatusCode::BAD_GATEWAY, json_error(e.to_string())))?;

            let action = resp.action.unwrap_or_else(|| "respond".to_string());
            let mut params = resp.params.unwrap_or(serde_json::Value::Object(Default::default()));
            params["response"] = serde_json::Value::String(resp.response);
            let result = state.skills.dispatch(&action, &params, &ctx).await;
            Ok(Json(serde_json::json!({
                "matched": "llm",
                "action": action,
                "spoken_reply": result.spoken_reply,
                "params": params,
                "latency_source": "ollama",
            })))
        }
    }
}
