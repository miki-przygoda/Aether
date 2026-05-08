use std::collections::HashMap;

/// Actions that can be dispatched directly without an LLM call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrieAction {
    PlayMusic,
    PauseMusic,
    StopMusic,
    SetTimer,
    LightsOn,
    LightsOff,
    Weather,
    VolumeUp,
    VolumeDown,
}

impl TrieAction {
    /// Snake-case string used as the `action` field in `SkillAction` / `LlmResponse`.
    pub fn as_str(&self) -> &'static str {
        match self {
            TrieAction::PlayMusic => "play_music",
            TrieAction::PauseMusic => "pause_music",
            TrieAction::StopMusic => "stop_music",
            TrieAction::SetTimer => "set_timer",
            TrieAction::LightsOn => "lights_on",
            TrieAction::LightsOff => "lights_off",
            TrieAction::Weather => "weather",
            TrieAction::VolumeUp => "volume_up",
            TrieAction::VolumeDown => "volume_down",
        }
    }
}

/// Result of classifying a transcript against the command trie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifyResult {
    /// A registered command phrase was found in the transcript.
    Match(TrieAction),
    /// The transcript ends on a valid trie prefix — more tokens may complete a command.
    Partial,
    /// No registered phrase matches and no phrase is in progress.
    NoMatch,
}

#[derive(Default)]
struct TrieNode {
    children: HashMap<String, TrieNode>,
    action: Option<TrieAction>,
}

/// Prefix tree over tokenised command phrases.
///
/// `classify` searches for any registered phrase within the transcript tokens
/// starting at every position, so commands embedded in longer utterances
/// ("can you play music please") are found correctly.
pub struct CommandTrie {
    root: TrieNode,
}

impl CommandTrie {
    fn insert(&mut self, phrase: &[&str], action: TrieAction) {
        let mut node = &mut self.root;
        for &word in phrase {
            node = node.children.entry(word.to_string()).or_default();
        }
        node.action = Some(action);
    }

    /// Classify `transcript` against the registered command set.
    ///
    /// Returns `Match` on the first complete phrase found, `Partial` if the
    /// transcript ends mid-phrase (useful for streaming evaluation), or
    /// `NoMatch` if no command is present or in progress.
    pub fn classify(&self, transcript: &str) -> ClassifyResult {
        let tokens = tokenize(transcript);
        if tokens.is_empty() {
            return ClassifyResult::NoMatch;
        }

        for start in 0..tokens.len() {
            if let Some(action) = self.try_match_at(&tokens[start..]) {
                return ClassifyResult::Match(action);
            }
        }

        for start in 0..tokens.len() {
            if self.is_partial_prefix(&tokens[start..]) {
                return ClassifyResult::Partial;
            }
        }

        ClassifyResult::NoMatch
    }

    /// Try to match a phrase starting at `tokens[0]`, following adjacent trie edges.
    /// Returns the action of the first terminal node reached.
    fn try_match_at(&self, tokens: &[String]) -> Option<TrieAction> {
        let mut node = &self.root;
        for token in tokens {
            match node.children.get(token.as_str()) {
                Some(child) => {
                    node = child;
                    if let Some(action) = &node.action {
                        return Some(action.clone());
                    }
                }
                None => return None,
            }
        }
        None
    }

    /// Returns `true` if every token in the slice follows a valid (non-terminal) trie path,
    /// meaning more tokens could complete a command.
    fn is_partial_prefix(&self, tokens: &[String]) -> bool {
        let mut node = &self.root;
        for token in tokens {
            match node.children.get(token.as_str()) {
                Some(child) => node = child,
                None => return false,
            }
        }
        !node.children.is_empty()
    }
}

