pub mod trie;
pub mod types;
pub mod wake_word;

pub use trie::{ClassifyResult, CommandTrie, TrieAction};
pub use types::{LlmResponse, NodeState, NodeStateEvent, SkillResult, TtsSettings};

/// Stable identifier for an edge node, set during pairing.
pub type NodeId = String;
