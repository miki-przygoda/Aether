use crate::grpc::{
    proto::{
        aether_brain_client::AetherBrainClient, aether_brain_server::AetherBrainServer, AudioChunk,
        BrainResponse,
    },
    BrainService,
};
use crate::llm::LlmClient;
use crate::session::SessionRegistry;
use crate::skills::SkillRegistry;
use crate::stt::{SpeechToText, TranscriptResult};
use crate::tts::{encode_wav, TextToSpeech};
use aether_core::LlmResponse;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, Server, ServerTlsConfig};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Spin up a plain (no-TLS) BrainService on a random localhost port.
async fn start_plain_server() -> (std::net::SocketAddr, SessionRegistry) {
    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
        stt: None,
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: None,
        tts: None,
        skills: Arc::new(SkillRegistry::default()),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );

    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    (addr, registry)
}

/// Poll until the registry reaches the expected count or the timeout elapses.
async fn wait_for_count(registry: &SessionRegistry, expected: usize, timeout_ms: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if registry.count().await == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for session count to reach {expected} (currently {})",
            registry.count().await
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

// ─── mock STT ─────────────────────────────────────────────────────────────────

struct MockStt {
    text: String,
    confidence: f32,
}

impl SpeechToText for MockStt {
    fn transcribe(&self, _pcm: &[f32]) -> anyhow::Result<TranscriptResult> {
        Ok(TranscriptResult {
            text: self.text.clone(),
            confidence: self.confidence,
        })
    }
}

// ─── mock LLM ─────────────────────────────────────────────────────────────────

struct MockLlm {
    response: LlmResponse,
}

impl LlmClient for MockLlm {
    fn ask(&self, _transcript: &str) -> anyhow::Result<LlmResponse> {
        Ok(self.response.clone())
    }
}

// ─── mock TTS ─────────────────────────────────────────────────────────────────

struct MockTts;

impl TextToSpeech for MockTts {
    fn synthesise(&self, _text: &str) -> anyhow::Result<Vec<u8>> {
        // Return a minimal valid WAV (silence, 100 samples at 24 kHz).
        encode_wav(&vec![0.0f32; 100], 24_000)
    }
}

// ─── tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn three_concurrent_audio_streams_register_and_cleanup() {
    let (addr, registry) = start_plain_server().await;

    let mut senders = Vec::new();

    for i in 0..3 {
        let ep = format!("http://{addr}");
        let node_id = format!("test-pi-{i}");
        let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(8);
        senders.push(tx);

        tokio::spawn(async move {
            let channel = tonic::transport::Channel::from_shared(ep)
                .unwrap()
                .connect()
                .await
                .unwrap();
            let mut client = AetherBrainClient::new(channel);
            let stream = ReceiverStream::new(rx);
            let mut req = tonic::Request::new(stream);
            req.metadata_mut()
                .insert("x-node-id", node_id.parse().unwrap());
            if let Ok(resp) = client.audio_stream(req).await {
                let mut rs = resp.into_inner();
                while let Ok(Some(_)) = rs.message().await {}
            }
        });
    }

    wait_for_count(&registry, 3, 2_000).await;
    assert_eq!(
        registry.count().await,
        3,
        "all three sessions should be active"
    );

    drop(senders);

    wait_for_count(&registry, 0, 2_000).await;
    assert_eq!(
        registry.count().await,
        0,
        "all sessions should be cleaned up"
    );
}

#[tokio::test]
async fn audio_stream_requires_node_id_metadata() {
    let (addr, _registry) = start_plain_server().await;

    let ep = format!("http://{addr}");
    let channel = tonic::transport::Channel::from_shared(ep)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = AetherBrainClient::new(channel);

    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(1);
    let stream = ReceiverStream::new(rx);
    drop(tx);

    let req = tonic::Request::new(stream);
    let err = client.audio_stream(req).await.unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::Unauthenticated,
        "missing metadata should yield Unauthenticated"
    );
}

/// Full mTLS handshake: real CA → server cert (IP SAN 127.0.0.1) → client cert.
#[tokio::test]
async fn mtls_audio_stream_handshake_and_pcm_delivery() {
    let ca = crate::pair::generate_ca().unwrap();
    let ca_key = rcgen::KeyPair::from_pem(&ca.key_pem).unwrap();
    let brain_ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let server_cert = crate::pair::generate_server_cert(&ca_key, brain_ip).unwrap();
    let client_cert = crate::pair::issue_client_cert("mtls-test-pi", &ca_key).unwrap();

    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
        stt: None,
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: None,
        tts: None,
        skills: Arc::new(SkillRegistry::default()),
    };

    let server_tls = ServerTlsConfig::new()
        .identity(Identity::from_pem(
            &server_cert.cert_pem,
            &server_cert.key_pem,
        ))
        .client_ca_root(Certificate::from_pem(&ca.cert_pem));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(
        Server::builder()
            .tls_config(server_tls)
            .unwrap()
            .add_service(AetherBrainServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(&ca.cert_pem))
        .identity(Identity::from_pem(
            &client_cert.cert_pem,
            &client_cert.key_pem,
        ));

    let channel = Channel::from_shared(format!("https://{addr}"))
        .unwrap()
        .tls_config(tls)
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(16);
    let node_id = "mtls-test-pi";

    tokio::spawn(async move {
        let mut client = AetherBrainClient::new(channel);
        let stream = ReceiverStream::new(rx);
        let mut req = tonic::Request::new(stream);
        req.metadata_mut()
            .insert("x-node-id", node_id.parse().unwrap());
        if let Ok(resp) = client.audio_stream(req).await {
            let mut rs = resp.into_inner();
            while let Ok(Some(_)) = rs.message().await {}
        }
    });

    wait_for_count(&registry, 1, 2_000).await;

    for seq in 0u64..3 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 512],
            seq,
        })
        .await
        .unwrap();
    }
    drop(tx);

    wait_for_count(&registry, 0, 2_000).await;
}

