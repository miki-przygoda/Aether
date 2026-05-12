use aether_core::{MusicCommandResult, SkillResult};
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
    /// Default HA entity for "lights on/off" without a room param, e.g. "light.living_room".
    pub ha_entity_id: Option<String>,
    /// Room-name → HA entity ID overrides, e.g. {"bedroom": "light.bedroom_main"}.
    pub ha_room_map: HashMap<String, String>,

    // Volume (ALSA)
    pub alsa_control: String,
    pub volume_step_pct: u8,

    // Music (Navidrome / Subsonic)
    pub navidrome_url: Option<String>, // e.g. "http://navidrome:4533"
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
            ha_entity_id: None,
            ha_room_map: HashMap::new(),
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
    /// LAN-accessible IP of the brain machine; used to construct external URLs
    /// (e.g. Navidrome stream URLs) the Pi can connect to.
    pub brain_ip: &'a str,
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
        let shell: Arc<dyn ShellExecutor> = Arc::new(RealShell);
        skills.insert(
            "volume_up".into(),
            Arc::new(VolumeSkill {
                up: true,
                shell: shell.clone(),
            }),
        );
        skills.insert(
            "volume_down".into(),
            Arc::new(VolumeSkill { up: false, shell }),
        );
        skills.insert("respond".into(), Arc::new(RespondSkill));
        Self {
            skills,
            fallback: Arc::new(UnknownSkill),
        }
    }
}

// ─── Music ───────────────────────────────────────────────────────────────────

struct PlayMusicSkill;
#[async_trait::async_trait]
impl Skill for PlayMusicSkill {
    async fn handle(&self, params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: play_music");
        let (url, user, pass) =
            match (
                ctx.config.navidrome_url.as_deref(),
                ctx.config.navidrome_user.as_deref(),
                ctx.config.navidrome_password.as_deref(),
            ) {
                (Some(u), Some(user), Some(pass)) => {
                    (u.to_string(), user.to_string(), pass.to_string())
                }
                _ => return SkillResult::speak(
                    "Music isn't set up yet — configure Navidrome credentials in Skills Settings.",
                ),
            };

        let client =
            crate::navidrome::NavidromeClient::new(url, user, pass, ctx.http_client.clone());

        let songs = if let Some(q) = params["query"].as_str().filter(|s| !s.is_empty()) {
            client.search_songs(q).await
        } else {
            client.get_random_songs(1).await
        };

        let songs = match songs {
            Ok(s) if !s.is_empty() => s,
            Ok(_) => return SkillResult::speak("No songs found in your library."),
            Err(e) => {
                tracing::error!(error = %e, "Navidrome query failed");
                return SkillResult::speak("Couldn't reach Navidrome — check it's running.");
            }
        };

        let song = &songs[0];

        // External URL: replace Docker-internal hostname with brain's LAN IP.
        let navidrome_port = ctx
            .config
            .navidrome_url
            .as_deref()
            .and_then(|u| u.rsplit(':').next())
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(4533);
        let external_base = format!("http://{}:{}", ctx.brain_ip, navidrome_port);
        let stream_url = client.stream_url(&song.id, &external_base);

        tracing::info!(
            title = %song.title,
            artist = %song.artist,
            "dispatching MusicCommand::play"
        );
        SkillResult {
            spoken_reply: format!("Playing {} by {}.", song.title, song.artist),
            music_command: Some(MusicCommandResult {
                action: "play".into(),
                stream_url,
                title: song.title.clone(),
                artist: song.artist.clone(),
            }),
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
            music_command: Some(MusicCommandResult {
                action: "pause".into(),
                stream_url: String::new(),
                title: String::new(),
                artist: String::new(),
            }),
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
            music_command: Some(MusicCommandResult {
                action: "stop".into(),
                stream_url: String::new(),
                title: String::new(),
                artist: String::new(),
            }),
        }
    }
}

