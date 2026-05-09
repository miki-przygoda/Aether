use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::path::Path;
use std::time::Duration;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity};
use tonic::Request;

pub mod proto {
    tonic::include_proto!("aether");
}

use proto::{aether_brain_client::AetherBrainClient, brain_response, AudioChunk, PairRequest};

/// Stored configuration written to disk during pairing.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct PairedConfig {
    pub node_id: String,
    pub brain_addr: String, // e.g. "192.168.1.100:50051"
}

impl PairedConfig {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("config.json");
        let data = std::fs::read_to_string(&path)
            .with_context(|| format!("reading config from {}", path.display()))?;
        serde_json::from_str(&data).context("parsing config.json")
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let path = config_dir.join("config.json");
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Execute the pairing ceremony against the brain's plain gRPC port.
/// Writes certs and config to `config_dir`.
pub async fn pair(brain_addr: &str, node_id: &str, config_dir: &Path) -> Result<()> {
    tracing::info!(brain = %brain_addr, node_id, "starting pairing ceremony");

    // Plain (non-TLS) connection to the pairing port.
    let endpoint = format!("http://{brain_addr}");
    let channel = Channel::from_shared(endpoint)?
        .connect()
        .await
        .context("connecting to brain pairing port")?;

    let mut client = AetherBrainClient::new(channel);
    let resp = client
        .pair(Request::new(PairRequest {
            node_id: node_id.to_string(),
        }))
        .await
        .context("pair RPC")?
        .into_inner();

    std::fs::create_dir_all(config_dir)?;
    std::fs::write(config_dir.join("client.pem"), &resp.client_certificate)?;
    std::fs::write(config_dir.join("client-key.pem"), &resp.client_private_key)?;
    std::fs::write(config_dir.join("ca.pem"), &resp.ca_certificate)?;

    // Derive the mTLS service address (same IP, port 50051).
    let host = brain_addr.split(':').next().unwrap_or(brain_addr);
    let service_addr = format!("{host}:50051");

    let config = PairedConfig {
        node_id: node_id.to_string(),
        brain_addr: service_addr,
    };
    config.save(config_dir)?;

    tracing::info!(
        "pairing complete — certs and config saved to {}",
        config_dir.display()
    );
    Ok(())
}

/// Resolve the brain's current address.
/// Uses the stored address first; falls back to mDNS discovery.
pub async fn resolve_brain(config_dir: &Path) -> Result<String> {
    if let Ok(cfg) = PairedConfig::load(config_dir) {
        tracing::debug!(addr = %cfg.brain_addr, "using stored brain address");
        return Ok(cfg.brain_addr);
    }
    tracing::info!("no stored address — discovering brain via mDNS");
    discover_via_mdns().await
}

async fn discover_via_mdns() -> Result<String> {
    tokio::task::spawn_blocking(|| {
        let mdns = ServiceDaemon::new()?;
        let receiver = mdns.browse("_aether._tcp.local.")?;

        loop {
            match receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if let Some(addr) = info.get_addresses().iter().next() {
                        let port = info.get_port();
                        return Ok(format!("{addr}:{port}"));
                    }
                }
                Ok(_) => continue,
                Err(_) => anyhow::bail!("mDNS discovery timed out after 30 s"),
            }
        }
    })
    .await?
}

/// Build an mTLS channel to the brain using stored certs.
pub fn build_mtls_channel(brain_addr: &str, config_dir: &Path) -> Result<Channel> {
    let ca_pem = std::fs::read(config_dir.join("ca.pem")).context("reading CA cert")?;
    let cert_pem = std::fs::read(config_dir.join("client.pem")).context("reading client cert")?;
    let key_pem = std::fs::read(config_dir.join("client-key.pem")).context("reading client key")?;

    let ca = Certificate::from_pem(&ca_pem);
    let identity = Identity::from_pem(&cert_pem, &key_pem);
    let tls = ClientTlsConfig::new().ca_certificate(ca).identity(identity);

    let endpoint = format!("https://{brain_addr}");
    let channel = Channel::from_shared(endpoint)?
        .tls_config(tls)?
        .connect_lazy();

    Ok(channel)
}

/// Open a bidirectional PCM stream to the brain.
/// `pcm_rx` is the audio feed from cpal; this function drives it until the
/// channel closes or an error occurs.
pub async fn stream_audio(
    channel: Channel,
    node_id: &str,
    mut pcm_rx: tokio::sync::mpsc::Receiver<Vec<f32>>,
) -> Result<()> {
    let mut client = AetherBrainClient::new(channel);

    let node_id = node_id.to_string();
    let mut seq: u64 = 0;

    let outbound = async_stream::stream! {
        while let Some(samples) = pcm_rx.recv().await {
            // Convert f32 samples to raw bytes (f32le).
            let pcm: Vec<u8> = samples
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect();
            yield AudioChunk { pcm, seq };
            seq = seq.wrapping_add(1);
        }
    };

    let mut request = Request::new(outbound);
    request.metadata_mut().insert("x-node-id", node_id.parse()?);

    let mut response = client.audio_stream(request).await?.into_inner();

    while let Some(msg) = response.message().await? {
        match msg.payload {
            Some(brain_response::Payload::Transcript(t)) => {
                tracing::info!(text = %t.text, confidence = t.confidence, "transcript");
            }
            Some(brain_response::Payload::Action(a)) => {
                tracing::info!(action = %a.action, params = %a.params_json, "skill action");
            }
            Some(brain_response::Payload::TtsAudio(chunk)) => {
                tracing::info!(bytes = chunk.wav.len(), "TTS audio received — playing");
                if let Err(e) =
                    tokio::task::spawn_blocking(move || crate::playback::play_wav(&chunk.wav))
                        .await
                        .expect("playback task panicked")
                {
                    tracing::warn!("TTS playback error: {e}");
                }
            }
            Some(brain_response::Payload::WakeWordModel(update)) => {
                tracing::info!(
                    bytes = update.model_bytes.len(),
                    "wake word model update received — hot-reloading"
                );
                if let Ok(path) = std::env::var("AETHER_MODEL_PATH") {
                    match std::fs::write(&path, &update.model_bytes) {
                        Ok(()) => tracing::info!(%path, "wake word model written"),
                        Err(e) => tracing::warn!(%path, "failed to write model: {e}"),
                    }
                } else {
                    tracing::warn!("AETHER_MODEL_PATH not set — cannot save model update");
                }
            }
            None => {}
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::PairedConfig;

    #[test]
    fn seq_wraps_at_u64_max() {
        // Validates the wrapping_add pattern used in stream_audio.
        assert_eq!(u64::MAX.wrapping_add(1), 0);
    }

    #[test]
    fn paired_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = PairedConfig {
            node_id: "office-pi".to_string(),
            brain_addr: "192.168.1.100:50051".to_string(),
        };
        cfg.save(dir.path()).unwrap();

        let loaded = PairedConfig::load(dir.path()).unwrap();
        assert_eq!(loaded.node_id, "office-pi");
        assert_eq!(loaded.brain_addr, "192.168.1.100:50051");
    }

    #[test]
    fn paired_config_load_fails_on_missing_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Point at a subdirectory that doesn't exist.
        let missing = dir.path().join("no_such_dir");
        assert!(PairedConfig::load(&missing).is_err());
    }
}
