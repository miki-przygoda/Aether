/// Lightweight HTTP SSE server that streams the edge node's `NodeState` to
/// auxiliary nodes.
///
/// Auxiliary nodes connect to `GET /state/events` and mirror their LED to the
/// primary node's state.  Each SSE event is a JSON-serialised `NodeState`.
///
/// Usage:
/// ```no_run
/// let (state_tx, _) = tokio::sync::broadcast::channel(16);
/// tokio::spawn(serve(3000, state_tx.clone()));
/// // Publish state changes:
/// let _ = state_tx.send(aether_core::NodeState::Processing);
/// ```
use aether_core::NodeState;
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::get,
    Router,
};
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone)]
struct AppState {
    state_tx: broadcast::Sender<NodeState>,
}

pub async fn serve(port: u16, state_tx: broadcast::Sender<NodeState>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/state/events", get(sse_handler))
        .with_state(AppState { state_tx });

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(%addr, "state SSE server listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn sse_handler(State(app): State<AppState>) -> impl IntoResponse {
    let rx = app.state_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().map(|state| {
            let data = serde_json::to_string(&state).unwrap_or_else(|_| "\"Idle\"".to_string());
            Ok::<Event, Infallible>(Event::default().data(data))
        })
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── Auxiliary mode SSE client ─────────────────────────────────────────────────

/// Connect to a primary edge node's SSE endpoint and yield `NodeState` values
/// as they arrive.  Runs until the connection is dropped or an error occurs.
///
/// `target` is the base URL of the primary node, e.g. `"http://192.168.1.50:3000"`.
pub async fn subscribe_to_primary(
    target: &str,
    mut on_state: impl FnMut(NodeState) + Send + 'static,
) -> anyhow::Result<()> {
    let url = format!("{target}/state/events");
    tracing::info!(%url, "connecting to primary node SSE");

    let response = reqwest::get(&url).await?;
    let mut stream = response.bytes_stream();

    let mut buf = String::new();

    use tokio_stream::StreamExt as _;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // SSE events are separated by blank lines.
        while let Some(pos) = buf.find("\n\n") {
            let event_block = buf[..pos].to_string();
            buf.drain(..pos + 2);

            for line in event_block.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(state) = serde_json::from_str::<NodeState>(data) {
                        on_state(state);
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_data_round_trips_through_json() {
        let state = NodeState::Processing;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"Processing\"");
        let decoded: NodeState = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, NodeState::Processing);
    }

    #[test]
    fn all_states_serialize() {
        for state in [
            NodeState::Idle,
            NodeState::Listening,
            NodeState::Processing,
            NodeState::Error,
        ] {
            let json = serde_json::to_string(&state).unwrap();
            let back: NodeState = serde_json::from_str(&json).unwrap();
            assert_eq!(back, state);
        }
    }
}
