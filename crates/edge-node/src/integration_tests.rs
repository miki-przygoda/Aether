use crate::brain_conn::{
    proto::{
        aether_brain_server::{AetherBrain, AetherBrainServer},
        brain_response, AudioChunk, BrainResponse, PairRequest, PairResponse, TtsChunk,
    },
    stream_audio,
};
use aether_core::wake_word::WakeWordDetector;
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::{transport::Server, Request, Response, Status, Streaming};

// ─── mock wake word detector ──────────────────────────────────────────────────

struct AlwaysTrigger;

impl WakeWordDetector for AlwaysTrigger {
    fn process_samples(&mut self, _: &[f32]) -> bool {
        true
    }
}

// ─── mock brain server ────────────────────────────────────────────────────────

/// Counts received PCM chunks and reports the total once the stream closes.
struct MockBrain {
    count_tx: mpsc::Sender<usize>,
}

#[tonic::async_trait]
impl AetherBrain for MockBrain {
    type AudioStreamStream = ReceiverStream<Result<BrainResponse, Status>>;

    async fn audio_stream(
        &self,
        request: Request<Streaming<AudioChunk>>,
    ) -> Result<Response<Self::AudioStreamStream>, Status> {
        let count_tx = self.count_tx.clone();
        let mut stream = request.into_inner();
        // Keep the response sender alive in the task so stream_audio waits for
        // the count to be reported before the response stream closes.
        let (resp_tx, resp_rx) = mpsc::channel::<Result<BrainResponse, Status>>(1);

        tokio::spawn(async move {
            let mut count = 0;
            while let Ok(Some(_)) = stream.message().await {
                count += 1;
            }
            let _ = count_tx.send(count).await;
            drop(resp_tx);
        });

        Ok(Response::new(ReceiverStream::new(resp_rx)))
    }

    async fn pair(&self, _: Request<PairRequest>) -> Result<Response<PairResponse>, Status> {
        Err(Status::unimplemented("not used in this test"))
    }
}

// ─── tests ────────────────────────────────────────────────────────────────────

/// Verifies the full edge-node wake-word → stream → PCM delivery path without
/// real audio or a trained model.
///
/// AlwaysTrigger fires on the first sample batch (mimicking a real wake word
/// detection). stream_audio then opens a plain-gRPC connection to a mock brain
/// and forwards PCM chunks. The mock brain counts chunks and reports the total
/// once the stream closes — both ends must agree.
#[tokio::test]
async fn wake_word_trigger_opens_stream_and_delivers_pcm() {
    let (count_tx, mut count_rx) = mpsc::channel::<usize>(1);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(MockBrain { count_tx }))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );

    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    // Wake word fires immediately — mirrors the `if detector.process_samples(...)` branch
    // in main.rs::run() that triggers a stream open.
    let mut detector: Box<dyn WakeWordDetector> = Box::new(AlwaysTrigger);
    assert!(detector.process_samples(&[0.0f32; 512]));

    // On trigger: build channel and open the audio stream (mirrors main.rs::run()).
    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<f32>>(16);

    let (dummy_reload_tx, _) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let music_handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        stream_audio(channel, "test-pi", pcm_rx, dummy_reload_tx, music_handle)
            .await
            .unwrap();
    });

    // Send three chunks of fake 16 kHz mono PCM (512 f32 samples each).
    for _ in 0..3 {
        pcm_tx.send(vec![0.0f32; 512]).await.unwrap();
    }
    drop(pcm_tx);

    let received = tokio::time::timeout(std::time::Duration::from_secs(2), count_rx.recv())
        .await
        .expect("timed out — mock brain did not report chunk count within 2 s")
        .expect("count channel closed unexpectedly");

    assert_eq!(received, 3, "brain should receive all 3 PCM chunks");
}

// ─── TTS dispatch test ────────────────────────────────────────────────────────

/// Mock brain that sends a TtsChunk back as soon as the stream closes.
struct MockBrainWithTts {
    ready_tx: mpsc::Sender<()>,
}

#[tonic::async_trait]
impl AetherBrain for MockBrainWithTts {
    type AudioStreamStream = ReceiverStream<Result<BrainResponse, Status>>;

    async fn audio_stream(
        &self,
        request: Request<Streaming<AudioChunk>>,
    ) -> Result<Response<Self::AudioStreamStream>, Status> {
        let ready_tx = self.ready_tx.clone();
        let mut stream = request.into_inner();
        let (resp_tx, resp_rx) = mpsc::channel::<Result<BrainResponse, Status>>(4);

        tokio::spawn(async move {
            // Drain the PCM stream.
            while let Ok(Some(_)) = stream.message().await {}

            // Send a minimal TtsChunk (silence, so play_wav can parse the WAV
            // header even if no audio device is available).
            let wav = crate::playback::tests::make_wav_silence(100, 24_000);
            let _ = resp_tx
                .send(Ok(BrainResponse {
                    payload: Some(brain_response::Payload::TtsAudio(TtsChunk { wav })),
                }))
                .await;

            // Signal that the chunk was sent, then close the stream.
            let _ = ready_tx.send(()).await;
            drop(resp_tx);
        });

        Ok(Response::new(ReceiverStream::new(resp_rx)))
    }

    async fn pair(&self, _: Request<PairRequest>) -> Result<Response<PairResponse>, Status> {
        Err(Status::unimplemented("not used in this test"))
    }
}

/// Verifies that stream_audio correctly receives a TtsChunk and invokes playback
/// without panicking — even when no audio device is available (CI headless).
#[tokio::test]
async fn tts_chunk_received_and_dispatched() {
    let (ready_tx, mut ready_rx) = mpsc::channel::<()>(1);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(
        Server::builder()
            .add_service(AetherBrainServer::new(MockBrainWithTts { ready_tx }))
            .serve_with_incoming(TcpListenerStream::new(listener)),
    );
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();

    let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<f32>>(4);

    let (dummy_reload_tx, _) = tokio::sync::mpsc::channel::<()>(1);
    let stream_result = tokio::spawn(async move {
        let music_handle = std::sync::Arc::new(std::sync::Mutex::new(None));
        stream_audio(
            channel,
            "tts-test-pi",
            pcm_rx,
            dummy_reload_tx,
            music_handle,
        )
        .await
    });

    // Send one chunk then close.
    pcm_tx.send(vec![0.0f32; 512]).await.unwrap();
    drop(pcm_tx);

    // Wait for the server to confirm the TtsChunk was sent.
    tokio::time::timeout(std::time::Duration::from_secs(5), ready_rx.recv())
        .await
        .expect("timed out waiting for TTS chunk to be sent")
        .expect("ready channel closed");

    // stream_audio must complete without error (playback error is logged, not propagated).
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), stream_result)
        .await
        .expect("stream_audio timed out")
        .expect("task panicked");

    assert!(
        result.is_ok(),
        "stream_audio should not propagate TTS errors: {result:?}"
    );
}