struct TimerSkill;
#[async_trait::async_trait]
impl Skill for TimerSkill {
    async fn handle(&self, params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult {
        let secs = params["duration_seconds"].as_u64().unwrap_or(60);
        let label = friendly_duration(secs);
        let node_id = ctx.node_id.to_string();
        let registry = ctx.registry.clone();
        let label_spawn = label.clone();

        tracing::info!(secs, label = %label, "skill: set_timer — spawning callback");

        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(secs)).await;
            tracing::info!(node_id = %node_id, label = %label_spawn, "timer fired");
            registry
                .enqueue_tts(&node_id, format!("Your {label_spawn} timer is up."))
                .await;
        });

        SkillResult::speak(format!("Timer set for {label}."))
    }
}

/// Convert a duration in seconds into a natural English phrase.
///
/// Rules:
/// - Only the two most significant non-zero units are spoken.
/// - Seconds are dropped when hours are present (irrelevant at that scale).
///
/// Examples: 30 → "30 seconds", 90 → "1 minute and 30 seconds",
///           3660 → "1 hour and 1 minute", 7200 → "2 hours".
fn friendly_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    fn p(n: u64, unit: &str) -> String {
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    }

    match (hours, mins, secs) {
        (0, 0, s) => p(s, "second"),
        (0, m, 0) => p(m, "minute"),
        (0, m, s) => format!("{} and {}", p(m, "minute"), p(s, "second")),
        (h, 0, _) => p(h, "hour"),
        (h, m, _) => format!("{} and {}", p(h, "hour"), p(m, "minute")),
    }
}

// ─── Lights (Home Assistant REST) ────────────────────────────────────────────

struct LightsOnSkill;
#[async_trait::async_trait]
impl Skill for LightsOnSkill {
    async fn handle(&self, params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: lights_on");
        call_ha_lights(true, params, ctx).await
    }
}

struct LightsOffSkill;
#[async_trait::async_trait]
impl Skill for LightsOffSkill {
    async fn handle(&self, params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult {
        tracing::info!("skill: lights_off");
        call_ha_lights(false, params, ctx).await
    }
}

async fn call_ha_lights(
    on: bool,
    params: &serde_json::Value,
    ctx: &SkillContext<'_>,
) -> SkillResult {
    let (ha_url, token) = match (
        ctx.config.home_assistant_url.as_deref(),
        ctx.config.home_assistant_token.as_deref(),
    ) {
        (Some(u), Some(t)) => (u, t),
        _ => {
            return SkillResult::speak(
                "Home Assistant isn't configured yet — set it up in Skills Settings.",
            )
        }
    };

    // Determine which entity to target:
    // 1. Room param from LLM → normalise to snake_case → look up room_map or construct entity id
    // 2. Fall back to ha_entity_id config value
    let entity_id = if let Some(room) = params["room"].as_str().filter(|s| !s.is_empty()) {
        let normalized = room.to_lowercase().replace(' ', "_");
        ctx.config
            .ha_room_map
            .get(&normalized)
            .cloned()
            .unwrap_or_else(|| format!("light.{normalized}"))
    } else {
        match &ctx.config.ha_entity_id {
            Some(e) => e.clone(),
            None => {
                return SkillResult::speak(
                    "Please configure a default light entity in Skills Settings.",
                )
            }
        }
    };

    let service = if on { "turn_on" } else { "turn_off" };
    let endpoint = format!("{ha_url}/api/services/light/{service}");

    let result = ctx
        .http_client
        .post(&endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({ "entity_id": entity_id }))
        .send()
        .await;

    match result {
        Ok(r) if r.status().is_success() => {
            let word = if on { "on" } else { "off" };
            SkillResult::speak(format!("Lights {word}."))
        }
        Ok(r) => {
            tracing::warn!(status = %r.status(), entity = %entity_id, "HA returned error");
            SkillResult::speak(
                "Home Assistant returned an error — check the entity ID in Settings.",
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "HA request failed");
            SkillResult::speak("Couldn't reach Home Assistant.")
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
            _ => return SkillResult::speak("Weather needs your location set in Skills Settings."),
        };

        let url = format!(
            "{}/v1/forecast?latitude={lat}&longitude={lon}&current_weather=true&temperature_unit=celsius&wind_speed_unit=kmh",
            ctx.config.weather_api_base
        );

        let resp = match ctx.http_client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "weather fetch failed");
                return SkillResult::speak("Sorry, I couldn't fetch the weather right now.");
            }
        };

        let json: serde_json::Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                tracing::error!(error = %e, "weather response parse failed");
                return SkillResult::speak(
                    "Sorry, the weather data came back in an unexpected format.",
                );
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

        SkillResult::speak(format!(
            "Currently in {location}: {condition}, {temp:.0}°C, wind {wind:.0} km/h."
        ))
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

