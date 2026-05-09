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

/// Published by the brain session registry on every `NodeState` transition.
/// Subscribers (auxiliary nodes, Phase 5 web UI) use this to mirror state.
#[derive(Debug, Clone)]
pub struct NodeStateEvent {
    pub node_id: String,
    pub state: NodeState,
}

/// TTS synthesis settings — loaded from env/config, overridable by the Phase 5 web UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSettings {
    /// Playback speed multiplier (1.0 = normal, 0.8 = slower, 1.2 = faster).
    pub speed: f32,
    /// Voice identifier; currently only "default" is supported (maps to voice_style.bin).
    pub voice: String,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            speed: 1.0,
            voice: "default".to_string(),
        }
    }
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
