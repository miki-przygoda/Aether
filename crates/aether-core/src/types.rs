use serde::{Deserialize, Serialize};

/// Outcome returned by a `Skill` handler.
#[derive(Debug, Clone)]
pub struct SkillResult {
    /// Text the TTS engine should speak back to the user.
    pub spoken_reply: String,
}

/// Structured response the LLM must emit for every turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub action: Option<String>,
    pub params: Option<serde_json::Value>,
    pub response: String,
}

/// State broadcast by the edge node; auxiliary nodes mirror this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeState {
    Idle,
    Listening,
    Processing,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_response_roundtrip() {
        let r = LlmResponse {
            action: Some("play_music".into()),
            params: None,
            response: "Playing music.".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let decoded: LlmResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.response, r.response);
    }

    #[test]
    fn node_state_serializes() {
        let s = serde_json::to_string(&NodeState::Processing).unwrap();
        assert_eq!(s, "\"Processing\"");
    }
}
