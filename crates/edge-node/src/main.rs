mod audio;
mod brain_conn;
mod device_discovery;
mod wake_word;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(name = "edge-node", about = "Aether edge node")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Listen for wake word and stream audio to the brain.
    Run {
        /// Path to the rustpotter .rpw wake word model.
        /// Fails fast with a helpful message if the file is missing.
        #[arg(long, env = "AETHER_MODEL_PATH")]
        model_path: PathBuf,

        /// Directory where pairing certs and config are stored.
        #[arg(long, env = "AETHER_CONFIG_DIR")]
        config_dir: Option<PathBuf>,
    },

    /// Pair this node with the brain over a wired connection.
    Pair {
        /// Brain's IP and pairing port, e.g. "192.168.1.100:50052".
        #[arg(long)]
        brain_addr: String,

        /// Stable identifier for this node, e.g. "office-pi".
        #[arg(long, env = "AETHER_NODE_ID")]
        node_id: String,

        #[arg(long, env = "AETHER_CONFIG_DIR")]
        config_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    match Cli::parse().command {
        Command::Run {
            model_path,
            config_dir,
        } => run(model_path, resolve_config_dir(config_dir)?).await,
        Command::Pair {
            brain_addr,
            node_id,
            config_dir,
        } => brain_conn::pair(&brain_addr, &node_id, &resolve_config_dir(config_dir)?).await,
    }
}

fn resolve_config_dir(override_: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = override_ {
        return Ok(dir);
    }
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".config").join("aether"))
}

async fn run(model_path: PathBuf, config_dir: PathBuf) -> Result<()> {
    let cfg = brain_conn::PairedConfig::load(&config_dir)
        .context("not paired yet — run `edge-node pair` first")?;

    tracing::info!(node_id = %cfg.node_id, "edge-node starting");

    let mut detector = wake_word::build(&model_path)?;

    // Resolve brain address (stored address, mDNS fallback).
    let brain_addr = brain_conn::resolve_brain(&config_dir).await?;
    let channel = brain_conn::build_mtls_channel(&brain_addr, &config_dir)?;

    // Audio capture channel: cpal callback → wake word loop.
    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<f32>>(64);
    let _stream = audio::start_capture(audio_tx)?;

    tracing::info!("listening for wake word…");

    loop {
        // Accumulate samples until wake word triggers.
        while let Some(samples) = audio_rx.recv().await {
            if detector.process_samples(&samples) {
                tracing::info!("wake word detected — opening stream to brain");
                break;
            }
        }

        // Stream audio until the brain closes the stream or an error occurs.
        let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<f32>>(64);

        let node_id = cfg.node_id.clone();
        let ch = channel.clone();
        // `mut` so we can take `&mut stream_task` in select! across iterations —
        // select! drops the non-winning branch's *reference*, not the task itself.
        let mut stream_task =
            tokio::spawn(async move { brain_conn::stream_audio(ch, &node_id, pcm_rx).await });

        // Forward audio from the capture ring into the streaming channel.
        'stream: loop {
            tokio::select! {
                Some(samples) = audio_rx.recv() => {
                    if pcm_tx.send(samples).await.is_err() {
                        break 'stream; // stream closed
                    }
                }
                result = &mut stream_task => {
                    if let Ok(Err(e)) = result {
                        tracing::warn!("brain stream error: {e}");
                    }
                    break 'stream;
                }
            }
        }

        tracing::info!("stream ended — returning to idle");
    }
}
