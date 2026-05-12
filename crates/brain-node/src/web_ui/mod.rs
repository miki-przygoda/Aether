pub mod api;
pub mod pages;
pub mod sse;
mod templates;

use crate::grpc::RagConfig;
use crate::llm::LlmClient;
use crate::session::SessionRegistry;
use crate::skills::{SkillConfig, SkillRegistry};
use crate::tts::TextToSpeech;
use aether_core::{CommandTrie, TtsSettings};
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::CorsLayer;

// ── Shared application state ───────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub env: Arc<minijinja::Environment<'static>>,
    pub registry: SessionRegistry,
    pub skills: Arc<SkillRegistry>,
    pub tts: Option<Arc<dyn TextToSpeech>>,
    pub tts_settings: Arc<RwLock<TtsSettings>>,
    pub llm: Option<Arc<dyn LlmClient>>,
    pub trie: Arc<CommandTrie>,
    pub rag: Option<RagConfig>,
    pub certs_dir: PathBuf,
    pub config_dir: PathBuf,
    pub documents_dir: Option<PathBuf>,
    pub ollama_url: String,
    pub wake_training: Arc<Mutex<WakeTrainingState>>,
    pub voice_training: Arc<Mutex<VoiceTrainingState>>,
    pub model_settings: Arc<RwLock<ModelSettings>>,
    pub finetuning_url: Option<String>,
    /// Shared HTTP client — one connection pool for all outbound requests (weather, HA, etc.).
    pub http_client: reqwest::Client,
    /// Skill configuration (location, HA, Navidrome, etc.) — persisted to skills.json.
    pub skill_config: Arc<RwLock<SkillConfig>>,
    /// LAN-accessible IP of the brain machine — used to construct Navidrome stream URLs.
    pub brain_ip: String,
    /// Channel for pushing wake-word training progress to SSE subscribers.
    pub wake_progress_tx: Arc<tokio::sync::broadcast::Sender<ProgressEvent>>,
    /// Channel for pushing voice training progress to SSE subscribers.
    pub voice_progress_tx: Arc<tokio::sync::broadcast::Sender<ProgressEvent>>,
    /// Channel for pushing document ingestion progress to SSE subscribers.
    pub ingest_progress_tx: Arc<tokio::sync::broadcast::Sender<ProgressEvent>>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: SessionRegistry,
        skills: Arc<SkillRegistry>,
        tts: Option<Arc<dyn TextToSpeech>>,
        tts_settings: Arc<RwLock<TtsSettings>>,
        llm: Option<Arc<dyn LlmClient>>,
        trie: Arc<CommandTrie>,
        rag: Option<RagConfig>,
        certs_dir: PathBuf,
        config_dir: PathBuf,
        documents_dir: Option<PathBuf>,
        ollama_url: String,
        finetuning_url: Option<String>,
        brain_ip: String,
    ) -> Self {
        let (wake_tx, _) = tokio::sync::broadcast::channel(64);
        let (voice_tx, _) = tokio::sync::broadcast::channel(64);
        let (ingest_tx, _) = tokio::sync::broadcast::channel(64);

        let model_settings = load_model_settings(&config_dir);

        let existing_wake_samples = load_wake_samples_from_disk(&config_dir);

        let skill_config = load_skill_config(&config_dir);

        Self {
            env: Arc::new(templates::build()),
            registry,
            skills,
            tts,
            tts_settings,
            llm,
            trie,
            rag,
            certs_dir,
            config_dir,
            documents_dir,
            ollama_url,
            wake_training: Arc::new(Mutex::new(WakeTrainingState {
                samples: existing_wake_samples,
                ..Default::default()
            })),
            voice_training: Arc::new(Mutex::new(VoiceTrainingState::default())),
            model_settings: Arc::new(RwLock::new(model_settings)),
            finetuning_url,
            http_client: reqwest::Client::new(),
            skill_config: Arc::new(RwLock::new(skill_config)),
            wake_progress_tx: Arc::new(wake_tx),
            voice_progress_tx: Arc::new(voice_tx),
            ingest_progress_tx: Arc::new(ingest_tx),
            brain_ip,
        }
    }
}

pub fn load_skill_config(config_dir: &std::path::Path) -> SkillConfig {
    let path = config_dir.join("skills.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_skill_config(config_dir: &std::path::Path, config: &SkillConfig) {
    let path = config_dir.join("skills.json");
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(path, json);
    }
}

