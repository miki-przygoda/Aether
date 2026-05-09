use aether_core::{NodeId, NodeState, NodeStateEvent};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{broadcast, RwLock};

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
}

impl SessionRegistry {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
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
