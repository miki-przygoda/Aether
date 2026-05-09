use aether_core::SkillResult;
use std::collections::HashMap;
use std::sync::Arc;

/// A skill handles one named action and returns a spoken reply.
pub trait Skill: Send + Sync {
    fn handle(&self, params: &serde_json::Value) -> SkillResult;
}

// ─── Registry ────────────────────────────────────────────────────────────────

pub struct SkillRegistry {
    skills: HashMap<String, Arc<dyn Skill>>,
    fallback: Arc<dyn Skill>,
}

/// Information about a registered skill, used by the web UI skills page.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillInfo {
    pub action: String,
    pub description: String,
    pub example_phrases: Vec<String>,
}

impl SkillRegistry {
    pub fn dispatch(&self, action: &str, params: &serde_json::Value) -> SkillResult {
        self.skills
            .get(action)
            .unwrap_or(&self.fallback)
            .handle(params)
    }

    /// Return metadata for all registered skills, sorted by action name.
    /// Used by the web UI skills page and the skill-tester API.
    pub fn list(&self) -> Vec<SkillInfo> {
        static DESCRIPTIONS: &[(&str, &str, &[&str])] = &[
            (
                "lights_off",
                "Turn off the lights",
                &["lights off", "turn off the lights"],
            ),
            (
                "lights_on",
                "Turn on the lights",
                &["lights on", "turn on the lights"],
            ),
            (
                "pause_music",
                "Pause music playback",
                &["pause", "pause music"],
            ),
            (
                "play_music",
                "Start music playback",
                &["play music", "play something"],
            ),
            (
                "respond",
                "General conversation / fallback",
                &["what time is it?", "tell me a joke"],
            ),
            (
                "set_timer",
                "Set a countdown timer",
                &["set a timer for 5 minutes", "timer 30 seconds"],
            ),
            ("stop_music", "Stop music playback", &["stop music", "stop"]),
            (
                "volume_down",
                "Decrease the volume",
                &["volume down", "quieter"],
            ),
            ("volume_up", "Increase the volume", &["volume up", "louder"]),
            (
                "weather",
                "Report the current weather",
                &["what's the weather?", "is it raining?"],
            ),
        ];
        let mut infos: Vec<SkillInfo> = self
            .skills
            .keys()
            .map(|action| {
                let (desc, phrases) = DESCRIPTIONS
                    .iter()
                    .find(|(a, _, _)| *a == action.as_str())
                    .map(|(_, d, p)| (*d, p.iter().map(|s| s.to_string()).collect::<Vec<_>>()))
                    .unwrap_or(("No description", vec![]));
                SkillInfo {
                    action: action.clone(),
                    description: desc.to_string(),
                    example_phrases: phrases,
                }
            })
            .collect();
        infos.sort_by(|a, b| a.action.cmp(&b.action));
        infos
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        let mut skills: HashMap<String, Arc<dyn Skill>> = HashMap::new();
        skills.insert("play_music".into(), Arc::new(PlayMusicSkill));
        skills.insert("pause_music".into(), Arc::new(PauseMusicSkill));
        skills.insert("stop_music".into(), Arc::new(StopMusicSkill));
        skills.insert("set_timer".into(), Arc::new(TimerSkill));
        skills.insert("lights_on".into(), Arc::new(LightsOnSkill));
        skills.insert("lights_off".into(), Arc::new(LightsOffSkill));
        skills.insert("weather".into(), Arc::new(WeatherSkill));
        skills.insert("volume_up".into(), Arc::new(VolumeSkill { up: true }));
        skills.insert("volume_down".into(), Arc::new(VolumeSkill { up: false }));
        skills.insert("respond".into(), Arc::new(RespondSkill));
        Self {
            skills,
            fallback: Arc::new(UnknownSkill),
        }
    }
}

// ─── Stub skills ─────────────────────────────────────────────────────────────

struct PlayMusicSkill;
impl Skill for PlayMusicSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::info!("skill: play_music");
        SkillResult {
            spoken_reply: "Playing music.".into(),
        }
    }
}

