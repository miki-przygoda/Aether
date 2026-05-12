use aether_core::{NodeId, NodeState, NodeStateEvent};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, mpsc, RwLock};

#[derive(Debug, Clone)]
pub struct Session {
    pub node_id: NodeId,
    pub state: NodeState,
}

#[derive(Clone)]
pub struct SessionRegistry {
    inner: Arc<RwLock<HashMap<NodeId, Session>>>,
    /// Published on every state transition — subscribe for real-time state mirroring.
    pub event_tx: broadcast::Sender<NodeStateEvent>,
    /// Per-session push channels for out-of-band model updates (wake word hot-reload).
    push_txs: Arc<RwLock<HashMap<NodeId, mpsc::Sender<Vec<u8>>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            push_txs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a push channel for `node_id`. Called by the gRPC handler after
    /// a session opens; the sender is used to deliver wake word model updates.
    pub async fn register_push(&self, node_id: NodeId, tx: mpsc::Sender<Vec<u8>>) {
        self.push_txs.write().await.insert(node_id, tx);
    }

    /// Remove the push channel when a session ends.
    pub async fn unregister_push(&self, node_id: &str) {
        self.push_txs.write().await.remove(node_id);
    }

    /// Push raw model bytes to the specified nodes (or all nodes if `node_ids`
    /// is empty). Returns the number of nodes that were actually online and
    /// whose channel accepted the message.
    pub async fn push_wake_word_model(&self, node_ids: &[String], model_bytes: Vec<u8>) -> usize {
        let txs = self.push_txs.read().await;
        let mut pushed = 0usize;
        for (id, tx) in txs.iter() {
            if node_ids.is_empty() || node_ids.contains(id) {
                if tx.send(model_bytes.clone()).await.is_ok() {
                    pushed += 1;
                } else {
                    tracing::warn!(node_id = %id, "model push channel closed — node may have disconnected");
                }
            }
        }
        pushed
    }

    /// Subscribe to state change events. Lagged receivers get
    /// `RecvError::Lagged` and should re-read current state.
    /// Used by auxiliary nodes and the Phase 5 web UI dashboard.
    #[allow(dead_code)]
    pub fn subscribe(&self) -> broadcast::Receiver<NodeStateEvent> {
        self.event_tx.subscribe()
    }

    pub async fn register(&self, node_id: NodeId) {
        let mut sessions = self.inner.write().await;
        sessions.insert(
            node_id.clone(),
            Session {
                node_id: node_id.clone(),
                state: NodeState::Idle,
            },
        );
        tracing::debug!(sessions = sessions.len(), "session registered");
        let _ = self.event_tx.send(NodeStateEvent {
            node_id,
            state: NodeState::Idle,
        });
    }

    pub async fn unregister(&self, node_id: &str) {
        let mut sessions = self.inner.write().await;
        if let Some(s) = sessions.remove(node_id) {
            tracing::debug!(node_id = %s.node_id, sessions = sessions.len(), "session unregistered");
        }
    }

    pub async fn set_state(&self, node_id: &str, state: NodeState) {
        if let Some(s) = self.inner.write().await.get_mut(node_id) {
            s.state = state;
            let _ = self.event_tx.send(NodeStateEvent {
                node_id: node_id.to_string(),
                state,
            });
        }
    }

    /// Return a point-in-time snapshot of all active sessions (used by web UI dashboard).
    pub async fn snapshot(&self) -> Vec<Session> {
        self.inner.read().await.values().cloned().collect()
    }

    #[cfg(test)]
    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_unregister() {
        let reg = SessionRegistry::new();
        reg.register("pi-1".into()).await;
        reg.register("pi-2".into()).await;
        assert_eq!(reg.inner.read().await.len(), 2);

        reg.unregister("pi-1").await;
        assert_eq!(reg.inner.read().await.len(), 1);
    }

    #[tokio::test]
    async fn state_transitions() {
        let reg = SessionRegistry::new();
        reg.register("pi-1".into()).await;
        reg.set_state("pi-1", NodeState::Processing).await;

        let sessions = reg.inner.read().await;
        let s = &sessions["pi-1"];
        assert_eq!(s.node_id, "pi-1");
        assert_eq!(s.state, NodeState::Processing);
    }

    #[tokio::test]
    async fn broadcast_emitted_on_register() {
        let reg = SessionRegistry::new();
        let mut rx = reg.subscribe();
        reg.register("pi-1".into()).await;

        let event = rx.try_recv().expect("event should be buffered");
        assert_eq!(event.node_id, "pi-1");
        assert_eq!(event.state, NodeState::Idle);
    }

    #[tokio::test]
    async fn broadcast_emitted_on_set_state() {
        let reg = SessionRegistry::new();
        reg.register("pi-1".into()).await;
        let mut rx = reg.subscribe();

        reg.set_state("pi-1", NodeState::Processing).await;

        let event = rx
            .try_recv()
            .expect("state-change event should be buffered");
        assert_eq!(event.node_id, "pi-1");
        assert_eq!(event.state, NodeState::Processing);
    }

    #[tokio::test]
    async fn multiple_subscribers_all_receive() {
        let reg = SessionRegistry::new();
        let mut rx1 = reg.subscribe();
        let mut rx2 = reg.subscribe();
        let mut rx3 = reg.subscribe();

        reg.register("pi-1".into()).await;
        reg.set_state("pi-1", NodeState::Processing).await;

        for rx in [&mut rx1, &mut rx2, &mut rx3] {
            let e = rx.try_recv().unwrap();
            assert_eq!(e.state, NodeState::Idle); // register event
            let e = rx.try_recv().unwrap();
            assert_eq!(e.state, NodeState::Processing); // set_state event
        }
    }
}