#[tokio::test]
async fn audio_chunk_seq_is_accepted_in_order() {
    let (addr, registry) = start_plain_server().await;

    let ep = format!("http://{addr}");
    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(16);
    let node_id = "seq-test-node";

    tokio::spawn(async move {
        let channel = tonic::transport::Channel::from_shared(ep)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = AetherBrainClient::new(channel);
        let stream = ReceiverStream::new(rx);
        let mut req = tonic::Request::new(stream);
        req.metadata_mut()
            .insert("x-node-id", node_id.parse().unwrap());
        if let Ok(resp) = client.audio_stream(req).await {
            let mut rs = resp.into_inner();
            while let Ok(Some(_)) = rs.message().await {}
        }
    });

    wait_for_count(&registry, 1, 1_000).await;

    for seq in 0u64..5 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 32],
            seq,
        })
        .await
        .unwrap();
    }

    drop(tx);
    wait_for_count(&registry, 0, 2_000).await;
}

/// MockStt returns a fixed transcript — verifies the full PCM-in → TranscriptUpdate-out path
/// without requiring a real Whisper model.
#[tokio::test]
async fn stt_transcription_sends_transcript_update() {
    let stt: Arc<dyn SpeechToText> = Arc::new(MockStt {
        text: "hello world".to_string(),
        confidence: 0.92,
    });

    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
        stt: Some(stt),
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: None,
        tts: None,
        skills: Arc::new(SkillRegistry::default()),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    tokio::time::sleep(Duration::from_millis(25)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(8);
    let mut client = AetherBrainClient::new(channel);
    let stream = ReceiverStream::new(rx);
    let mut req = tonic::Request::new(stream);
    req.metadata_mut()
        .insert("x-node-id", "stt-test-pi".parse().unwrap());

    let resp = client.audio_stream(req).await.unwrap();
    let mut resp_stream = resp.into_inner();

    // Send 3 fake PCM chunks then close the stream.
    for seq in 0u64..3 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 2048],
            seq,
        })
        .await
        .unwrap();
    }
    drop(tx);

    let msg: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for TranscriptUpdate")
        .expect("stream error")
        .expect("stream closed without message");

    use crate::grpc::proto::brain_response;
    match msg.payload {
        Some(brain_response::Payload::Transcript(t)) => {
            assert_eq!(t.text, "hello world");
            assert!(t.is_final);
            assert!((t.confidence - 0.92).abs() < 1e-5, "confidence mismatch");
        }
        other => panic!("expected Transcript payload, got: {other:?}"),
    }
}

/// MockStt returns "play music" — verifies trie dispatch: brain sends TranscriptUpdate
/// immediately followed by a SkillAction with action = "play_music", no LLM call needed.
#[tokio::test]
async fn trie_match_sends_skill_action() {
    use crate::grpc::proto::brain_response;

    let stt: Arc<dyn SpeechToText> = Arc::new(MockStt {
        text: "play music".to_string(),
        confidence: 0.95,
    });

    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
        stt: Some(stt),
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: None,
        tts: None,
        skills: Arc::new(SkillRegistry::default()),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    tokio::time::sleep(Duration::from_millis(25)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(8);
    let mut client = AetherBrainClient::new(channel);
    let stream = ReceiverStream::new(rx);
    let mut req = tonic::Request::new(stream);
    req.metadata_mut()
        .insert("x-node-id", "trie-test-pi".parse().unwrap());

    let resp = client.audio_stream(req).await.unwrap();
    let mut resp_stream = resp.into_inner();

    for seq in 0u64..2 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 2048],
            seq,
        })
        .await
        .unwrap();
    }
    drop(tx);

    // First message: TranscriptUpdate.
    let first: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for TranscriptUpdate")
        .expect("stream error")
        .expect("stream closed before TranscriptUpdate");

    match first.payload {
        Some(brain_response::Payload::Transcript(t)) => {
            assert_eq!(t.text, "play music");
            assert!(t.is_final);
        }
        other => panic!("expected Transcript, got: {other:?}"),
    }

    // Second message: SkillAction dispatched by the trie.
    let second: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for SkillAction")
        .expect("stream error")
        .expect("stream closed before SkillAction");

    match second.payload {
        Some(brain_response::Payload::Action(a)) => {
            assert_eq!(a.action, "play_music");
        }
        other => panic!("expected Action, got: {other:?}"),
    }
}