struct PauseMusicSkill;
impl Skill for PauseMusicSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::info!("skill: pause_music");
        SkillResult {
            spoken_reply: "Music paused.".into(),
        }
    }
}

struct StopMusicSkill;
impl Skill for StopMusicSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::info!("skill: stop_music");
        SkillResult {
            spoken_reply: "Music stopped.".into(),
        }
    }
}

struct TimerSkill;
impl Skill for TimerSkill {
    fn handle(&self, params: &serde_json::Value) -> SkillResult {
        let secs = params["duration_seconds"].as_u64().unwrap_or(60);
        tracing::info!(secs, "skill: set_timer");
        SkillResult {
            spoken_reply: format!("Timer set for {} seconds.", secs),
        }
    }
}

struct LightsOnSkill;
impl Skill for LightsOnSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::info!("skill: lights_on");
        SkillResult {
            spoken_reply: "Lights on.".into(),
        }
    }
}

struct LightsOffSkill;
impl Skill for LightsOffSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::info!("skill: lights_off");
        SkillResult {
            spoken_reply: "Lights off.".into(),
        }
    }
}

struct WeatherSkill;
impl Skill for WeatherSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::info!("skill: weather");
        SkillResult {
            spoken_reply: "Sorry, I don't have access to weather data yet.".into(),
        }
    }
}

struct VolumeSkill {
    up: bool,
}
impl Skill for VolumeSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        if self.up {
            tracing::info!("skill: volume_up");
            SkillResult {
                spoken_reply: "Volume up.".into(),
            }
        } else {
            tracing::info!("skill: volume_down");
            SkillResult {
                spoken_reply: "Volume down.".into(),
            }
        }
    }
}

/// Used when the LLM returns action="respond" — reply text comes from the LLM response field,
/// so this skill is only reached via trie dispatch; it just echoes the params response field.
struct RespondSkill;
impl Skill for RespondSkill {
    fn handle(&self, params: &serde_json::Value) -> SkillResult {
        let reply = params["response"].as_str().unwrap_or("Okay.").to_string();
        SkillResult {
            spoken_reply: reply,
        }
    }
}

struct UnknownSkill;
impl Skill for UnknownSkill {
    fn handle(&self, _params: &serde_json::Value) -> SkillResult {
        tracing::warn!("skill: unknown action — no handler registered");
        SkillResult {
            spoken_reply: "Sorry, I don't know how to do that yet.".into(),
        }
    }
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_action_is_dispatched() {
        let reg = SkillRegistry::default();
        let result = reg.dispatch("play_music", &json!({}));
        assert_eq!(result.spoken_reply, "Playing music.");
    }

    #[test]
    fn unknown_action_uses_fallback() {
        let reg = SkillRegistry::default();
        let result = reg.dispatch("teleport_me", &json!({}));
        assert!(result.spoken_reply.contains("don't know"));
    }

    #[test]
    fn timer_skill_uses_duration_param() {
        let reg = SkillRegistry::default();
        let result = reg.dispatch("set_timer", &json!({"duration_seconds": 120}));
        assert!(result.spoken_reply.contains("120"));
    }

    #[test]
    fn timer_skill_defaults_to_60s_when_param_missing() {
        let reg = SkillRegistry::default();
        let result = reg.dispatch("set_timer", &json!({}));
        assert!(result.spoken_reply.contains("60"));
    }

    #[test]
    fn volume_up_and_down_dispatched() {
        let reg = SkillRegistry::default();
        assert!(reg
            .dispatch("volume_up", &json!({}))
            .spoken_reply
            .contains("up"));
        assert!(reg
            .dispatch("volume_down", &json!({}))
            .spoken_reply
            .contains("down"));
    }

    #[test]
    fn all_registered_actions_return_non_empty_reply() {
        let reg = SkillRegistry::default();
        for action in &[
            "play_music",
            "pause_music",
            "stop_music",
            "set_timer",
            "lights_on",
            "lights_off",
            "weather",
            "volume_up",
            "volume_down",
            "respond",
        ] {
            let result = reg.dispatch(action, &json!({}));
            assert!(!result.spoken_reply.is_empty(), "empty reply for {action}");
        }
    }
}