// ─── Shell executor ───────────────────────────────────────────────────────────

/// Thin abstraction over running a subprocess — swap for `MockShell` in tests.
#[async_trait::async_trait]
pub trait ShellExecutor: Send + Sync {
    async fn run(&self, cmd: &str, args: &[&str]) -> anyhow::Result<String>;
}

pub struct RealShell;

#[async_trait::async_trait]
impl ShellExecutor for RealShell {
    async fn run(&self, cmd: &str, args: &[&str]) -> anyhow::Result<String> {
        let out = tokio::process::Command::new(cmd)
            .args(args)
            .output()
            .await?;
        anyhow::ensure!(
            out.status.success(),
            "{cmd} exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

// ─── Volume ───────────────────────────────────────────────────────────────────

struct VolumeSkill {
    up: bool,
    shell: std::sync::Arc<dyn ShellExecutor>,
}

#[async_trait::async_trait]
impl Skill for VolumeSkill {
    async fn handle(&self, _params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult {
        let direction = if self.up { "volume_up" } else { "volume_down" };
        tracing::info!(direction, "skill: volume");

        let control = ctx.config.alsa_control.as_str();
        let step = ctx.config.volume_step_pct;
        let delta = if self.up {
            format!("{step}%+")
        } else {
            format!("{step}%-")
        };

        // amixer sset <control> <step>%± unmute
        let output = match self
            .shell
            .run("amixer", &["sset", control, &delta, "unmute"])
            .await
        {
            Ok(o) => o,
            Err(e) => {
                tracing::error!(error = %e, "amixer failed");
                return SkillResult::speak("Sorry, I couldn't adjust the volume right now.");
            }
        };

        let word = if self.up { "up" } else { "down" };
        SkillResult::speak(match parse_volume_pct(&output) {
            Some(p) => format!("Volume {word}, now at {p}%."),
            None => format!("Volume {word}."),
        })
    }
}

/// Extract the first `[N%]` from `amixer sset` / `amixer sget` output.
fn parse_volume_pct(output: &str) -> Option<u8> {
    let start = output.find('[')? + 1;
    let rest = &output[start..];
    let end = rest.find('%')?;
    rest[..end].trim().parse::<u8>().ok()
}

/// Used when the LLM returns action="respond" — reply text comes from the LLM response field.
struct RespondSkill;
#[async_trait::async_trait]
impl Skill for RespondSkill {
    async fn handle(&self, params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        let reply = params["response"].as_str().unwrap_or("Okay.").to_string();
        SkillResult::speak(reply)
    }
}

struct UnknownSkill;
#[async_trait::async_trait]
impl Skill for UnknownSkill {
    async fn handle(&self, _params: &serde_json::Value, _ctx: &SkillContext<'_>) -> SkillResult {
        tracing::warn!("skill: unknown action — no handler registered");
        SkillResult::speak("Sorry, I don't know how to do that yet.")
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
        SkillContext {
            node_id: "test",
            http_client: client,
            config,
            registry,
            brain_ip: "127.0.0.1",
        }
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
        let result = reg
            .dispatch("set_timer", &json!({"duration_seconds": 120}), &ctx)
            .await;
        // 120 s → "2 minutes"
        assert!(
            result.spoken_reply.contains("2 minutes"),
            "{}",
            result.spoken_reply
        );
    }

    #[tokio::test]
    async fn timer_skill_defaults_to_60s_when_param_missing() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("set_timer", &json!({}), &ctx).await;
        // 60 s → "1 minute"
        assert!(
            result.spoken_reply.contains("1 minute"),
            "{}",
            result.spoken_reply
        );
    }

    #[tokio::test]
    async fn volume_up_and_down_dispatched() {
        // Just verifies the actions route to a handler that returns something.
        // Behaviour is covered by volume_up/down_includes_percentage_in_reply tests
        // which use MockShell rather than a real amixer binary.
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx_up = make_ctx(&client, &config, &registry);
        assert!(!reg
            .dispatch("volume_up", &json!({}), &ctx_up)
            .await
            .spoken_reply
            .is_empty());
        let ctx_down = make_ctx(&client, &config, &registry);
        assert!(!reg
            .dispatch("volume_down", &json!({}), &ctx_down)
            .await
            .spoken_reply
            .is_empty());
    }

    #[tokio::test]
    async fn weather_skill_prompts_for_location_when_unconfigured() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default(); // latitude/longitude both None
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        let result = reg.dispatch("weather", &json!({}), &ctx).await;
        assert!(
            result.spoken_reply.contains("location"),
            "should prompt for location: {}",
            result.spoken_reply
        );
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

    // ── friendly_duration ─────────────────────────────────────────────────────

    #[test]
    fn friendly_duration_seconds_only() {
        assert_eq!(friendly_duration(1), "1 second");
        assert_eq!(friendly_duration(30), "30 seconds");
        assert_eq!(friendly_duration(59), "59 seconds");
    }

    #[test]
    fn friendly_duration_minutes_only() {
        assert_eq!(friendly_duration(60), "1 minute");
        assert_eq!(friendly_duration(300), "5 minutes");
        assert_eq!(friendly_duration(3600 - 60), "59 minutes");
    }

    #[test]
    fn friendly_duration_minutes_and_seconds() {
        assert_eq!(friendly_duration(61), "1 minute and 1 second");
        assert_eq!(friendly_duration(90), "1 minute and 30 seconds");
        assert_eq!(friendly_duration(125), "2 minutes and 5 seconds");
    }

    #[test]
    fn friendly_duration_hours_only() {
        assert_eq!(friendly_duration(3600), "1 hour");
        assert_eq!(friendly_duration(7200), "2 hours");
    }

    #[test]
    fn friendly_duration_hours_and_minutes() {
        assert_eq!(friendly_duration(3660), "1 hour and 1 minute");
        assert_eq!(friendly_duration(5400), "1 hour and 30 minutes");
        // Seconds are dropped when hours are present.
        assert_eq!(friendly_duration(3661), "1 hour and 1 minute");
    }

    // ── Timer callback ────────────────────────────────────────────────────────

    /// The timer should enqueue the callback text after the duration elapses.
    #[tokio::test]
    async fn timer_enqueues_tts_after_duration() {
        let reg = crate::session::SessionRegistry::new();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();

        // Use a very short duration so the test stays fast.
        let ctx = SkillContext {
            node_id: "pi-test",
            http_client: &client,
            config: &config,
            registry: &reg,
            brain_ip: "127.0.0.1",
        };
        let result = TimerSkill
            .handle(&json!({"duration_seconds": 0}), &ctx)
            .await;

        assert!(
            result.spoken_reply.contains("Timer set for"),
            "{}",
            result.spoken_reply
        );

        // Give the spawned task a moment to run.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let pending = reg.drain_pending_tts("pi-test").await;
        assert_eq!(pending.len(), 1, "expected one queued TTS message");
        assert!(pending[0].contains("timer is up"), "{}", pending[0]);
    }

    /// Multiple concurrent timers should all fire and each enqueue independently.
    #[tokio::test]
    async fn multiple_concurrent_timers_all_fire() {
        let reg = crate::session::SessionRegistry::new();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();

        for _ in 0..3 {
            let ctx = SkillContext {
                node_id: "pi-test",
                http_client: &client,
                config: &config,
                registry: &reg,
                brain_ip: "127.0.0.1",
            };
            TimerSkill
                .handle(&json!({"duration_seconds": 0}), &ctx)
                .await;
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let pending = reg.drain_pending_tts("pi-test").await;
        assert_eq!(pending.len(), 3, "all three timers should have fired");
    }

    // ── Volume / ShellExecutor ────────────────────────────────────────────────

    struct MockShell {
        result: std::result::Result<String, &'static str>,
    }

    #[async_trait::async_trait]
    impl ShellExecutor for MockShell {
        async fn run(&self, _cmd: &str, _args: &[&str]) -> anyhow::Result<String> {
            match &self.result {
                Ok(s) => Ok(s.clone()),
                Err(e) => anyhow::bail!("{e}"),
            }
        }
    }

    fn amixer_output(pct: u8) -> String {
        format!(
            "Simple mixer control 'Master',0\n\
             Capabilities: pvolume pvolume-joined pswitch pswitch-joined\n\
             Playback channels: Mono\n\
             Limits: Playback 0 - 65536\n\
             Mono: Playback 43384 [{pct}%] [on]\n"
        )
    }

    #[test]
    fn parse_volume_pct_extracts_percentage() {
        assert_eq!(parse_volume_pct(&amixer_output(66)), Some(66));
        assert_eq!(parse_volume_pct(&amixer_output(0)), Some(0));
        assert_eq!(parse_volume_pct(&amixer_output(100)), Some(100));
        assert_eq!(parse_volume_pct("no brackets here"), None);
    }

    #[tokio::test]
    async fn volume_up_includes_percentage_in_reply() {
        let shell = Arc::new(MockShell {
            result: Ok(amixer_output(70)),
        });
        let skill = VolumeSkill { up: true, shell };
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);

        let result = skill.handle(&json!({}), &ctx).await;
        assert!(
            result.spoken_reply.contains("up"),
            "{}",
            result.spoken_reply
        );
        assert!(
            result.spoken_reply.contains("70%"),
            "{}",
            result.spoken_reply
        );
    }

    #[tokio::test]
    async fn volume_down_includes_percentage_in_reply() {
        let shell = Arc::new(MockShell {
            result: Ok(amixer_output(40)),
        });
        let skill = VolumeSkill { up: false, shell };
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);

        let result = skill.handle(&json!({}), &ctx).await;
        assert!(
            result.spoken_reply.contains("down"),
            "{}",
            result.spoken_reply
        );
        assert!(
            result.spoken_reply.contains("40%"),
            "{}",
            result.spoken_reply
        );
    }

    #[tokio::test]
    async fn volume_skill_handles_amixer_failure() {
        let shell = Arc::new(MockShell {
            result: Err("amixer: command not found"),
        });
        let skill = VolumeSkill { up: true, shell };
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);

        let result = skill.handle(&json!({}), &ctx).await;
        assert!(
            result.spoken_reply.to_lowercase().contains("sorry"),
            "{}",
            result.spoken_reply
        );
    }

    #[tokio::test]
    async fn volume_skill_uses_config_control_and_step() {
        // Capture what args were passed to amixer.
        use std::sync::Mutex;
        struct SpyShell {
            calls: Mutex<Vec<(String, Vec<String>)>>,
        }
        #[async_trait::async_trait]
        impl ShellExecutor for SpyShell {
            async fn run(&self, cmd: &str, args: &[&str]) -> anyhow::Result<String> {
                self.calls.lock().unwrap().push((
                    cmd.to_string(),
                    args.iter().map(|s| s.to_string()).collect(),
                ));
                Ok(amixer_output(50))
            }
        }

        let shell = Arc::new(SpyShell {
            calls: Mutex::new(vec![]),
        });
        let skill = VolumeSkill {
            up: true,
            shell: shell.clone(),
        };
        let client = reqwest::Client::new();
        let config = SkillConfig {
            alsa_control: "PCM".into(),
            volume_step_pct: 15,
            ..Default::default()
        };
        let registry = crate::session::SessionRegistry::new();
        let ctx = make_ctx(&client, &config, &registry);
        skill.handle(&json!({}), &ctx).await;

        let calls = shell.calls.lock().unwrap();
        assert_eq!(calls[0].0, "amixer");
        assert_eq!(calls[0].1, ["sset", "PCM", "15%+", "unmute"]);
    }

    #[tokio::test]
    async fn all_registered_actions_return_non_empty_reply() {
        let reg = SkillRegistry::default();
        let client = reqwest::Client::new();
        let config = SkillConfig::default();
        let registry = crate::session::SessionRegistry::new();
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
            let ctx = make_ctx(&client, &config, &registry);
            let result = reg.dispatch(action, &json!({}), &ctx).await;
            assert!(!result.spoken_reply.is_empty(), "empty reply for {action}");
        }
    }
}
