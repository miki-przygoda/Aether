use aether_core::SkillResult;
use std::collections::HashMap;
use std::sync::Arc;

// ─── Config ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SkillConfig {
    // Weather
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location_display_name: Option<String>,
    /// Override to point at a self-hosted Open-Meteo instance.
    pub weather_api_base: String,

    // Home Assistant
    pub home_assistant_url: Option<String>,
    pub home_assistant_token: Option<String>,

    // Volume (ALSA)
    pub alsa_control: String,
    pub volume_step_pct: u8,

    // Music (Navidrome / Subsonic)
    pub navidrome_url: Option<String>,  // e.g. "http://navidrome:4533"
    pub navidrome_user: Option<String>,
    pub navidrome_password: Option<String>,

    // Update checks
    pub update_check_enabled: bool,
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            latitude: None,
            longitude: None,
            location_display_name: None,
            weather_api_base: "https://api.open-meteo.com".into(),
            home_assistant_url: None,
            home_assistant_token: None,
            alsa_control: "Master".into(),
            volume_step_pct: 10,
            navidrome_url: Some("http://navidrome:4533".into()),
            navidrome_user: None,
            navidrome_password: None,
            update_check_enabled: true,
        }
    }
}

// ─── Context ─────────────────────────────────────────────────────────────────

/// Injected into every skill dispatch — provides I/O primitives without
/// coupling individual skills to AppState or BrainService directly.
pub struct SkillContext<'a> {
    pub node_id: &'a str,
    pub http_client: &'a reqwest::Client,
    pub config: &'a SkillConfig,
    pub registry: &'a crate::session::SessionRegistry,
}

// ─── Trait ───────────────────────────────────────────────────────────────────

/// A skill handles one named action and returns a spoken reply.
#[async_trait::async_trait]
pub trait Skill: Send + Sync {
    async fn handle(&self, params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult;
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
    pub async fn dispatch(
        &self,
        action: &str,
        params: &serde_json::Value,
        ctx: &SkillContext<'_>,
    ) -> SkillResult {
        self.skills
            .get(action)
            .unwrap_or(&self.fallback)
            .handle(params, ctx)
            .await
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
#[async_trait::async_trait]
impl Skill for PlayMusicSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: play_music");
        SkillResult {
            spoken_reply: "Music isn't set up yet — add files to the music folder and configure Navidrome in Settings.".into(),
        }
    }
}

struct PauseMusicSkill;
#[async_trait::async_trait]
impl Skill for PauseMusicSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: pause_music");
        SkillResult {
            spoken_reply: "Music paused.".into(),
        }
    }
}

struct StopMusicSkill;
#[async_trait::async_trait]
impl Skill for StopMusicSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: stop_music");
        SkillResult {
            spoken_reply: "Music stopped.".into(),
        }
    }
}

struct TimerSkill;
#[async_trait::async_trait]
impl Skill for TimerSkill {
    async fn handle(&self, params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        let secs = params["duration_seconds"].as_u64().unwrap_or(60);
        tracing::info!(secs, "skill: set_timer");
        SkillResult {
            spoken_reply: format!("Timer set for {} seconds.", secs),
        }
    }
}

struct LightsOnSkill;
#[async_trait::async_trait]
impl Skill for LightsOnSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: lights_on");
        SkillResult {
            spoken_reply: "Lights control needs Home Assistant set up in Settings.".into(),
        }
    }
}

struct LightsOffSkill;
#[async_trait::async_trait]
impl Skill for LightsOffSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: lights_off");
        SkillResult {
            spoken_reply: "Lights control needs Home Assistant set up in Settings.".into(),
        }
    }
}