fn load_model_settings(config_dir: &std::path::Path) -> ModelSettings {
    let path = config_dir.join("model_settings.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// ── Paired-node registry ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedNode {
    pub node_id: String,
    pub paired_at: String,
}

pub fn load_paired_nodes(config_dir: &std::path::Path) -> Vec<PairedNode> {
    let path = config_dir.join("paired_nodes.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_paired_nodes(config_dir: &std::path::Path, nodes: &[PairedNode]) {
    let path = config_dir.join("paired_nodes.json");
    if let Ok(json) = serde_json::to_string_pretty(nodes) {
        let _ = std::fs::write(path, json);
    }
}

pub fn register_paired_node(config_dir: &std::path::Path, node_id: &str) {
    let mut nodes = load_paired_nodes(config_dir);
    if !nodes.iter().any(|n| n.node_id == node_id) {
        nodes.push(PairedNode {
            node_id: node_id.to_string(),
            paired_at: chrono::Utc::now().to_rfc3339(),
        });
        save_paired_nodes(config_dir, &nodes);
    }
}

// ── Wake sample persistence ────────────────────────────────────────────────────

fn wake_samples_index(config_dir: &std::path::Path) -> std::path::PathBuf {
    config_dir.join("wake_samples").join("index.json")
}

pub fn load_wake_samples_from_disk(config_dir: &std::path::Path) -> Vec<WakeSample> {
    std::fs::read_to_string(wake_samples_index(config_dir))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_wake_samples_to_disk(config_dir: &std::path::Path, samples: &[WakeSample]) {
    let path = wake_samples_index(config_dir);
    // Ensure the directory exists before writing.
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(samples) {
        let _ = std::fs::write(path, json);
    }
}

pub fn remove_paired_node(config_dir: &std::path::Path, node_id: &str) {
    let mut nodes = load_paired_nodes(config_dir);
    nodes.retain(|n| n.node_id != node_id);
    save_paired_nodes(config_dir, &nodes);
}

// ── Setup wizard state ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WizardStage {
    BrainCheck,
    Pairing,
    WakeWord,
    GoLive,
    Complete,
}

impl WizardStage {
    pub fn next(&self) -> Option<WizardStage> {
        match self {
            Self::BrainCheck => Some(Self::Pairing),
            Self::Pairing    => Some(Self::WakeWord),
            Self::WakeWord   => Some(Self::GoLive),
            Self::GoLive     => Some(Self::Complete),
            Self::Complete   => None,
        }
    }

    pub fn index(&self) -> usize {
        match self {
            Self::BrainCheck => 0,
            Self::Pairing    => 1,
            Self::WakeWord   => 2,
            Self::GoLive     => 3,
            Self::Complete   => 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WizardState {
    pub stage: WizardStage,
    pub target_node_id: Option<String>,
    /// Path to the trained wake-word model on the brain (config dir).
    pub wake_model_path: Option<String>,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            stage: WizardStage::BrainCheck,
            target_node_id: None,
            wake_model_path: None,
        }
    }
}

pub fn load_wizard_state(config_dir: &std::path::Path) -> WizardState {
    let path = config_dir.join("setup_wizard.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_wizard_state(config_dir: &std::path::Path, state: &WizardState) {
    let path = config_dir.join("setup_wizard.json");
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, json);
    }
}

// ── Domain types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TrainingStatus {
    #[default]
    Idle,
    Running {
        progress: u8,
        message: String,
    },
    Complete {
        accuracy: f32,
        model_path: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WakeSample {
    pub id: String,
    pub filename: String,
    pub duration_ms: u32,
    pub size_bytes: u64,
}

#[derive(Debug, Default)]
pub struct WakeTrainingState {
    pub samples: Vec<WakeSample>,
    #[allow(dead_code)]
    pub status: TrainingStatus,
    pub model_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceUser {
    pub id: String,
    pub name: String,
    pub sample_count: usize,
    pub trained: bool,
    pub trained_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VoiceTrainingStatus {
    #[default]
    Idle,
    Running {
        user_id: String,
        progress: u8,
    },
    Complete {
        user_id: String,
    },
    Failed {
        user_id: String,
        error: String,
    },
}

#[derive(Debug, Default)]
pub struct VoiceTrainingState {
    pub users: Vec<VoiceUser>,
    pub active_samples: Vec<VoiceSample>,
    #[allow(dead_code)]
    pub status: VoiceTrainingStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSample {
    pub id: String,
    pub user_id: String,
    pub transcript: String,
    pub filename: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    pub llm_model: String,
    pub whisper_mode: WhisperMode,
    pub whisper_confidence: f32,
    pub llm_routing: LlmRouting,
    pub embed_model: String,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            llm_model: "llama3.2:3b".to_string(),
            whisper_mode: WhisperMode::Dynamic,
            whisper_confidence: 0.75,
            llm_routing: LlmRouting::Auto,
            embed_model: "nomic-embed-text".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhisperMode {
    Medium,
    Dynamic,
    Large,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LlmRouting {
    Fast,
    Auto,
}

/// Progress event emitted by training jobs and ingestion — subscribers render it as SSE data.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProgressEvent {
    pub percent: u8,
    pub message: String,
    pub done: bool,
    pub error: bool,
}

// ── Error type ────────────────────────────────────────────────────────────────

pub struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Internal error: {}", self.0),
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

pub type AppResult<T> = Result<T, AppError>;

// ── JSON error ────────────────────────────────────────────────────────────────

pub fn json_error(msg: impl Into<String>) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({ "error": msg.into() }))
}

// ── Router ────────────────────────────────────────────────────────────────────

async fn root_redirect() -> Redirect {
    Redirect::permanent("/ui/")
}

pub fn make_router(state: AppState) -> Router {
    Router::new()
        // Root redirect
        .route("/", get(root_redirect))
        // Static assets
        .route("/static/app.css", get(serve_css))
        .route("/static/app.js", get(serve_js))
        // Pages
        .route("/ui/setup", get(pages::setup::handler))
        .route("/ui/", get(pages::dashboard::handler))
        .route("/ui", get(pages::dashboard::handler))
        .route("/ui/nodes", get(pages::nodes::list_handler))
        .route("/ui/nodes/pair", get(pages::nodes::pair_handler))
        .route("/ui/documents", get(pages::documents::handler))
        .route("/ui/skills", get(pages::skills::handler))
        .route("/ui/settings/tts", get(pages::settings::tts_handler))
        .route("/ui/settings/models", get(pages::settings::models_handler))
        .route("/ui/settings/skills", get(pages::skills_settings::handler))
        .route(
            "/ui/training/wake-word",
            get(pages::training::wake_word_handler),
        )
        .route("/ui/training/voice", get(pages::training::voice_handler))
        // SSE streams
        .route("/events/nodes", get(sse::nodes_handler))
        .route(
            "/events/training/wake-word",
            get(sse::wake_training_handler),
        )
        .route("/events/training/voice", get(sse::voice_training_handler))
        .route(
            "/events/documents/ingest",
            get(sse::ingest_progress_handler),
        )
        // API — setup wizard
        .route("/api/setup/status", get(api::setup::get_status))
        .route("/api/setup/advance", axum::routing::post(api::setup::advance_stage))
        .route("/api/setup/node", axum::routing::post(api::setup::set_target_node))
        .route("/api/setup/wake-model", axum::routing::post(api::setup::set_wake_model))
        .route("/api/setup/reset", axum::routing::delete(api::setup::reset))
        // API — nodes
        .route("/api/nodes", get(api::nodes::list))
        .route(
            "/api/nodes/pair",
            axum::routing::post(api::nodes::confirm_pair),
        )
        .route(
            "/api/nodes/:id",
            axum::routing::delete(api::nodes::unpair).patch(api::nodes::rename),
        )
        // API — documents
        .route(
            "/api/documents",
            get(api::documents::list).post(api::documents::upload),
        )
        .route(
            "/api/documents/ingest",
            axum::routing::post(api::documents::trigger_ingest),
        )
        .route(
            "/api/history/:node_id",
            axum::routing::delete(api::documents::clear_history),
        )
        // API — skills
        .route("/api/skills", get(api::skills::list))
        .route("/api/skills/test", axum::routing::post(api::skills::test))
        .route(
            "/api/settings/skills",
            get(api::skills_settings::get).post(api::skills_settings::save),
        )
        .route(
            "/api/skills/location-search",
            get(api::skills_settings::location_search),
        )
        // API — settings
        .route(
            "/api/settings/tts",
            get(api::settings::get_tts).post(api::settings::save_tts),
        )
        .route(
            "/api/settings/tts/preview",
            axum::routing::post(api::settings::tts_preview),
        )
        .route(
            "/api/settings/models",
            get(api::settings::get_models).post(api::settings::save_models),
        )
        .route(
            "/api/settings/models/:name/pull",
            axum::routing::post(api::settings::pull_model),
        )
        .route(
            "/api/settings/models/:name",
            axum::routing::delete(api::settings::remove_model),
        )
        // API — wake word training
        .route(
            "/api/training/wake-word/samples",
            get(api::training::list_wake_samples).post(api::training::upload_wake_sample),
        )
        .route(
            "/api/training/wake-word/samples/:id",
            axum::routing::delete(api::training::delete_wake_sample),
        )
        .route(
            "/api/training/wake-word/train",
            axum::routing::post(api::training::train_wake_word),
        )
        .route(
            "/api/training/wake-word/deploy",
            axum::routing::post(api::training::deploy_wake_word),
        )
        // API — voice training
        .route(
            "/api/training/voice/users",
            get(api::training::list_voice_users).post(api::training::create_voice_user),
        )
        .route(
            "/api/training/voice/users/:id",
            axum::routing::delete(api::training::delete_voice_user),
        )
        .route(
            "/api/training/voice/samples",
            axum::routing::post(api::training::upload_voice_sample),
        )
        .route(
            "/api/training/voice/train",
            axum::routing::post(api::training::train_voice),
        )
        .route(
            "/api/training/voice/activate",
            axum::routing::post(api::training::activate_voice_model),
        )
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn serve_css() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css")],
        include_str!("static/app.css"),
    )
}

async fn serve_js() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        include_str!("static/app.js"),
    )
}

// ── Template rendering helper ──────────────────────────────────────────────────

pub fn render(state: &AppState, template: &str, ctx: minijinja::Value) -> AppResult<Html<String>> {
    let tmpl = state
        .env
        .get_template(template)
        .map_err(|e| anyhow::anyhow!("template {template}: {e}"))?;
    let html = tmpl
        .render(ctx)
        .map_err(|e| anyhow::anyhow!("render {template}: {e}"))?;
    Ok(Html(html))
}
