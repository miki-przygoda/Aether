pub mod types;
pub mod wake_word;

pub use types::{LlmResponse, NodeState};

/// Stable identifier for an edge node, set during pairing.
pub type NodeId = String;
