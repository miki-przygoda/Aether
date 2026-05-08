use crate::session::SessionRegistry;
use crate::stt::{bytes_to_f32le, SpeechToText};
use aether_core::trie::{ClassifyResult, CommandTrie};
use aether_core::NodeState;
use std::sync::Arc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

pub mod proto {
    tonic::include_proto!("aether");
}

use proto::{
    aether_brain_server::AetherBrain, brain_response, AudioChunk, BrainResponse, PairRequest,
    PairResponse, SkillAction, TranscriptUpdate,
};

#[derive(Clone)]
pub struct BrainService {
    pub registry: SessionRegistry,
    pub certs_dir: std::path::PathBuf,
    pub stt: Option<Arc<dyn SpeechToText>>,
    pub trie: Arc<CommandTrie>,
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
        let stt = self.stt.clone();
        let trie = self.trie.clone();

        registry.register(node_id.clone()).await;
        registry.set_state(&node_id, NodeState::Listening).await;

        let mut stream = request.into_inner();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<BrainResponse, Status>>(32);

        tokio::spawn(async move {
            let mut pcm_buf: Vec<f32> = Vec::new();
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
                pcm_buf.extend(bytes_to_f32le(&chunk.pcm));
            }

            if let Some(stt) = stt {
                registry.set_state(&nid, NodeState::Processing).await;
                let nid2 = nid.clone();
                match tokio::task::spawn_blocking(move || stt.transcribe(&pcm_buf)).await {
                    Ok(Ok(t)) => {
                        tracing::info!(
                            node_id = %nid2,
                            text = %t.text,
                            confidence = t.confidence,
                            "transcript ready"
                        );

                        let transcript_msg = BrainResponse {
                            payload: Some(brain_response::Payload::Transcript(TranscriptUpdate {
                                text: t.text.clone(),
                                is_final: true,
                                confidence: t.confidence,
                            })),
                        };
                        if tx.send(Ok(transcript_msg)).await.is_err() {
                            tracing::warn!(
                                node_id = %nid2,
                                "edge disconnected before transcript delivered"
                            );
                        }

                        match trie.classify(&t.text) {
                            ClassifyResult::Match(action) => {
                                tracing::info!(
                                    node_id = %nid2,
                                    action = action.as_str(),
                                    "trie matched — dispatching directly"
                                );
                                let action_msg = BrainResponse {
                                    payload: Some(brain_response::Payload::Action(SkillAction {
                                        action: action.as_str().to_string(),
                                        params_json: "{}".to_string(),
                                    })),
                                };
                                let _ = tx.send(Ok(action_msg)).await;
                            }
                            _ => {
                                // TODO: route to LLM fast tier (Phase 2 PR 3)
                                tracing::info!(
                                    node_id = %nid2,
                                    "no trie match — LLM path not yet implemented"
                                );
                            }
                        }
                    }
                    Ok(Err(e)) => tracing::error!(node_id = %nid, "STT error: {e}"),
                    Err(e) => tracing::error!(node_id = %nid, "STT task panicked: {e}"),
                }
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
