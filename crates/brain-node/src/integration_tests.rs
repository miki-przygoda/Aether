use crate::grpc::{
    proto::{
        aether_brain_client::AetherBrainClient, aether_brain_server::AetherBrainServer, AudioChunk,
    },
    BrainService,
};
use crate::session::SessionRegistry;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, Server, ServerTlsConfig};

/// Spin up a plain (no-TLS) BrainService on a random localhost port.
async fn start_plain_server() -> (std::net::SocketAddr, SessionRegistry) {
    let registry = SessionRegistry::new();
    let service = BrainService {
        registry: registry.clone(),
        certs_dir: std::path::PathBuf::from("/tmp"),
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
/// Each 10 ms sleep gives the tokio scheduler a chance to run cleanup tasks.
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

#[tokio::test]
async fn three_concurrent_audio_streams_register_and_cleanup() {
    let (addr, registry) = start_plain_server().await;

    // Senders keep the request half-streams open; dropping them triggers cleanup.
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
                // Drive the response stream to completion.
                while let Ok(Some(_)) = rs.message().await {}
            }
        });
    }

    // Phase 1 acceptance criterion: 3 distinct nodes, 3 distinct sessions.
    wait_for_count(&registry, 3, 2_000).await;
    assert_eq!(
        registry.count().await,
        3,
        "all three sessions should be active"
    );

    // Dropping request senders ends the request half-streams.
    // The server tasks then exit their while loops and call unregister().
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
    // Immediately close the stream so the RPC doesn't hang.
    drop(tx);

    let req = tonic::Request::new(stream);
    // No x-node-id header — server should reject with Unauthenticated.
    let err = client.audio_stream(req).await.unwrap_err();
    assert_eq!(
        err.code(),
        tonic::Code::Unauthenticated,
        "missing metadata should yield Unauthenticated"
    );
}

/// Full mTLS handshake: real CA → server cert (IP SAN 127.0.0.1) → client cert.
/// Verifies the cert chain produced by `pair` is accepted end-to-end by tonic/rustls
/// and that PCM chunks delivered over the authenticated stream are registered and cleaned up.
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

    // Wait for the session to appear before sending chunks.
    wait_for_count(&registry, 1, 1_000).await;

    // Send a few contiguous-sequence chunks — server should not log out-of-order warnings.
    for seq in 0u64..5 {
        tx.send(AudioChunk {
            pcm: vec![0u8; 32],
            seq,
        })
        .await
        .unwrap();
    }

    // Close the stream and wait for cleanup.
    drop(tx);
    wait_for_count(&registry, 0, 2_000).await;
}