impl Default for CommandTrie {
    fn default() -> Self {
        let mut t = Self {
            root: TrieNode::default(),
        };
        t.insert(&["play", "music"], TrieAction::PlayMusic);
        t.insert(&["start", "music"], TrieAction::PlayMusic);
        t.insert(&["pause", "music"], TrieAction::PauseMusic);
        t.insert(&["stop", "music"], TrieAction::StopMusic);
        t.insert(&["set", "timer"], TrieAction::SetTimer);
        t.insert(&["start", "timer"], TrieAction::SetTimer);
        t.insert(&["lights", "on"], TrieAction::LightsOn);
        t.insert(&["lights", "off"], TrieAction::LightsOff);
        t.insert(&["turn", "lights", "on"], TrieAction::LightsOn);
        t.insert(&["turn", "lights", "off"], TrieAction::LightsOff);
        t.insert(&["weather"], TrieAction::Weather);
        t.insert(&["volume", "up"], TrieAction::VolumeUp);
        t.insert(&["volume", "down"], TrieAction::VolumeDown);
        t
    }
}

/// Lowercase and split on non-alphabetic characters, dropping empty tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphabetic())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trie() -> CommandTrie {
        CommandTrie::default()
    }

    #[test]
    fn exact_command_matches() {
        assert_eq!(
            trie().classify("play music"),
            ClassifyResult::Match(TrieAction::PlayMusic)
        );
    }

    #[test]
    fn command_embedded_in_longer_phrase_matches() {
        assert_eq!(
            trie().classify("can you play music please"),
            ClassifyResult::Match(TrieAction::PlayMusic)
        );
    }

    #[test]
    fn case_insensitive_match() {
        assert_eq!(
            trie().classify("Play Music"),
            ClassifyResult::Match(TrieAction::PlayMusic)
        );
    }

    #[test]
    fn single_word_prefix_is_partial() {
        assert_eq!(trie().classify("play"), ClassifyResult::Partial);
    }

    #[test]
    fn multi_word_prefix_is_partial() {
        assert_eq!(trie().classify("turn lights"), ClassifyResult::Partial);
    }

    #[test]
    fn turn_prefix_alone_is_partial() {
        assert_eq!(trie().classify("turn"), ClassifyResult::Partial);
    }

    #[test]
    fn unrelated_phrase_is_no_match() {
        assert_eq!(trie().classify("hello world"), ClassifyResult::NoMatch);
    }

    #[test]
    fn empty_input_is_no_match() {
        assert_eq!(trie().classify(""), ClassifyResult::NoMatch);
    }

    #[test]
    fn lights_off_matches() {
        assert_eq!(
            trie().classify("lights off"),
            ClassifyResult::Match(TrieAction::LightsOff)
        );
    }

    #[test]
    fn turn_lights_on_matches() {
        assert_eq!(
            trie().classify("turn lights on"),
            ClassifyResult::Match(TrieAction::LightsOn)
        );
    }

    #[test]
    fn weather_matches_inside_question() {
        assert_eq!(
            trie().classify("what is the weather today"),
            ClassifyResult::Match(TrieAction::Weather)
        );
    }

    #[test]
    fn volume_up_matches() {
        assert_eq!(
            trie().classify("volume up"),
            ClassifyResult::Match(TrieAction::VolumeUp)
        );
    }

    #[test]
    fn volume_down_matches() {
        assert_eq!(
            trie().classify("volume down"),
            ClassifyResult::Match(TrieAction::VolumeDown)
        );
    }

    #[test]
    fn set_timer_matches() {
        assert_eq!(
            trie().classify("set timer for five minutes"),
            ClassifyResult::Match(TrieAction::SetTimer)
        );
    }

    #[test]
    fn action_as_str_is_snake_case() {
        assert_eq!(TrieAction::PlayMusic.as_str(), "play_music");
        assert_eq!(TrieAction::LightsOn.as_str(), "lights_on");
        assert_eq!(TrieAction::VolumeDown.as_str(), "volume_down");
    }

    #[test]
    fn punctuation_stripped_before_matching() {
        assert_eq!(
            trie().classify("play music, please!"),
            ClassifyResult::Match(TrieAction::PlayMusic)
        );
    }
}
