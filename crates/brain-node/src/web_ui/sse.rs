use super::AppState;
use axum::{
    extract::State,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
};
use futures::stream::Stream;
use std::convert::Infallible;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

pub async fn nodes_handler(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.registry.event_tx.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|res| {
            res.ok().and_then(|ev| {
                serde_json::to_string(&serde_json::json!({
                    "node_id": ev.node_id,
                    "state": format!("{:?}", ev.state),
                }))
                .ok()
                .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
            })
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn wake_training_handler(State(state): State<AppState>) -> impl IntoResponse {
    make_progress_sse(state.wake_progress_tx.subscribe())
}

pub async fn voice_training_handler(State(state): State<AppState>) -> impl IntoResponse {
    make_progress_sse(state.voice_progress_tx.subscribe())
}

pub async fn ingest_progress_handler(State(state): State<AppState>) -> impl IntoResponse {
    make_progress_sse(state.ingest_progress_tx.subscribe())
}

fn make_progress_sse(
    rx: tokio::sync::broadcast::Receiver<super::ProgressEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(rx)
        .filter_map(|res| {
            res.ok().and_then(|ev| {
                serde_json::to_string(&ev)
                    .ok()
                    .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
            })
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
