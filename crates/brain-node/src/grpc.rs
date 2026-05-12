use crate::history::{self, Role};
use crate::ingest;
use crate::llm::LlmClient;
use crate::session::SessionRegistry;
use crate::skills::SkillRegistry;
use crate::stt::{bytes_to_f32le, SpeechToText};
use crate::tts::TextToSpeech;
use crate::vector_store::{VectorStore, COLLECTION_DOCUMENTS};
use aether_core::trie::{ClassifyResult, CommandTrie};
use aether_core::{NodeState, TtsSettings};
use std::sync::Arc as StdArc;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};

pub mod proto {
    tonic::include_proto!("aether");
}

use proto::{
    aether_brain_server::AetherBrain, brain_response, AudioChunk, BrainResponse, PairRequest,
    PairResponse, SkillAction, TranscriptUpdate, TtsChunk, WakeWordModelUpdate,
};

/// Configuration for RAG and conversation history.
/// `None` disables both features (used in tests and when Qdrant is not configured).
#[derive(Clone)]
pub struct RagConfig {
    pub store: Arc<dyn VectorStore>,
    /// Qdrant base URL (e.g. `http://qdrant:6334`) for scroll-based history retrieval.
    pub qdrant_url: String,
    /// Ollama base URL used for embedding the user query before searching documents.
    pub embed_url: String,
    pub embed_model: String,
    /// Number of past turns to inject as conversation history.
    pub history_turns: usize,
    /// Minimum cosine similarity score for a document chunk to be included in context.
    pub score_threshold: f32,
}

#[derive(Clone)]
pub struct BrainService {
    pub registry: SessionRegistry,
    pub certs_dir: std::path::PathBuf,
    pub config_dir: std::path::PathBuf,
    pub stt: Option<Arc<dyn SpeechToText>>,
    pub trie: Arc<CommandTrie>,
    pub llm: Option<Arc<dyn LlmClient>>,
    pub tts: Option<Arc<dyn TextToSpeech>>,
    /// Shared with the web UI — the UI can update settings and the next synthesis picks them up.
    pub tts_settings: StdArc<RwLock<TtsSettings>>,
    pub skills: Arc<SkillRegistry>,
    pub rag: Option<RagConfig>,
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
        let llm = self.llm.clone();
        let tts = self.tts.clone();
        let tts_settings: StdArc<RwLock<TtsSettings>> = self.tts_settings.clone();
        let skills = self.skills.clone();
        let rag = self.rag.clone();

        registry.register(node_id.clone()).await;
        registry.set_state(&node_id, NodeState::Listening).await;

        let mut stream = request.into_inner();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<BrainResponse, Status>>(32);

