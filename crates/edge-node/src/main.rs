mod audio;
mod brain_conn;
mod device_discovery;
mod gpio;
#[cfg(test)]
mod integration_tests;
mod kill_signal;
mod playback;
mod state_server;
mod wake_word;

use aether_core::NodeState;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};

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
        #[arg(long, env = "AETHER_MODEL_PATH")]
        model_path: PathBuf,

        /// Directory where pairing certs and config are stored.
        #[arg(long, env = "AETHER_CONFIG_DIR")]
        config_dir: Option<PathBuf>,

        /// Port for the state SSE server (auxiliary nodes connect here).
        #[arg(long, env = "AETHER_STATE_PORT", default_value = "3000")]
        state_port: u16,
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

    /// Run in auxiliary mode: mirror a primary node's state on this node's LED.
    ///
    /// Connects to the primary node's SSE endpoint and drives the local GPIO
    /// LED to match. No wake word or audio capture runs in this mode.
    Auxiliary {
        /// Base URL of the primary edge node, e.g. "http://192.168.1.50:3000".
        #[arg(long, env = "AETHER_PRIMARY_URL")]
        target: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    match Cli::parse().command {
        Command::Run {
            model_path,
            config_dir,
            state_port,
        } => run(model_path, resolve_config_dir(config_dir)?, state_port).await,
        Command::Pair {
            brain_addr,
            node_id,
            config_dir,
        } => brain_conn::pair(&brain_addr, &node_id, &resolve_config_dir(config_dir)?).await,
        Command::Auxiliary { target } => run_auxiliary(&target).await,
    }
}

fn resolve_config_dir(override_: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = override_ {
        return Ok(dir);
    }
    let home = dirs::home_dir().context("cannot determine home directory")?;
    Ok(home.join(".config").join("aether"))
}

