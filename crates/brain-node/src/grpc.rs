use crate::session::SessionRegistry;
use aether_core::NodeState;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

pub mod proto {
    tonic::include_proto!("aether");
}

use proto::{
    aether_brain_server::AetherBrain, AudioChunk, BrainResponse, PairRequest, PairResponse,
};

#[derive(Clone)]
pub struct BrainService {
    pub registry: SessionRegistry,
    pub certs_dir: std::path::PathBuf,
}

#[tonic::async_trait]
impl AetherBrain for BrainService {
    type AudioStreamStream = ReceiverStream<Result<BrainResponse, Status>>;

    async fn audio_stream(
        &self,
        request: Request<Streaming<AudioChunk>>,
    ) -> Result<Response<Self::AudioStreamStream>, Status> {
        let node_id = request
            .metadata()
            .get("x-node-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| Status::unauthenticated("missing x-node-id metadata"))?;

        tracing::info!(node_id = %node_id, "audio stream opened");

        let registry = self.registry.clone();
        let nid = node_id.clone();

        registry.register(node_id.clone()).await;
        registry.set_state(&node_id, NodeState::Listening).await;

        let mut stream = request.into_inner();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<BrainResponse, Status>>(32);

        tokio::spawn(async move {
            let mut expected_seq = 0u64;
            while let Ok(Some(chunk)) = stream.message().await {
                if chunk.seq != expected_seq {
                    tracing::warn!(
                        node_id = %nid,
                        expected = expected_seq,
                        got = chunk.seq,
                        "out-of-order PCM chunk"
                    );
                }
                expected_seq = chunk.seq.wrapping_add(1);
                // Phase 1: consume and discard — STT/LLM/TTS wired in Phase 2.
                let _ = tx; // keep tx alive; unused responses until Phase 2
            }

            registry.set_state(&nid, NodeState::Idle).await;
            registry.unregister(&nid).await;
            tracing::info!(node_id = %nid, "audio stream closed");
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn pair(&self, request: Request<PairRequest>) -> Result<Response<PairResponse>, Status> {
        let node_id = request.into_inner().node_id;
        tracing::info!(node_id = %node_id, "pairing request received");

        // Prompt operator to approve.
        println!("\n>>> Pairing request from node: \"{node_id}\" <<<");
        println!("Press ENTER to approve, Ctrl-C to deny...");
        let mut buf = String::new();
        std::io::stdin()
            .read_line(&mut buf)
            .map_err(|e| Status::internal(format!("stdin: {e}")))?;

        let ca_cert_pem = std::fs::read_to_string(self.certs_dir.join("ca.pem"))
            .map_err(|e| Status::internal(format!("CA cert missing: {e}")))?;
        let ca_key_pem = std::fs::read_to_string(self.certs_dir.join("ca-key.pem"))
            .map_err(|e| Status::internal(format!("CA key missing: {e}")))?;
        let ca_key = rcgen::KeyPair::from_pem(&ca_key_pem)
            .map_err(|e| Status::internal(format!("CA key parse: {e}")))?;

        let issued = crate::pair::issue_client_cert(&node_id, &ca_key)
            .map_err(|e| Status::internal(format!("cert issuance: {e}")))?;

        tracing::info!(node_id = %node_id, "client cert issued");

        Ok(Response::new(PairResponse {
            client_private_key: issued.key_pem.into_bytes(),
            client_certificate: issued.cert_pem.into_bytes(),
            ca_certificate: ca_cert_pem.into_bytes(),
        }))
    }
}
