mod grpc;
#[cfg(test)]
mod integration_tests;
mod llm;
mod mdns_adv;
mod pair;
mod session;
mod skills;
mod stt;
mod tts;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use grpc::{proto::aether_brain_server::AetherBrainServer, BrainService};
use session::SessionRegistry;
use skills::SkillRegistry;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

#[derive(Parser)]
#[command(name = "brain-node", about = "Aether brain node")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the mTLS gRPC server and advertise via mDNS.
    Serve {
        #[arg(long, env = "BRAIN_GRPC_PORT", default_value = "50051")]
        port: u16,

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

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    match Cli::parse().command {
        Command::Serve {
            port,
            certs_dir,
            whisper_model,
            whisper_fallback,
            whisper_confidence,
            ollama_url,
            llm_model,
            kokoro_model,
        } => {
            serve(ServeArgs {
                port,
                certs_dir,
                whisper_model,
                whisper_fallback,
                whisper_confidence,
                ollama_url,
                llm_model,
                kokoro_model,
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
    certs_dir: PathBuf,
    whisper_model: Option<PathBuf>,
    whisper_fallback: Option<PathBuf>,
    whisper_confidence: f32,
    ollama_url: String,
    llm_model: String,
    kokoro_model: Option<PathBuf>,
}

async fn serve(args: ServeArgs) -> Result<()> {
    let ServeArgs {
        port,
        certs_dir,
        whisper_model,
        whisper_fallback,
        whisper_confidence,
        ollama_url,
        llm_model,
        kokoro_model,
    } = args;
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
    let llm_engine: Option<Arc<dyn llm::LlmClient>> = Some(Arc::new(
        llm::OllamaClient::new(ollama_url, llm_model).context("creating Ollama client")?,
    ));

    let tts_engine: Option<Arc<dyn tts::TextToSpeech>> = if let Some(model_path) = kokoro_model {
        let model_str = model_path
            .to_str()
            .context("KOKORO_MODEL_PATH is not valid UTF-8")?;
        tracing::info!(model = %model_str, "loading Kokoro TTS model");
        let k = tts::KokoroTts::new(model_str).context("loading Kokoro TTS model")?;
        Some(Arc::new(k))
    } else {
        tracing::warn!("--kokoro-model not set — TTS disabled");
        None
    };

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let service = BrainService {
        registry: SessionRegistry::new(),
        certs_dir,
        stt: stt_engine,
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: llm_engine,
        tts: tts_engine,
        skills: Arc::new(SkillRegistry::default()),
    };

    let _mdns = match local_ip {
        IpAddr::V4(v4) => mdns_adv::advertise(port, v4).ok(),
        IpAddr::V6(_) => {
            tracing::warn!("IPv6 local address — skipping mDNS advertisement");
            None
        }
    };

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
            std::fs::write(&path, &wav)
                .with_context(|| format!("writing {}", path.display()))?;

            tracing::info!(file = %filename, speed, "wrote sample");
            total += 1;
        }
    }

    println!(
        "Generated {total} samples in {}",
        output_dir.display()
    );
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
        skills: Arc::new(SkillRegistry::default()),
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
