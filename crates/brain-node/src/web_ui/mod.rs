pub mod api;
pub mod pages;
pub mod sse;
mod templates;

use crate::grpc::RagConfig;
use crate::llm::LlmClient;
use crate::session::SessionRegistry;
use crate::skills::SkillRegistry;
use crate::tts::TextToSpeech;
use aether_core::{CommandTrie, TtsSettings};
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
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
    ) -> Self {
        let (wake_tx, _) = tokio::sync::broadcast::channel(64);
        let (voice_tx, _) = tokio::sync::broadcast::channel(64);
        let (ingest_tx, _) = tokio::sync::broadcast::channel(64);

        let model_settings = load_model_settings(&config_dir);

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
            wake_training: Arc::new(Mutex::new(WakeTrainingState::default())),
            voice_training: Arc::new(Mutex::new(VoiceTrainingState::default())),
            model_settings: Arc::new(RwLock::new(model_settings)),
            finetuning_url,
            wake_progress_tx: Arc::new(wake_tx),
            voice_progress_tx: Arc::new(voice_tx),
            ingest_progress_tx: Arc::new(ingest_tx),
        }
    }
}

fn load_model_settings(config_dir: &std::path::Path) -> ModelSettings {
    let path = config_dir.join("model_settings.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub percent: u8,
    pub message: String,
    pub done: bool,
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

pub fn make_router(state: AppState) -> Router {
    Router::new()
        // Static assets
        .route("/static/app.css", get(serve_css))
        .route("/static/app.js", get(serve_js))
        // Pages
        .route("/ui/", get(pages::dashboard::handler))
        .route("/ui", get(pages::dashboard::handler))
        .route("/ui/nodes", get(pages::nodes::list_handler))
        .route("/ui/nodes/pair", get(pages::nodes::pair_handler))
        .route("/ui/documents", get(pages::documents::handler))
        .route("/ui/skills", get(pages::skills::handler))
        .route("/ui/settings/tts", get(pages::settings::tts_handler))
        .route("/ui/settings/models", get(pages::settings::models_handler))
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
