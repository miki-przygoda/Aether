mod grpc;
mod history;
mod ingest;
#[cfg(test)]
mod integration_tests;
mod llm;
mod mdns_adv;
mod pair;
mod session;
mod skills;
mod stt;
mod tts;
mod vector_store;
mod web_ui;

use aether_core::TtsSettings;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use grpc::{proto::aether_brain_server::AetherBrainServer, BrainService};
use session::SessionRegistry;
use skills::SkillRegistry;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

#[derive(Parser)]
#[command(name = "brain-node", about = "Aether brain node")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Run the mTLS gRPC server and advertise via mDNS.
    Serve {
        #[arg(long, env = "BRAIN_GRPC_PORT", default_value = "50051")]
        port: u16,

        /// Port for the web configuration UI (HTTP, unencrypted — local network only).
        #[arg(long, env = "BRAIN_WEB_PORT", default_value = "8080")]
        web_port: u16,

        #[arg(long, env = "BRAIN_CERTS_DIR", default_value = "/data/certs")]
        certs_dir: PathBuf,

        /// Path to the Whisper GGUF model file (e.g. ggml-medium.bin).
        /// STT is disabled and transcripts will not be produced if not set.
        #[arg(long, env = "WHISPER_MODEL_PATH")]
        whisper_model: Option<PathBuf>,

        /// Path to a larger Whisper model used when confidence falls below threshold.
        #[arg(long, env = "WHISPER_FALLBACK_MODEL_PATH")]
        whisper_fallback: Option<PathBuf>,

        /// Confidence threshold [0.0–1.0] below which the fallback model is used.
        #[arg(long, env = "WHISPER_CONFIDENCE_THRESHOLD", default_value = "0.75")]
        whisper_confidence: f32,

        /// Base URL of the Ollama instance for LLM inference.
        #[arg(
            long,
            env = "OLLAMA_BASE_URL",
            default_value = "http://localhost:11434"
        )]
        ollama_url: String,

        /// Ollama model name used for the LLM fast tier.
        #[arg(long, env = "LLM_FAST_MODEL", default_value = "llama3.2:3b")]
        llm_model: String,

        /// Path to the Kokoro-82M ONNX model file.
        /// TTS is disabled if not set; expects vocab.json and voice_style.bin alongside.
        #[arg(long, env = "KOKORO_MODEL_PATH")]
        kokoro_model: Option<PathBuf>,

        /// TTS playback speed (1.0 = normal, 0.8 = slower, 1.2 = faster).
        #[arg(long, env = "TTS_SPEED", default_value = "1.0")]
        tts_speed: f32,

        /// TTS voice identifier (currently only "default" is supported).
        #[arg(long, env = "TTS_VOICE", default_value = "default")]
        tts_voice: String,

        /// Qdrant gRPC URL. RAG and conversation history are disabled if not set.
        #[arg(long, env = "QDRANT_URL")]
        qdrant_url: Option<String>,

        /// Ollama embedding model (used for RAG query embedding and document ingestion).
        #[arg(long, env = "EMBED_MODEL", default_value = "nomic-embed-text")]
        embed_model: String,

        /// Number of conversation turns to inject as history context.
        #[arg(long, env = "HISTORY_TURNS", default_value = "10")]
        history_turns: usize,

        /// Directory of documents to ingest into Qdrant on startup.
        /// Accepts .txt and .md files. Skipped if QDRANT_URL is not set.
        #[arg(long, env = "DOCUMENTS_DIR")]
        documents_dir: Option<PathBuf>,

        /// Writable directory for web UI state persistence (TTS settings, wake word samples, etc.).
        #[arg(long, env = "BRAIN_CONFIG_DIR", default_value = "/data/config")]
        config_dir: PathBuf,

        /// Base URL of the voice fine-tuning Python service.
        /// Voice personalisation is disabled when not set.
        #[arg(long, env = "FINETUNING_URL")]
        finetuning_url: Option<String>,
    },

    /// Pairing ceremony — plain (non-TLS) gRPC on a separate port.
    /// Plug the Pi in with a direct cable before running this.
    Pair {
        #[arg(long, env = "BRAIN_PAIR_PORT", default_value = "50052")]
        port: u16,

        #[arg(long, env = "BRAIN_CERTS_DIR", default_value = "/data/certs")]
        certs_dir: PathBuf,
    },

    /// Generate synthetic wake-word WAV samples via Kokoro TTS.
    ///
    /// Outputs `<phrase>_sp<speed>_<n>.wav` files into `--output-dir`.
    /// Run this after `scripts/download-models.sh` has populated `./models/tts/`.
    GenerateWakeWordSamples {
        /// Path to the Kokoro-82M ONNX model file (same as --kokoro-model for serve).
        #[arg(long, env = "KOKORO_MODEL_PATH")]
        kokoro_model: PathBuf,

        /// Directory to write WAV samples into (created if absent).
        #[arg(long, default_value = "models/wake-word/samples/synthetic")]
        output_dir: PathBuf,

        /// Wake word phrase to synthesise.
        #[arg(long, default_value = "Hey Aether")]
        phrase: String,

        /// Number of samples to generate per speed level.
        #[arg(long, default_value = "3")]
        count: usize,
    },
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

async fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Serve {
            port,
            web_port,
            certs_dir,
            whisper_model,
            whisper_fallback,
            whisper_confidence,
            ollama_url,
            llm_model,
            kokoro_model,
            tts_speed,
            tts_voice,
            qdrant_url,
            embed_model,
            history_turns,
            documents_dir,
            config_dir,
            finetuning_url,
        } => {
            serve(ServeArgs {
                port,
                web_port,
                certs_dir,
                whisper_model,
                whisper_fallback,
                whisper_confidence,
                ollama_url,
                llm_model,
                kokoro_model,
                tts_settings: TtsSettings {
                    speed: tts_speed,
                    voice: tts_voice,
                },
                qdrant_url,
                embed_model,
                history_turns,
                documents_dir,
                config_dir,
                finetuning_url,
            })
            .await
        }
        Command::Pair { port, certs_dir } => run_pair_server(port, certs_dir).await,
        Command::GenerateWakeWordSamples {
            kokoro_model,
            output_dir,
            phrase,
            count,
        } => generate_wake_word_samples(kokoro_model, output_dir, phrase, count),
    }
}

struct ServeArgs {
    port: u16,
    web_port: u16,
    certs_dir: PathBuf,
    whisper_model: Option<PathBuf>,
    whisper_fallback: Option<PathBuf>,
    whisper_confidence: f32,
    ollama_url: String,
    llm_model: String,
    kokoro_model: Option<PathBuf>,
    tts_settings: TtsSettings,
    qdrant_url: Option<String>,
    embed_model: String,
    history_turns: usize,
    documents_dir: Option<PathBuf>,
    config_dir: PathBuf,
    finetuning_url: Option<String>,
}

