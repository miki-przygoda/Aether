use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OllamaUpdateInfo {
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
}

/// Fetches Ollama's current version and the latest GitHub release, then returns
/// a comparison result. Never fails — errors are captured in the `error` field.
pub async fn fetch_update_info(ollama_url: &str, http: &reqwest::Client) -> OllamaUpdateInfo {
    let current = fetch_current_version(ollama_url, http).await;
    let latest = fetch_latest_github_release(http).await;

    let now = Some(chrono::Utc::now());

    match (current, latest) {
        (Ok(current), Ok(latest)) => {
            let update_available = current != latest.trim_start_matches('v');
            OllamaUpdateInfo {
                current_version: Some(current),
                latest_version: Some(latest),
                update_available,
                last_checked_at: now,
                error: None,
            }
        }
        (Err(e), _) => OllamaUpdateInfo {
            last_checked_at: now,
            error: Some(format!("could not reach Ollama: {e:#}")),
            ..Default::default()
        },
        (_, Err(e)) => OllamaUpdateInfo {
            last_checked_at: now,
            error: Some(format!("could not reach GitHub: {e:#}")),
            ..Default::default()
        },
    }
}

async fn fetch_current_version(ollama_url: &str, http: &reqwest::Client) -> anyhow::Result<String> {
    let body: serde_json::Value = http
        .get(format!("{ollama_url}/api/version"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?
        .json()
        .await?;
    body["version"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing `version` field in Ollama response"))
}

async fn fetch_latest_github_release(http: &reqwest::Client) -> anyhow::Result<String> {
    let body: serde_json::Value = http
        .get("https://api.github.com/repos/ollama/ollama/releases/latest")
        .header("User-Agent", "aether/1.0")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await?
        .json()
        .await?;
    body["tag_name"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing `tag_name` in GitHub releases response"))
}