struct WeatherSkill;
#[async_trait::async_trait]
impl Skill for WeatherSkill {
    async fn handle(&self, _params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: weather");
        let (lat, lon) = match (ctx.config.latitude, ctx.config.longitude) {
            (Some(lat), Some(lon)) => (lat, lon),
            _ => {
                return SkillResult {
                    spoken_reply: "Weather needs your location set in Skills Settings.".into(),
                };
            }
        };

        let url = format!(
            "{}/v1/forecast?latitude={lat}&longitude={lon}&current_weather=true&temperature_unit=celsius&wind_speed_unit=kmh",
            ctx.config.weather_api_base
        );

        let resp = match ctx.http_client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "weather fetch failed");
                return SkillResult {
                    spoken_reply: "Sorry, I couldn't fetch the weather right now.".into(),
                };
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "weather response parse failed");
                return SkillResult {
                    spoken_reply: "Sorry, the weather data came back in an unexpected format.".into(),
                };
            }
        };

        let cw = &json["current_weather"];
        let temp = cw["temperature"].as_f64().unwrap_or(0.0);
        let code = cw["weathercode"].as_u64().unwrap_or(0);
        let wind = cw["windspeed"].as_f64().unwrap_or(0.0);
        let condition = wmo_description(code);
        let location = ctx
            .config
            .location_display_name
            .as_deref()
            .unwrap_or("your location");

        SkillResult {
            spoken_reply: format!(
                "Currently in {location}: {condition}, {temp:.0}°C, wind {wind:.0} km/h."
            ),
        }
    }
}

/// WMO Weather Interpretation Code → human-readable English.
/// Source: <https://open-meteo.com/en/docs#weathervariables>
fn wmo_description(code: u64) -> &'static str {
    match code {
        0 => "clear sky",
        1 => "mainly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "foggy",
        51 => "light drizzle",
        53 => "moderate drizzle",
        55 => "heavy drizzle",
        61 => "light rain",
        63 => "moderate rain",
        65 => "heavy rain",
        71 => "light snow",
        73 => "moderate snow",
        75 => "heavy snow",
        77 => "snow grains",
        80 => "light showers",
        81 => "moderate showers",
        82 => "heavy showers",
        85 => "light snow showers",
        86 => "heavy snow showers",
        95 => "thunderstorm",
        96 | 99 => "thunderstorm with hail",
        _ => "unknown conditions",
    }
}

struct VolumeSkill {
    up: bool,
}
#[async_trait::async_trait]
impl Skill for VolumeSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
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

/// Used when the LLM returns action="respond" — reply text comes from the LLM response field.
struct RespondSkill;
#[async_trait::async_trait]
impl Skill for RespondSkill {
    async fn handle(&self, params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        let reply = params["response"].as_str().unwrap_or("Okay.").to_string();
        SkillResult {
            spoken_reply: reply,
        }
    }
}

