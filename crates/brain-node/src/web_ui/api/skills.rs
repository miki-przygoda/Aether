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
    let trie = state.trie.clone();
    let skills = state.skills.clone();
    let llm = state.llm.clone();
    let query = body.query.clone();

    let result = tokio::task::spawn_blocking(move || {
        use aether_core::trie::ClassifyResult;
        match trie.classify(&query) {
            ClassifyResult::Match(action) => {
                let action_str = action.as_str().to_string();
                let params = serde_json::Value::Object(Default::default());
                let result = skills.dispatch(&action_str, &params);
                Ok(serde_json::json!({
                    "matched": "trie",
                    "action": action_str,
                    "spoken_reply": result.spoken_reply,
                    "params": params,
                    "latency_source": "trie",
                }))
            }
            _ => {
                if let Some(llm) = llm {
                    match llm.ask(&query) {
                        Ok(resp) => {
                            let action = resp.action.unwrap_or_else(|| "respond".to_string());
                            let mut params = resp
                                .params
                                .unwrap_or(serde_json::Value::Object(Default::default()));
                            params["response"] = serde_json::Value::String(resp.response);
                            let result = skills.dispatch(&action, &params);
                            Ok(serde_json::json!({
                                "matched": "llm",
                                "action": action,
                                "spoken_reply": result.spoken_reply,
                                "params": params,
                                "latency_source": "ollama",
                            }))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                } else {
                    Err("LLM not configured".to_string())
                }
            }
        }
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, json_error(e.to_string())))?;

    result
        .map(Json)
        .map_err(|e| (StatusCode::BAD_GATEWAY, json_error(e)))
}