/// MockStt returns "what time is it" (trie: NoMatch) and MockLlm returns a respond action —
/// verifies the full no-trie-match → LLM → SkillAction path without a real model or Ollama.
#[tokio::test]
async fn llm_invoked_on_trie_no_match_sends_skill_action() {
    use crate::grpc::proto::brain_response;

    let stt: Arc<dyn SpeechToText> = Arc::new(MockStt {
        text: "what time is it".to_string(),
        confidence: 0.88,
    });
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlm {
        response: LlmResponse {
            action: Some("respond".to_string()),
            params: None,
            response: "I don't have access to a clock.".to_string(),
        },
    });

    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
        stt: Some(stt),
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: Some(llm),
        tts: None,
        skills: Arc::new(SkillRegistry::default()),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    tokio::time::sleep(Duration::from_millis(25)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(8);
    let mut client = AetherBrainClient::new(channel);
    let stream = ReceiverStream::new(rx);
    let mut req = tonic::Request::new(stream);
    req.metadata_mut()
        .insert("x-node-id", "llm-test-pi".parse().unwrap());

    let resp = client.audio_stream(req).await.unwrap();
    let mut resp_stream = resp.into_inner();

    for seq in 0u64..2 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 2048],
            seq,
        })
        .await
        .unwrap();
    }
    drop(tx);

    // First message: TranscriptUpdate.
    let first: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for TranscriptUpdate")
        .expect("stream error")
        .expect("stream closed before TranscriptUpdate");

    match first.payload {
        Some(brain_response::Payload::Transcript(t)) => {
            assert_eq!(t.text, "what time is it");
        }
        other => panic!("expected Transcript, got: {other:?}"),
    }

    // Second message: SkillAction from LLM (trie returned NoMatch).
    let second: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for SkillAction from LLM")
        .expect("stream error")
        .expect("stream closed before LLM SkillAction");

    match second.payload {
        Some(brain_response::Payload::Action(a)) => {
            assert_eq!(a.action, "respond", "LLM action should be 'respond'");
        }
        other => panic!("expected Action from LLM, got: {other:?}"),
    }
}

/// MockTts returns a valid WAV — verifies the brain sends a TtsChunk after the SkillAction.
#[tokio::test]
async fn tts_chunk_sent_after_skill_action() {
    use crate::grpc::proto::brain_response;

    let stt: Arc<dyn SpeechToText> = Arc::new(MockStt {
        text: "play music".to_string(),
        confidence: 0.95,
    });
    let tts: Arc<dyn TextToSpeech> = Arc::new(MockTts);

    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
        stt: Some(stt),
        trie: Arc::new(aether_core::CommandTrie::default()),
        llm: None,
        tts: Some(tts),
        skills: Arc::new(SkillRegistry::default()),
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(service))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    tokio::time::sleep(Duration::from_millis(25)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (tx, rx) = tokio::sync::mpsc::channel::<AudioChunk>(8);
    let mut client = AetherBrainClient::new(channel);
    let stream = ReceiverStream::new(rx);
    let mut req = tonic::Request::new(stream);
    req.metadata_mut()
        .insert("x-node-id", "tts-test-pi".parse().unwrap());

    let resp = client.audio_stream(req).await.unwrap();
    let mut resp_stream = resp.into_inner();

    for seq in 0u64..2 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 2048],
            seq,
        })
        .await
        .unwrap();
    }
    drop(tx);

    // Message 1: TranscriptUpdate.
    let first: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for TranscriptUpdate")
        .expect("stream error")
        .expect("stream closed before TranscriptUpdate");
    assert!(
        matches!(first.payload, Some(brain_response::Payload::Transcript(_))),
        "expected Transcript, got: {:?}",
        first.payload
    );

    // Message 2: SkillAction.
    let second: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for SkillAction")
        .expect("stream error")
        .expect("stream closed before SkillAction");
    assert!(
        matches!(second.payload, Some(brain_response::Payload::Action(_))),
        "expected Action, got: {:?}",
        second.payload
    );

    // Message 3: TtsChunk with valid WAV bytes.
    let third: BrainResponse = tokio::time::timeout(Duration::from_secs(5), resp_stream.message())
        .await
        .expect("timed out waiting for TtsChunk")
        .expect("stream error")
        .expect("stream closed before TtsChunk");

    match third.payload {
        Some(brain_response::Payload::TtsAudio(chunk)) => {
            assert!(chunk.wav.len() > 44, "WAV should be more than a header");
            assert_eq!(&chunk.wav[0..4], b"RIFF", "should be valid WAV");
        }
        other => panic!("expected TtsAudio, got: {other:?}"),
    }
}