        // Per-session push channel: allows deploy_wake_word to send model bytes
        // to this node while it is mid-stream.
        let (push_tx, mut push_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        registry.register_push(node_id.clone(), push_tx).await;

        tokio::spawn(async move {
            let mut pcm_buf: Vec<f32> = Vec::new();
            let mut expected_seq = 0u64;

            loop {
                tokio::select! {
                    result = stream.message() => {
                        match result {
                            Ok(Some(chunk)) => {
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
                            _ => break,
                        }
                    }
                    Some(model_bytes) = push_rx.recv() => {
                        tracing::info!(node_id = %nid, bytes = model_bytes.len(), "sending wake word model update");
                        let _ = tx.send(Ok(BrainResponse {
                            payload: Some(brain_response::Payload::WakeWordModel(
                                WakeWordModelUpdate { model_bytes }
                            )),
                        })).await;
                    }
                }
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

                        let _ = tx
                            .send(Ok(BrainResponse {
                                payload: Some(brain_response::Payload::Transcript(
                                    TranscriptUpdate {
                                        text: t.text.clone(),
                                        is_final: true,
                                        confidence: t.confidence,
                                    },
                                )),
                            }))
                            .await;

                        // Shared helper: send SkillAction then optionally synthesise TTS.
                        let dispatch = {
                            let tx = tx.clone();
                            let tts = tts.clone();
                            let skills = skills.clone();
                            let nid2 = nid2.clone();
                            move |action_str: String,
                                  params: serde_json::Value,
                                  params_json: String| {
                                let skill_result = skills.dispatch(&action_str, &params);
                                tracing::info!(
                                    node_id = %nid2,
                                    reply = %skill_result.spoken_reply,
                                    "skill dispatched"
                                );
                                (tx, tts, skill_result.spoken_reply, action_str, params_json)
                            }
                        };

                        match trie.classify(&t.text) {
                            ClassifyResult::Match(action) => {
                                let action_str = action.as_str().to_string();
                                tracing::info!(
                                    node_id = %nid2,
                                    action = %action_str,
                                    "trie matched — dispatching directly"
                                );
                                let params = serde_json::Value::Object(Default::default());
                                let (tx, tts, spoken_reply, action_str, params_json) =
                                    dispatch(action_str, params, "{}".to_string());
                                let _ = tx
                                    .send(Ok(BrainResponse {
                                        payload: Some(brain_response::Payload::Action(
                                            SkillAction {
                                                action: action_str,
                                                params_json,
                                            },
                                        )),
                                    }))
                                    .await;
                                synthesise_and_send(
                                    &tx,
                                    tts.clone(),
                                    &spoken_reply,
                                    &nid2,
                                    tts_settings.clone(),
                                )
                                .await;
                            }
                            _ => {
                                if let Some(llm) = llm {
                                    let text = t.text;
                                    let nid3 = nid2.clone();
                                    let nid3_log = nid3.clone();
                                    let rag3 = rag.clone();
                                    match tokio::task::spawn_blocking(move || {
                                        ask_with_rag(&*llm, rag3.as_ref(), &nid3, &text)
                                    })
                                    .await
                                    {
                                        Ok(Ok(resp)) => {
                                            tracing::info!(
                                                node_id = %nid3_log,
                                                action = ?resp.action,
                                                "LLM response ready"
                                            );
                                            let action_str = resp
                                                .action
                                                .unwrap_or_else(|| "respond".to_string());
                                            let mut params = resp.params.unwrap_or(
                                                serde_json::Value::Object(Default::default()),
                                            );
                                            params["response"] =
                                                serde_json::Value::String(resp.response);
                                            let params_json = params.to_string();
                                            let (tx, tts, spoken_reply, action_str, params_json) =
                                                dispatch(action_str, params, params_json);
                                            let _ = tx
                                                .send(Ok(BrainResponse {
                                                    payload: Some(brain_response::Payload::Action(
                                                        SkillAction {
                                                            action: action_str,
                                                            params_json,
                                                        },
                                                    )),
                                                }))
                                                .await;
                                            synthesise_and_send(
                                                &tx,
                                                tts.clone(),
                                                &spoken_reply,
                                                &nid3_log,
                                                tts_settings.clone(),
                                            )
                                            .await;
                                        }
                                        Ok(Err(e)) => {
                                            tracing::error!(node_id = %nid3_log, "LLM error: {e}")
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                node_id = %nid3_log,
                                                "LLM task panicked: {e}"
                                            )
                                        }
                                    }
                                } else {
                                    tracing::info!(
                                        node_id = %nid2,
                                        "LLM not configured — no response sent"
                                    );
                                }
                            }
                        }
                    }
                    Ok(Err(e)) => tracing::error!(node_id = %nid, "STT error: {e}"),
                    Err(e) => tracing::error!(node_id = %nid, "STT task panicked: {e}"),
                }
            }

            registry.unregister_push(&nid).await;
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

        crate::web_ui::register_paired_node(&self.config_dir, &node_id);
        tracing::info!(node_id = %node_id, "client cert issued");

        Ok(Response::new(PairResponse {
            client_private_key: issued.key_pem.into_bytes(),
            client_certificate: issued.cert_pem.into_bytes(),
            ca_certificate: ca_cert_pem.into_bytes(),
        }))
    }
}

// ─── RAG helpers ──────────────────────────────────────────────────────────────