async fn run(model_path: PathBuf, config_dir: PathBuf, state_port: u16) -> Result<()> {
    let cfg = brain_conn::PairedConfig::load(&config_dir)
        .context("not paired yet — run `edge-node pair` first")?;

    tracing::info!(node_id = %cfg.node_id, "edge-node starting");

    // ── Peripheral discovery ─────────────────────────────────────────────────
    let disc_config = device_discovery::DeviceConfig::default();
    let devices = device_discovery::discover(&disc_config);
    for w in &devices.warnings {
        tracing::warn!("{w}");
    }
    if let Some(hat) = &devices.hat {
        tracing::info!(vendor = %hat.vendor, product = %hat.product, "HAT detected");
    }
    for (bus, chip) in &devices.i2c_chips {
        tracing::info!(bus, addr = chip.addr, name = chip.name, "I2C chip found");
    }

    // ── Kill signal broadcast ────────────────────────────────────────────────
    let (kill_tx, _kill_rx0) = kill_signal::channel();

    // ── GPIO (Raspberry Pi only) ─────────────────────────────────────────────
    #[cfg(feature = "gpio")]
    {
        gpio::register_panic_button(kill_tx.clone()).context("registering panic button")?;
    }

    // ── State broadcast (for SSE server) ────────────────────────────────────
    let (state_tx, _state_rx0): (broadcast::Sender<NodeState>, _) = broadcast::channel(16);

    // ── SIGUSR1 → re-scan peripherals ────────────────────────────────────────
    #[cfg(unix)]
    {
        let disc_config2 = disc_config.clone();
        tokio::spawn(async move {
            let mut usr1 =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined1())
                    .expect("registering SIGUSR1");
            loop {
                usr1.recv().await;
                tracing::info!("SIGUSR1 received — re-scanning peripherals");
                let report = device_discovery::discover(&disc_config2);
                for w in &report.warnings {
                    tracing::warn!("{w}");
                }
                tracing::info!(
                    alsa_cards = report.alsa_cards.len(),
                    i2c_chips = report.i2c_chips.len(),
                    "re-scan complete"
                );
            }
        });
    }

    // ── LED state driver (Raspberry Pi only) ─────────────────────────────────
    #[cfg(feature = "gpio")]
    {
        if let Ok(Some(mut led)) = gpio::LedController::from_env() {
            let mut state_rx = state_tx.subscribe();
            tokio::spawn(async move {
                while let Ok(state) = state_rx.recv().await {
                    let pattern = gpio::state_to_pattern(state);
                    led.apply(pattern);
                }
            });
        }
    }

    // ── State SSE server ─────────────────────────────────────────────────────
    {
        let tx = state_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = state_server::serve(state_port, tx).await {
                tracing::error!("state SSE server error: {e}");
            }
        });
    }

    // ── Wake word + audio streaming loop ─────────────────────────────────────
    let mut detector = wake_word::build(&model_path)?;

    let brain_addr = brain_conn::resolve_brain(&config_dir).await?;
    let channel = brain_conn::build_mtls_channel(&brain_addr, &config_dir)?;

    let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<f32>>(64);
    let _stream = audio::start_capture(audio_tx)?;

    // Channel used by stream_audio to signal that a new wake word model was
    // received and written to disk — main loop rebuilds the detector on next idle.
    let (model_reload_tx, mut model_reload_rx) = mpsc::channel::<()>(4);

    // Shared slot for the current music playback task — allows pause/stop
    // commands to abort mid-track playback from any subsequent wake-word session.
    let music_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>> = Arc::new(Mutex::new(None));

    publish_state(&state_tx, NodeState::Idle);
    tracing::info!("listening for wake word…");

    loop {
        // Accumulate samples until wake word triggers (or kill signal).
        let mut kill_rx = kill_tx.subscribe();
        'listen: loop {
            tokio::select! {
                Some(samples) = audio_rx.recv() => {
                    if detector.process_samples(&samples) {
                        tracing::info!("wake word detected — opening stream to brain");
                        break 'listen;
                    }
                }
                Ok(_) = kill_rx.recv() => {
                    tracing::info!("kill signal during idle listen — ignoring (already idle)");
                }
            }
        }

        publish_state(&state_tx, NodeState::Listening);

        // Stream audio until the brain closes the stream or kill is signalled.
        let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<f32>>(64);

        let node_id = cfg.node_id.clone();
        let ch = channel.clone();
        let reload_tx = model_reload_tx.clone();
        let music_handle_clone = music_handle.clone();
        let mut stream_task = tokio::spawn(async move {
            brain_conn::stream_audio(ch, &node_id, pcm_rx, reload_tx, music_handle_clone).await
        });

        let mut kill_rx = kill_tx.subscribe();
        publish_state(&state_tx, NodeState::Processing);

        'stream: loop {
            tokio::select! {
                Some(samples) = audio_rx.recv() => {
                    if pcm_tx.send(samples).await.is_err() {
                        break 'stream;
                    }
                }
                result = &mut stream_task => {
                    if let Ok(Err(e)) = result {
                        tracing::warn!("brain stream error: {e}");
                    }
                    break 'stream;
                }
                Ok(_) = kill_rx.recv() => {
                    tracing::warn!("kill signal received — aborting audio stream");
                    stream_task.abort();
                    break 'stream;
                }
            }
        }

        // If a model update arrived during this session, rebuild the detector
        // now so the next wake word cycle uses the new model.
        if model_reload_rx.try_recv().is_ok() {
            // Drain any extra signals (in case multiple updates arrived).
            while model_reload_rx.try_recv().is_ok() {}
            match wake_word::build(&model_path) {
                Ok(d) => {
                    detector = d;
                    tracing::info!("wake word detector hot-reloaded from updated model");
                }
                Err(e) => tracing::warn!("failed to reload wake word model: {e}"),
            }
        }

        publish_state(&state_tx, NodeState::Idle);
        tracing::info!("stream ended — returning to idle");
    }
}

/// Auxiliary mode: mirror a primary node's LED state on this device.
async fn run_auxiliary(target: &str) -> Result<()> {
    tracing::info!(%target, "auxiliary mode starting");

    // ── GPIO LED (Raspberry Pi only) ─────────────────────────────────────────
    #[cfg(feature = "gpio")]
    let led = gpio::LedController::from_env().context("opening LED GPIO")?;
    #[cfg(feature = "gpio")]
    let led = std::sync::Arc::new(std::sync::Mutex::new(led));

    let on_state = {
        #[cfg(feature = "gpio")]
        let led = led.clone();
        move |state: NodeState| {
            tracing::info!(?state, "mirroring primary node state");
            #[cfg(feature = "gpio")]
            if let Ok(mut guard) = led.lock() {
                if let Some(ref mut led) = *guard {
                    led.apply(gpio::state_to_pattern(state));
                }
            }
            #[cfg(not(feature = "gpio"))]
            tracing::info!(
                ?state,
                "auxiliary: gpio feature not enabled, LED mirroring skipped"
            );
        }
    };

    state_server::subscribe_to_primary(target, on_state).await?;
    Ok(())
}

/// Publish a state change to the SSE broadcast channel; log failures.
fn publish_state(tx: &broadcast::Sender<NodeState>, state: NodeState) {
    if tx.send(state).is_err() {
        // No active subscribers — fine, auxiliary nodes may not be connected.
    }
}
