use aether_core::{NodeId, NodeState};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct Session {
    pub node_id: NodeId,
    pub state: NodeState,
}

#[derive(Clone)]
pub struct SessionRegistry {
    inner: Arc<RwLock<HashMap<NodeId, Session>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn register(&self, node_id: NodeId) {
        let mut sessions = self.inner.write().await;
        sessions.insert(
            node_id.clone(),
            Session {
                node_id,
                state: NodeState::Idle,
            },
        );
        tracing::debug!(sessions = sessions.len(), "session registered");
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
}
