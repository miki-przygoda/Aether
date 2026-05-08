use crate::brain_conn::{
    proto::{
        aether_brain_server::{AetherBrain, AetherBrainServer},
        AudioChunk, BrainResponse, PairRequest, PairResponse,
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

    async fn pair(
        &self,
        _: Request<PairRequest>,
    ) -> Result<Response<PairResponse>, Status> {
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

    tokio::spawn(async move {
        stream_audio(channel, "test-pi", pcm_rx).await.unwrap();
    });

    // Send three chunks of fake 16 kHz mono PCM (512 f32 samples each).
    for _ in 0..3 {
        pcm_tx.send(vec![0.0f32; 512]).await.unwrap();
    }
    drop(pcm_tx);

    let received = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        count_rx.recv(),
    )
    .await
    .expect("timed out — mock brain did not report chunk count within 2 s")
    .expect("count channel closed unexpectedly");

    assert_eq!(received, 3, "brain should receive all 3 PCM chunks");
}