struct UnknownSkill;
#[async_trait::async_trait]
impl Skill for UnknownSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
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

    fn make_ctx<'a>(
        client: &'a reqwest::Client,
        config: &'a SkillConfig,
        registry: &'a crate::session::SessionRegistry,
    ) -> SkillContext<'a> {
        SkillContext { node_id: "test", http_client: client, config, registry }
    }

    #[tokio::test]
    async fn known_action_is_dispatched() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("play_music", &json!({}), &ctx).await;
        assert!(!result.spoken_reply.is_empty());
    }

    #[tokio::test]
    async fn unknown_action_uses_fallback() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("teleport_me", &json!({}), &ctx).await;
        assert!(result.spoken_reply.contains("don't know"));
    }

    #[tokio::test]
    async fn timer_skill_uses_duration_param() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("set_timer", &json!({"duration_seconds": 120}), &ctx).await;
        assert!(result.spoken_reply.contains("120"));
    }

    #[tokio::test]
    async fn timer_skill_defaults_to_60s_when_param_missing() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("set_timer", &json!({}), &ctx).await;
        assert!(result.spoken_reply.contains("60"));
    }

    #[tokio::test]
    async fn volume_up_and_down_dispatched() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx_up = make_ctx(&client, &config, &registry);
        assert!(reg.dispatch("volume_up", &json!({}), &ctx_up).await.spoken_reply.contains("up"));
        let ctx_down = make_ctx(&client, &config, &registry);
        assert!(reg.dispatch("volume_down", &json!({}), &ctx_down).await.spoken_reply.contains("down"));
    }

    #[tokio::test]
    async fn weather_skill_prompts_for_location_when_unconfigured() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default(); // latitude/longitude both None
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("weather", &json!({}), &ctx).await;
        assert!(result.spoken_reply.contains("location"), "should prompt for location: {}", result.spoken_reply);
    }

    // ── Weather HTTP tests ────────────────────────────────────────────────────

    /// Spins up a tiny axum mock server that mimics Open-Meteo, checks that
    /// WeatherSkill formats the spoken reply correctly.
    #[tokio::test]
    async fn weather_skill_formats_reply_from_api() {
        use axum::{routing::get, Router};
        use tokio::net::TcpListener;

        let app = Router::new().route(
            "/v1/forecast",
            get(|| async {
                axum::Json(serde_json::json!({
                    "current_weather": {
                        "temperature": 15.0,
                        "windspeed": 12.0,
                        "winddirection": 180,
                        "weathercode": 2,
                        "is_day": 1,
                        "time": "2024-01-01T12:00"
                    }
                }))
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = reqwest::Client::new();
        let config = SkillConfig {
            latitude: Some(51.5),
            longitude: Some(-0.1),
            location_display_name: Some("London".into()),
            weather_api_base: format!("http://127.0.0.1:{port}"),
            ..Default::default()
        };
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);

        let result = WeatherSkill.handle(&json!({}), &ctx).await;
        assert!(
            result.spoken_reply.contains("London"),
            "missing location: {}",
            result.spoken_reply
        );
        assert!(
            result.spoken_reply.contains("15"),
            "missing temp: {}",
            result.spoken_reply
        );
        assert!(
            result.spoken_reply.contains("partly cloudy"),
            "missing condition: {}",
            result.spoken_reply
        );
    }

    /// When the HTTP call itself fails, WeatherSkill returns a graceful sorry.
    #[tokio::test]
    async fn weather_skill_handles_http_failure() {
        let client = reqwest::Client::new();
        let config = SkillConfig {
            latitude: Some(51.5),
            longitude: Some(-0.1),
            // Port 1 is reserved and nothing listens there — connection refused.
            weather_api_base: "http://127.0.0.1:1".into(),
            ..Default::default()
        };
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);

        let result = WeatherSkill.handle(&json!({}), &ctx).await;
        assert!(
            result.spoken_reply.to_lowercase().contains("sorry"),
            "expected apology: {}",
            result.spoken_reply
        );
    }

    /// When the server returns garbage JSON, WeatherSkill returns a graceful sorry.
    #[tokio::test]
    async fn weather_skill_handles_bad_json() {
        use axum::{routing::get, Router};
        use tokio::net::TcpListener;

        let app = Router::new().route(
            "/v1/forecast",
            get(|| async {
                (
                    axum::http::StatusCode::OK,
                    [("content-type", "application/json")],
                    "not-json-at-all",
                )
            }),
        );

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let client = reqwest::Client::new();
        let config = SkillConfig {
            latitude: Some(51.5),
            longitude: Some(-0.1),
            weather_api_base: format!("http://127.0.0.1:{port}"),
            ..Default::default()
        };
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);

        let result = WeatherSkill.handle(&json!({}), &ctx).await;
        assert!(
            result.spoken_reply.to_lowercase().contains("sorry"),
            "expected apology: {}",
            result.spoken_reply
        );
    }

    #[tokio::test]
    async fn all_registered_actions_return_non_empty_reply() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        for action in &[
            "play_music", "pause_music", "stop_music", "set_timer",
            "lights_on", "lights_off", "weather", "volume_up", "volume_down", "respond",
        ] {
            let ctx = make_ctx(&client, &config, &registry);
            let result = reg.dispatch(action, &json!({}), &ctx).await;
            assert!(!result.spoken_reply.is_empty(), "empty reply for {action}");
        }
    }
}