/// Build an augmented prompt that prepends conversation history and document
/// context to the raw user transcript. Falls back to the plain transcript when
/// both history and context are empty (no RAG configured, first turn, etc.).
fn build_rag_prompt(transcript: &str, history_text: &str, context_text: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !history_text.is_empty() {
        parts.push(format!("[History]\n{history_text}"));
    }
    if !context_text.is_empty() {
        parts.push(format!("[Context]\n{context_text}"));
    }
    if parts.is_empty() {
        return transcript.to_string();
    }
    parts.push(format!("[User]\n{transcript}"));
    parts.join("\n\n")
}

/// Call the LLM with RAG context and conversation history injected into the prompt.
/// Also stores the user and assistant turns in the history collection.
///
/// When `rag` is `None` this degrades to a plain `llm.ask(transcript)` call.
fn ask_with_rag(
    llm: &dyn LlmClient,
    rag: Option<&RagConfig>,
    node_id: &str,
    transcript: &str,
) -> anyhow::Result<aether_core::LlmResponse> {
    // 1. Retrieve conversation history.
    let history_text = rag
        .map(|r| {
            history::scroll_recent(&r.qdrant_url, node_id, r.history_turns)
                .map(|turns| history::format_history(&turns))
                .unwrap_or_else(|e| {
                    tracing::warn!(node_id, "history retrieval failed: {e}");
                    String::new()
                })
        })
        .unwrap_or_default();

    // 2. Embed query and retrieve top document chunks.
    let context_text = rag
        .map(|r| {
            ingest::embed(&r.embed_url, &r.embed_model, transcript)
                .and_then(|vec| r.store.search(COLLECTION_DOCUMENTS, vec, 3))
                .map(|results| {
                    let chunks: Vec<&str> = results
                        .iter()
                        .filter(|res| res.score >= r.score_threshold)
                        .filter_map(|res| res.payload["text"].as_str())
                        .collect();
                    if chunks.is_empty() {
                        tracing::debug!(node_id, "RAG: no chunks above score threshold");
                    }
                    chunks.join("\n\n---\n\n")
                })
                .unwrap_or_else(|e| {
                    tracing::warn!(node_id, "RAG retrieval failed: {e}");
                    String::new()
                })
        })
        .unwrap_or_default();

    // 3. Store user turn before calling LLM (so it's in history on retry).
    if let Some(r) = rag {
        if let Err(e) = history::store_turn(&r.store, node_id, Role::User, transcript) {
            tracing::warn!(node_id, "failed to store user turn: {e}");
        }
    }

    // 4. Call LLM with augmented prompt.
    let prompt = build_rag_prompt(transcript, &history_text, &context_text);
    let resp = llm.ask(&prompt)?;

    // 5. Store assistant turn.
    if let Some(r) = rag {
        if let Err(e) = history::store_turn(&r.store, node_id, Role::Assistant, &resp.response) {
            tracing::warn!(node_id, "failed to store assistant turn: {e}");
        }
    }

    Ok(resp)
}

// ─── TTS helper ───────────────────────────────────────────────────────────────

async fn synthesise_and_send(
    tx: &tokio::sync::mpsc::Sender<Result<BrainResponse, Status>>,
    tts: Option<Arc<dyn TextToSpeech>>,
    text: &str,
    node_id: &str,
    settings: StdArc<RwLock<TtsSettings>>,
) {
    let Some(tts) = tts else { return };
    let text = text.to_string();
    let nid = node_id.to_string();
    let settings = settings.read().await.clone();
    match tokio::task::spawn_blocking(move || tts.synthesise(&text, &settings)).await {
        Ok(Ok(wav)) => {
            tracing::info!(node_id = %nid, bytes = wav.len(), "TTS chunk ready");
            let _ = tx
                .send(Ok(BrainResponse {
                    payload: Some(brain_response::Payload::TtsAudio(TtsChunk { wav })),
                }))
                .await;
        }
        Ok(Err(e)) => tracing::error!(node_id = %nid, "TTS error: {e}"),
        Err(e) => tracing::error!(node_id = %nid, "TTS task panicked: {e}"),
    }
}