async fn serve(args: ServeArgs) -> Result<()> {
    let ServeArgs {
        port,
        web_port,
        certs_dir,
        whisper_model,
        whisper_fallback,
        whisper_confidence,
        ollama_url,
        llm_model,
        kokoro_model,
        tts_settings,
        qdrant_url,
        embed_model,
        history_turns,
        documents_dir,
        config_dir,
        finetuning_url,
    } = args;

    std::fs::create_dir_all(&config_dir).context("creating config dir")?;
    let local_ip = local_ip_address::local_ip().context("detecting local IP")?;
    tracing::info!(ip = %local_ip, "brain local address");

    pair::ensure_certs(&certs_dir, local_ip).context("ensuring certs")?;

    let ca_pem = std::fs::read(certs_dir.join("ca.pem"))?;
    let server_cert_pem = std::fs::read(certs_dir.join("brain.pem"))?;
    let server_key_pem = std::fs::read(certs_dir.join("brain-key.pem"))?;

    let identity = Identity::from_pem(&server_cert_pem, &server_key_pem);
    let client_ca = Certificate::from_pem(&ca_pem);
    let tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca);

    let stt_engine: Option<Arc<dyn stt::SpeechToText>> = if let Some(model_path) = whisper_model {
        let model_str = model_path
            .to_str()
            .context("WHISPER_MODEL_PATH is not valid UTF-8")?;
        let fallback_str = whisper_fallback
            .as_deref()
            .map(|p| {
                p.to_str()
                    .context("WHISPER_FALLBACK_MODEL_PATH is not valid UTF-8")
            })
            .transpose()?;
        tracing::info!(model = %model_str, "loading Whisper STT model");
        let w = stt::WhisperStt::new(model_str, fallback_str, whisper_confidence)
            .context("loading Whisper STT model")?;
        Some(Arc::new(w))
    } else {
        tracing::warn!("--whisper-model not set — STT disabled (no transcripts will be produced)");
        None
    };

    tracing::info!(url = %ollama_url, model = %llm_model, "configuring Ollama LLM client");
    let embed_url = ollama_url.clone();
    let ollama_url_for_ui = ollama_url.clone();
    let llm_engine: Option<Arc<dyn llm::LlmClient>> = Some(Arc::new(
        llm::OllamaClient::new(ollama_url, llm_model).context("creating Ollama client")?,
    ));

    let tts_engine: Option<Arc<dyn tts::TextToSpeech>> = if let Some(model_path) = kokoro_model {
        let model_str = model_path
            .to_str()
            .context("KOKORO_MODEL_PATH is not valid UTF-8")?
            .to_owned();
        tracing::info!(model = %model_str, "loading Kokoro TTS model");
        // ORT environment creation can deadlock on some aarch64/Docker Desktop setups.
        // Apply a 60s timeout: if it hangs, disable TTS and continue so the web UI
        // still starts. The idle ORT threads consume no CPU while sleeping.
        let load_result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            tokio::task::spawn_blocking(move || tts::KokoroTts::new(&model_str)),
        )
        .await;
        match load_result {
            Ok(Ok(Ok(k))) => Some(Arc::new(k)),
            Ok(Ok(Err(e))) => {
                tracing::error!("TTS model load failed: {e:#} — TTS disabled");
                None
            }
            Ok(Err(e)) => {
                tracing::error!("TTS init thread panicked: {e} — TTS disabled");
                None
            }
            Err(_elapsed) => {
                tracing::warn!("TTS model load timed out after 60 s — TTS disabled (ORT init deadlock?)");
                None
            }
        }
    } else {
        tracing::warn!("--kokoro-model not set — TTS disabled");
        None
    };

    // ── Qdrant / RAG ──────────────────────────────────────────────────────────
    // QdrantStore uses reqwest::blocking — run all Qdrant I/O on a blocking
    // thread to avoid occupying Tokio worker threads.
    let rag_config: Option<grpc::RagConfig> = if let Some(ref url) = qdrant_url {
        use vector_store::{QdrantStore, VectorStore, COLLECTION_DOCUMENTS, COLLECTION_HISTORY};
        tracing::info!(%url, "connecting to Qdrant");

        let qdrant_url_str = url.clone();
        let embed_url_clone = embed_url.clone();
        let embed_model_clone = embed_model.clone();
        let documents_dir_clone = documents_dir.clone();

        let store: Arc<dyn VectorStore> = tokio::task::spawn_blocking(move || {
            // nomic-embed-text produces 768-dimensional vectors.
            const EMBED_DIM: usize = 768;
            let store = Arc::new(
                QdrantStore::new(&qdrant_url_str).context("creating Qdrant client")?,
            ) as Arc<dyn VectorStore>;
            store
                .ensure_collection(COLLECTION_DOCUMENTS, EMBED_DIM)
                .context("creating Qdrant documents collection")?;
            store
                .ensure_collection(COLLECTION_HISTORY, 1)
                .context("creating Qdrant history collection")?;

            if let Some(ref dir) = documents_dir_clone {
                match ingest::ingest_dir(dir, &store, &embed_url_clone, &embed_model_clone) {
                    Ok(n) => tracing::info!(chunks = n, "document ingestion complete"),
                    Err(e) => tracing::warn!("document ingestion failed: {e}"),
                }
            }
            Ok::<_, anyhow::Error>(store)
        })
        .await
        .context("Qdrant init thread panicked")?
        .context("Qdrant setup failed")?;

        Some(grpc::RagConfig {
            store,
            qdrant_url: url.clone(),
            embed_url,
            embed_model,
            history_turns,
            score_threshold: 0.5,
        })
    } else {
        tracing::warn!("QDRANT_URL not set — RAG and conversation history disabled");
        None
    };

    let tts_settings = Arc::new(RwLock::new(tts_settings));
    let registry = SessionRegistry::new();
    let trie = Arc::new(aether_core::CommandTrie::default());
    let skills = Arc::new(SkillRegistry::default());

    let web_state = web_ui::AppState::new(
        registry.clone(),
        skills.clone(),
        tts_engine.clone(),
        tts_settings.clone(),
        llm_engine.clone(),
        trie.clone(),
        rag_config.clone(),
        certs_dir.clone(),
        config_dir,
        documents_dir,
        ollama_url_for_ui,
        finetuning_url,
    );

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let service = BrainService {
        registry,
        certs_dir,
        stt: stt_engine,
        trie,
        llm: llm_engine,
        tts: tts_engine,
        tts_settings,
        skills,
        rag: rag_config,
    };

    let _mdns = match local_ip {
        IpAddr::V4(v4) => mdns_adv::advertise(port, v4).ok(),
        IpAddr::V6(_) => {
            tracing::warn!("IPv6 local address — skipping mDNS advertisement");
            None
        }
    };

    // ── Web UI HTTP server ────────────────────────────────────────────────────
    let web_addr: SocketAddr = ([0, 0, 0, 0], web_port).into();
    let web_router = web_ui::make_router(web_state);
    tokio::spawn(async move {
        tracing::info!(%web_addr, "web UI server starting (HTTP)");
        let listener = tokio::net::TcpListener::bind(web_addr)
            .await
            .expect("binding web UI port");
        axum::serve(listener, web_router)
            .await
            .expect("web UI server error");
    });

    tracing::info!(%addr, "brain gRPC server starting (mTLS)");
    Server::builder()
        .tls_config(tls)?
        .add_service(AetherBrainServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

fn generate_wake_word_samples(
    kokoro_model: PathBuf,
    output_dir: PathBuf,
    phrase: String,
    count: usize,
) -> Result<()> {
    let model_str = kokoro_model
        .to_str()
        .context("KOKORO_MODEL_PATH is not valid UTF-8")?;

    tracing::info!(model = %model_str, phrase = %phrase, "loading Kokoro TTS model");
    let tts = tts::KokoroTts::new(model_str).context("loading Kokoro TTS model")?;

    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating output directory {}", output_dir.display()))?;

    // Speeds that give useful pitch/pace variation for wake-word training.
    let speeds: &[f32] = &[0.8, 0.9, 1.0, 1.1, 1.2];
    let stem = phrase.to_lowercase().replace(' ', "_");
    let mut total = 0usize;

    for &speed in speeds {
        for n in 0..count {
            let wav = tts
                .synthesise_at_speed(&phrase, speed)
                .with_context(|| format!("synthesising at speed {speed}"))?;

            let filename = format!("{stem}_sp{speed:.1}_{n:02}.wav");
            let path = output_dir.join(&filename);
            std::fs::write(&path, &wav).with_context(|| format!("writing {}", path.display()))?;

            tracing::info!(file = %filename, speed, "wrote sample");
            total += 1;
        }
    }

    println!("Generated {total} samples in {}", output_dir.display());
    println!("Next: run scripts/train-wake-word.sh to train the rustpotter model.");
    Ok(())
}

async fn run_pair_server(port: u16, certs_dir: PathBuf) -> Result<()> {
    let local_ip = local_ip_address::local_ip().context("detecting local IP")?;
    pair::ensure_certs(&certs_dir, local_ip).context("ensuring certs")?;

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let service = BrainService {
        registry: SessionRegistry::new(),
        certs_dir,
        stt: None,
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: None,
        tts: None,
        tts_settings: Arc::new(RwLock::new(TtsSettings::default())),
        skills: Arc::new(SkillRegistry::default()),
        rag: None,
    };

    tracing::info!(%addr, "pairing server listening (plain gRPC)");
    println!("Pairing mode active on port {port}.  Connect the Pi with a cable, then run:");
    println!("  edge-node pair --brain-addr <this-machine-ip>:{port} --node-id <name>");

    Server::builder()
        .add_service(AetherBrainServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
