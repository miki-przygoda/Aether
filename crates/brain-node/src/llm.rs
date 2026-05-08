use aether_core::LlmResponse;
use anyhow::{Context, Result};

/// Abstraction over LLM backends — allows tests to inject a mock without a running Ollama.
pub trait LlmClient: Send + Sync {
    fn ask(&self, transcript: &str) -> Result<LlmResponse>;
}

// ─── Ollama client ────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "You are Aether, a local-first voice assistant. \
Reply ONLY with a JSON object — no other text. Schema:\n\
{\"action\": \"<action_name>\", \"params\": {}, \"response\": \"<spoken reply>\"}\n\
Valid actions: play_music, pause_music, stop_music, set_timer, lights_on, lights_off, \
weather, volume_up, volume_down, respond.\n\
Use action \"respond\" for general conversation or when no specific action applies. \
Keep responses concise.";

pub struct OllamaClient {
    client: reqwest::blocking::Client,
    base_url: String,
    model: String,
}

impl OllamaClient {
    pub fn new(base_url: String, model: String) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .context("building reqwest blocking client")?;
        Ok(Self {
            client,
            base_url,
            model,
        })
    }
}

impl LlmClient for OllamaClient {
    fn ask(&self, transcript: &str) -> Result<LlmResponse> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user",   "content": transcript}
            ],
            "format": "json",
            "stream": false
        });

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .context("sending request to Ollama")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            anyhow::bail!("Ollama returned {status}: {body}");
        }

        let val: serde_json::Value = resp.json().context("parsing Ollama JSON response")?;
        let content = val["message"]["content"]
            .as_str()
            .with_context(|| format!("missing message.content in Ollama response: {val}"))?;

        serde_json::from_str::<LlmResponse>(content)
            .with_context(|| format!("parsing LlmResponse from Ollama content: {content:?}"))
    }
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_llm_response_json_is_parsed() {
        let json = r#"{"action":"play_music","params":null,"response":"Playing music."}"#;
        let r: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.action.as_deref(), Some("play_music"));
        assert_eq!(r.response, "Playing music.");
    }

    #[test]
    fn missing_action_deserialises_to_none() {
        let json = r#"{"params":null,"response":"Hello!"}"#;
        let r: LlmResponse = serde_json::from_str(json).unwrap();
        assert!(r.action.is_none());
        assert_eq!(r.response, "Hello!");
    }

    #[test]
    fn malformed_json_returns_error() {
        let result: Result<LlmResponse, _> = serde_json::from_str("not json at all");
        assert!(result.is_err());
    }

    #[test]
    fn respond_action_with_params_object() {
        let json =
            r#"{"action":"set_timer","params":{"duration_seconds":300},"response":"Timer set."}"#;
        let r: LlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.action.as_deref(), Some("set_timer"));
        assert!(r.params.is_some());
    }

    #[test]
    #[ignore = "requires a running Ollama instance — set OLLAMA_BASE_URL or use default"]
    fn live_ollama_returns_valid_llm_response() {
        let url = std::env::var("OLLAMA_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        let model = std::env::var("LLM_FAST_MODEL").unwrap_or_else(|_| "llama3.2:3b".to_string());
        let client = OllamaClient::new(url, model).unwrap();
        let resp = client.ask("what is the weather like today").unwrap();
        assert!(!resp.response.is_empty(), "response should not be empty");
    }
}
