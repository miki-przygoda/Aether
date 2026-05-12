# Epic: Skills Integration

## Goal

Replace the nine stub skills with real implementations. After this epic, Aether can report live weather, fire timer callbacks spoken aloud on the Pi, adjust ALSA volume, control an MPD music player, and toggle Home Assistant lights. The `respond` fallback (LLM general conversation) is already working and unchanged.

---

## Current State

| Action                                      | Current behaviour                                        |
|---------------------------------------------|----------------------------------------------------------|
| `weather`                                   | **Working** — live Open-Meteo call; speaks temp + conditions |
| `set_timer`                                 | Speaks confirmation but no callback fires yet            |
| `volume_up` / `volume_down`                 | "Volume up/down." — ALSA untouched                       |
| `play_music` / `pause_music` / `stop_music` | Config-missing stub; Navidrome compose entry ready       |
| `lights_on` / `lights_off`                  | Config-missing stub; HA URL/token fields in Settings     |
| `respond`                                   | **Working** — LLM reply spoken aloud                     |

---

## Architecture Changes Required

### 1. `SkillContext` — inject dependencies into every `handle()` call

The current signature `fn handle(&self, params: &serde_json::Value) -> SkillResult` cannot support async I/O or cross-skill state. Replace it with:

```rust
// aether-core or brain-node/src/skills.rs

pub struct SkillContext<'a> {
    pub node_id: &'a str,
    pub http_client: &'a reqwest::Client,
    pub config: &'a SkillConfig,
    pub registry: &'a SessionRegistry,
}

#[async_trait::async_trait]
pub trait Skill: Send + Sync {
    async fn handle(&self, params: &serde_json::Value, ctx: &SkillContext<'_>) -> SkillResult;
}
```

`SkillRegistry::dispatch` becomes `async fn dispatch(&self, action, params, ctx)`.

All call sites in `grpc.rs` must `await` the dispatch.

### 2. `SkillConfig` — user-configured settings

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SkillConfig {
    // Weather
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location_display_name: Option<String>, // e.g. "London, England" — shown in UI only
    pub weather_api_base: String,              // default "https://api.open-meteo.com" — override for self-hosted

    // Home Assistant
    pub home_assistant_url: Option<String>,   // e.g. "http://homeassistant.local:8123"
    pub home_assistant_token: Option<String>, // long-lived access token

    // Volume
    pub alsa_control: String, // default "Master"
    pub volume_step_pct: u8,  // default 10

    // Music (Navidrome / Subsonic)
    pub navidrome_url: Option<String>,      // e.g. "http://navidrome:4533"
    pub navidrome_user: Option<String>,
    pub navidrome_password: Option<String>,

    // Update checks
    pub update_check_enabled: bool,         // default true; monthly GitHub tag check
}
```

Stored as `config_dir/skills.json`. Loaded at brain startup. Editable via a new **Skills Settings** tab on the web UI settings page (or inline on `/ui/skills`).

### 3. Timer callback channel — deliver TTS when timer fires between utterances

When a timer fires and the triggering node is not in an active gRPC stream, the spoken reminder cannot be delivered immediately. The solution: a pending TTS queue per node in `SessionRegistry`.

Add to `SessionRegistry`:

```rust
pending_tts: Arc<RwLock<HashMap<NodeId, VecDeque<String>>>>,
```

New methods:

```rust
pub async fn enqueue_tts(&self, node_id: &str, text: String)
pub async fn drain_pending_tts(&self, node_id: &str) -> Vec<String>
```

In `grpc.rs`, at the start of each `audio_stream` handler, after registering the push channel, drain any pending TTS and synthesise + stream them to the node before entering the main accumulation loop. This way a "Your 5-minute timer is up!" message is delivered on the next wake word — typically within seconds on a smart speaker.

### 4. Shared `reqwest::Client` on `AppState`

Add `http_client: reqwest::Client` to `AppState`. Constructed once at startup (`reqwest::Client::new()`). Passed through `SkillContext` on every dispatch. Avoids spawning a new connection pool per request.

---

## Complete Network Access Audit

This section documents **every outbound network request Aether makes across its entire lifetime** — not just skills. Nothing is hidden. The goal is that a user who wants to firewall everything after setup can do so with a complete picture.

---

### Phase 1 — Initial setup (one-time, user-initiated)

These happen when you run `./scripts/download-models.sh` and `docker compose up --build` for the first time. They never happen again unless you explicitly re-run those commands or delete cached data.

| # | Domain | What's sent | What's received | Why |
|---|--------|-------------|-----------------|-----|
| 1 | `huggingface.co` | HTTP GET — file path only | Whisper medium (~1.5 GB) | STT model |
| 2 | `huggingface.co` | HTTP GET — file path only | Whisper large-v3-turbo (~1.5 GB) | STT fallback model |
| 3 | `github.com` | HTTP GET — release path only | Kokoro-82M ONNX (~325 MB) | TTS model |
| 4 | `raw.githubusercontent.com` | HTTP GET — file path only | Kokoro `config.json` (~10 KB) | Phoneme vocab |
| 5 | `github.com` | HTTP GET with `Range: bytes=0-20479` | First 20 KB of `voices.json` | Default voice embedding |
| 6 | `registry-1.docker.io` | Docker image pull | `ollama/ollama` image | Ollama runtime |
| 7 | `registry-1.docker.io` | Docker image pull | `qdrant/qdrant` image | Vector store |
| 8 | `registry.ollama.ai` | Ollama model pull | `llama3.2:3b` weights | LLM |
| 9 | `registry.ollama.ai` | Ollama model pull | `nomic-embed-text` weights | Embedding model |

No account, no authentication, no identifying headers for any of the above beyond a standard Docker/curl User-Agent. These are all anonymous downloads of public model weights.

---

### Phase 2 — Docker image freshness checks (every `docker compose up`)

**This is a hidden cost the original epic did not document.**

The `compose.yml` currently uses `:latest` tags for `ollama/ollama` and `qdrant/qdrant`. Docker checks Docker Hub on every `docker compose up` to see if a newer image exists. This means two network requests to `registry-1.docker.io` every time you restart the brain, even after initial setup.

**Fix:** Pin images to specific digest hashes in `compose.yml`:
```yaml
ollama:
  image: ollama/ollama@sha256:<digest>   # pinned — no Docker Hub check on restart

qdrant:
  image: qdrant/qdrant@sha256:<digest>  # pinned — no Docker Hub check on restart
```

After pinning, Docker only uses the cached local image. Zero network access on restart.

**The pinned digests must be updated manually** when the user decides to upgrade. This is the intended behaviour for a privacy-first setup — updates on your schedule, not Docker's.

---

### Phase 3 — Ollama runtime behaviour

**Ollama does not auto-update models and does not send telemetry by default.** However, the Ollama binary does make one external request that is worth being aware of:

- `ollama serve` may check `ollama.com` for a newer version of the Ollama daemon itself on startup. This is a single version-check HTTP request that sends the current Ollama version string and receives a latest-version response. No model data, no query data, no user data.

The compose.yml adds `OLLAMA_NOPRUNE=1` to prevent automatic removal of old model versions. There is no official `OLLAMA_NO_UPDATE_CHECK` env var at the time of writing — the update check behaviour should be confirmed against the Ollama changelog before deployment.

**Approach for Aether (documented in the Ollama update section below):** The brain handles update notification in-app. Ollama's own check is disabled if possible; otherwise it is isolated to the Docker network with no UI surface.

---

### Phase 4 — Qdrant telemetry

Qdrant collects anonymous usage telemetry by default (cluster health, collection counts — not document content). This is disabled in the `compose.yml` via:

```yaml
qdrant:
  environment:
    QDRANT__TELEMETRY_DISABLED: "true"
```

**This flag must be present in `compose.yml`.** Without it, Qdrant sends periodic reports to `telemetry.qdrant.io`.

---

### Phase 5 — Runtime, weather skill only

After initial setup and with image digests pinned, the **only remaining outbound traffic** comes from the weather skill.

#### Open-Meteo — forecast request

Sent once per "what's the weather?" query. Never sent when the skill is not used.

```
GET https://api.open-meteo.com/v1/forecast
  ?latitude=<stored_lat>
  &longitude=<stored_lon>
  &current_weather=true
  &hourly=precipitation_probability
  &forecast_days=1
  &wind_speed_unit=kmh
  &temperature_unit=celsius

Host: api.open-meteo.com
User-Agent: aether-brain/0.1 (reqwest)
Accept: application/json
```

What is sent: two floating-point numbers (your location). Nothing else — no utterance text, no node ID, no session data, no identifying headers.

#### Open-Meteo — geocoding request (location setup only)

Sent only when the user types a city name into the Skills Settings location picker. Not sent during normal use.

```
GET https://geocoding-api.open-meteo.com/v1/search
  ?name=<city_name>
  &count=5
  &language=en
  &format=json

Host: geocoding-api.open-meteo.com
User-Agent: aether-brain/0.1 (reqwest)
Accept: application/json
```

What is sent: the city name string the user typed. Once the user picks a result and saves, the coordinates are stored locally and this API is never called again.

#### Open-Meteo — self-hosting

Users who want zero outbound traffic even for weather can self-host Open-Meteo (it is open source, Docker image available). Set `SkillConfig.weather_api_base` to point at the local instance. After that, Aether makes no outbound internet requests under any circumstances.

---

### Phase 6 — Music (Spotify, if configured)

If the user enables Spotify as a music source, `librespot` makes outbound connections to Spotify's servers (`ap.spotify.com`, `audio-*.spotifycdn.com`). This is fully opt-in, requires the user's own Spotify credentials, and involves no Aether servers. See the Music skill section for details.

---

### Summary: what leaves your network after initial setup

| Scenario | Outbound traffic |
|----------|-----------------|
| Normal use, weather disabled | **Nothing** |
| Normal use, weather enabled | lat/lon to `api.open-meteo.com` on each weather query |
| Spotify music enabled | Audio stream from Spotify's CDN (user's own account) |
| Compose uses `:latest` tags (unfixed) | Docker Hub image check on every restart |
| Qdrant telemetry not disabled (unfixed) | Periodic report to `telemetry.qdrant.io` |

---

## Ollama Update Strategy

Auto-updates are off by default. The brain never upgrades Ollama models without the user asking. The strategy has three layers:

### 1. Startup version check (monthly, background)

On brain startup, if more than 30 days have passed since the last check, the brain compares the running Ollama version against the latest release tag on `api.github.com/repos/ollama/ollama/releases/latest`. This is a single unauthenticated HTTPS GET:

```
GET https://api.github.com/repos/ollama/ollama/releases/latest

Host: api.github.com
User-Agent: aether-brain/0.1
Accept: application/vnd.github+json
```

What's sent: nothing identifying — just a public API request for release metadata. What's received: the latest version tag and release notes summary.

If a new version is available, a **persistent notification badge** appears on the brain dashboard. The check timestamp is stored in `config_dir/update_check.json`. No auto-download, no prompt, no restart — just a badge.

The check can be disabled entirely by setting `update_check_enabled: false` in `SkillConfig`. Users who want full offline operation can turn this off and never have it contact GitHub.

### 2. Manual "Check for updates" button

On the dashboard or settings page, a **"Check for updates"** button triggers an immediate version check regardless of the monthly schedule. Returns inline: current version, latest version, changelog link.

### 3. Manual "Update now" button

If a newer version is available, an **"Update now"** button appears (separate from the notification badge). Clicking it runs `ollama pull <model>` for each configured model in sequence, streaming progress via SSE to the UI. The user sees exactly what is being downloaded and from where before clicking.

**Ollama daemon updates** (the container image itself) require the user to pull a new Docker image manually — `docker compose pull && docker compose up -d`. The UI shows instructions rather than attempting to restart Docker from within the container.

### Summary

| Action | Who initiates | Network contact |
|--------|--------------|-----------------|
| Monthly version check | Brain (background, startup) | `api.github.com` — version tag only |
| Manual check | User clicks button | Same |
| Model download | User clicks "Update now" | `registry.ollama.ai` — model weights |
| Container update | User runs docker commands | `registry-1.docker.io` |

---

## Skills — Implementation Plan

### P0: Weather (Open-Meteo)

**Why P0:** Most-requested smart speaker feature. No API key. Completely private — the brain calls Open-Meteo directly over the local network's NAT.

**Forecast API:** `GET https://api.open-meteo.com/v1/forecast` (full URL documented in External Network Access section above).

**Response fields used:**
- `current_weather.temperature` — °C
- `current_weather.windspeed` — km/h
- `current_weather.weathercode` — WMO code → English description

**WMO code table** (implement in full):

| Code range | Description            |
|------------|------------------------|
| 0          | Clear sky              |
| 1–3        | Partly cloudy          |
| 45, 48     | Fog                    |
| 51–57      | Drizzle                |
| 61–67      | Rain                   |
| 71–77      | Snow                   |
| 80–82      | Rain showers           |
| 95         | Thunderstorm           |
| 96, 99     | Thunderstorm with hail |

**Spoken reply format:** `"Currently {description} and {temp}°C, with wind at {wind} km/h."`

**Graceful degradation:** If `latitude`/`longitude` not configured → `"Weather needs your location set in Skills Settings."`. If HTTP fails → `"Couldn't reach the weather service right now."`.

**Implementation location:** `crates/brain-node/src/skills/weather.rs`

**Tests:**
- Mock HTTP server returns known JSON → assert spoken reply contains temperature and description
- Missing lat/lon → assert config-missing message
- HTTP 500 → assert graceful error message

---

### P0: Timer with callback

**Why P0:** "Set a timer" with no callback is worse than useless — it trains the user to distrust the assistant.

**Parsing:**

The LLM already extracts `duration_seconds` from the utterance. The skill receives it in params.

**Implementation:**

```rust
// Inside TimerSkill::handle (simplified)
let secs = params["duration_seconds"].as_u64().unwrap_or(60);
let node_id = ctx.node_id.to_string();
let registry = ctx.registry.clone(); // Arc inside
let label = friendly_duration(secs); // "5 minutes", "30 seconds", etc.

tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let text = format!("Your {label} timer is up.");
    // Try immediate push (node mid-utterance).
    if registry.push_tts_immediate(&node_id, text.clone()).await == 0 {
        // Node not connected — queue for next wake.
        registry.enqueue_tts(&node_id, text).await;
    }
});

SkillResult { spoken_reply: format!("Timer set for {label}.") }
```

`push_tts_immediate` reuses the existing `push_txs` channel infrastructure, sending a synthesised WAV rather than a model binary. Requires extending the push channel payload to be an enum:

```rust
enum PushPayload {
    WakeWordModel(Vec<u8>),
    TtsText(String), // brain synthesises WAV, then streams as TtsChunk
}
```

Or simpler: keep `push_txs` for model-only, add a separate `tts_push_txs: HashMap<NodeId, mpsc::Sender<String>>` registered per stream in `grpc.rs`. The `grpc.rs` stream handler selects on three arms: audio chunks, model push, TTS push.

**Tests:**
- Timer fires after sleep → `enqueue_tts` called with correct text
- `friendly_duration(90)` → `"1 minute and 30 seconds"`
- `friendly_duration(300)` → `"5 minutes"`
- Multiple concurrent timers → each fires independently

---

### P1: Volume (ALSA amixer)

**Why P1:** Volume control works entirely on the brain's Docker host. No external dependencies.

**Implementation:** Subprocess call to `amixer`:

```bash
amixer sset Master 10%+    # volume_up
amixer sset Master 10%-    # volume_down
amixer sget Master          # read current level
```

Parse `amixer sget` output to extract current percentage before/after, include in spoken reply: `"Volume at 60%."`.

The ALSA control name and step size come from `SkillConfig.alsa_control` and `SkillConfig.volume_step_pct`.

**Graceful degradation:** If `amixer` not found or returns non-zero → `"Couldn't adjust the volume right now."`.

**Implementation location:** `crates/brain-node/src/skills/volume.rs`

**Tests:**
- Mock subprocess executor returns known `amixer sget` output → assert percentage in reply
- `amixer` missing → assert graceful error
- Step clamped at 0% and 100% (no negative or >100% requests sent)

Note: In Docker, `amixer` controls the host's ALSA. The Docker container must have `/dev/snd` mounted. Add to `docker-compose.yml`:
```yaml
brain-node:
  devices:
    - /dev/snd:/dev/snd
```

---

### P2: Music (Navidrome + Subsonic API)

**Why Navidrome over plain MPD or a custom solution:**

Plain MPD has no HTTP API and no library search — you'd have to build a search layer on top. A custom music app is large scope and duplicates what Navidrome already does well. Navidrome is open source, has no accounts, runs as a single Docker container, and speaks the Subsonic API — a well-documented protocol with wide client support. It handles library indexing, metadata, artwork, and search. Aether's role is a thin integration layer on top: query Navidrome via Subsonic, stream audio to the Pi via gRPC.

**What Plex and Spotify looked like and why they're out:**

- **Plex:** Requires a plex.tv account to claim the server. Matches metadata via cloud even for local files. Eliminated on privacy grounds — no account means no Plex.
- **Spotify (librespot):** librespot technically violates Spotify's Terms of Service. Spotify has killed third-party clients before; shipping it first-party means a reliability dependency outside Aether's control on top of the privacy compromise. Deferred — if there's community demand, it can be added as an optional community extension later, clearly labelled as unsupported.

**Architecture:**

```
./music/          ← user drops FLAC/MP3/OPUS files here
   └── (bind mount → Navidrome container)
         └── Navidrome indexes library, serves Subsonic API on :4533

brain-node
   └── NavidromeClient (Subsonic REST)
         ├── search("miles davis") → track list
         ├── stream(trackId)       → raw audio bytes
         └── nowPlaying()          → title, artist

brain-node gRPC → MusicChunk stream → edge-node → cpal playback on Pi speaker
```

**compose.yml addition:**

```yaml
navidrome:
  image: deluan/navidrome:<pinned-digest>
  restart: unless-stopped
  ports:
    - "4533:4533"
  volumes:
    - ./music:/music:ro
    - navidrome-data:/data
  environment:
    ND_SCANSCHEDULE: "@every 1h"
    ND_LOGLEVEL: warn
    ND_BASEURL: ""
    ND_ENABLEDOWNLOADS: "false"
    ND_ENABLESHARING: "false"
    ND_LASTFM_ENABLED: "false"   # no external metadata
    ND_SPOTIFY_ID: ""            # no Spotify integration
    ND_LISTENBRAINZ_ENABLED: "false"
```

`ND_LASTFM_ENABLED: "false"` and `ND_LISTENBRAINZ_ENABLED: "false"` are required — Navidrome supports Last.fm scrobbling and ListenBrainz, both of which send play data externally. Both must be explicitly disabled.

**Subsonic API calls (all local network, no internet):**

```
GET http://navidrome:4533/rest/search3?query=<q>&songCount=20&f=json&u=<user>&t=<token>&s=<salt>&v=1.16.1&c=aether
GET http://navidrome:4533/rest/stream?id=<trackId>&f=json&u=<user>&t=<token>&s=<salt>&v=1.16.1&c=aether
GET http://navidrome:4533/rest/getNowPlaying?f=json&...
```

All requests stay within the Docker network. Token authentication uses MD5(password + salt) — no plaintext credentials on the wire.

**Music skill actions:**

| Voice command | Skill action | What brain does |
|---|---|---|
| "Play Miles Davis" | `play_music { query: "miles davis" }` | Subsonic search → queue top result → stream |
| "Play some jazz" | `play_music { genre: "jazz" }` | Subsonic genre search → shuffle → stream |
| "Pause" | `pause_music` | Signal Navidrome stream pause |
| "Stop the music" | `stop_music` | Stop stream, clear queue |

**Spoken replies:** `"Playing 'Kind of Blue' by Miles Davis."` — from Navidrome's `getNowPlaying` response. Falls back to `"Playing music."` if metadata is sparse.

**Graceful degradation:** If Navidrome not reachable → `"Music isn't set up — see the Music section in Settings."`. If search returns no results → `"Couldn't find anything matching that in your library."`.

**Implementation location:** `crates/brain-node/src/skills/music.rs` + `crates/brain-node/src/navidrome.rs` (Subsonic client)

**Tests:**
- Mock Subsonic HTTP server → assert correct search query sent, correct track queued
- Navidrome unreachable → assert graceful error
- Search with no results → assert "couldn't find" message
- `getNowPlaying` response parsed correctly into title/artist

**Future extensions (not v1):**
- Spotify: possible community extension, labelled unsupported, librespot ToS risk documented
- Custom standalone music app: tracked in `todos/concept-standalone-music-app.md`

---

### P2: Lights (Home Assistant)

**Why P2:** Requires Home Assistant. Completely optional — brain operates fully without it. Stub message updated to explain what's needed.

**API:**

```
POST /api/services/light/turn_on
POST /api/services/light/turn_off

Headers:
  Authorization: Bearer {token}
  Content-Type: application/json

Body: { "entity_id": "light.all" }  // or specific entity from params
```

The LLM extracts `entity_id` (or a room name) from the utterance. A simple lookup table in `SkillConfig` maps room names → HA entity IDs:

```json
{ "room_map": { "office": "light.office", "bedroom": "light.bedroom_main" } }
```

**Graceful degradation:** If `home_assistant_url` not configured → `"Lights need Home Assistant set up in Skills Settings."`. HTTP error → `"Couldn't reach Home Assistant."`.

**Implementation location:** `crates/brain-node/src/skills/lights.rs`

**Tests:**
- Mock HA server → assert correct entity_id sent for each action
- No HA URL configured → assert config-missing message
- Room name mapped correctly via `room_map`

---

## Web UI Changes

### Skills Settings tab

New page at `GET /ui/settings/skills` (linked from the existing settings nav).

#### Location setup (weather)

The user should never have to look up raw coordinates. Use a city-name search backed by the Open-Meteo geocoding API (see External Network Access section — one-time request, setup only).

**Flow:**

1. User types a city name into a text input (e.g. "London" or "New York").
2. After 400 ms debounce, the browser calls `GET /api/skills/location-search?q=<name>` (HTMX `hx-trigger="keyup changed delay:400ms"`).
3. Brain proxies to Open-Meteo geocoding API and returns up to 5 candidates as an HTML partial:
   ```
   London, England, United Kingdom  (51.51, -0.13)   [Select]
   London, Ontario, Canada           (42.98, -81.24)  [Select]
   ...
   ```
4. User clicks **Select** on the correct row → `POST /api/skills/location-confirm { lat, lon, display_name }` → stored in `SkillConfig`, UI updates to show `"Location: London, England (51.51°N, 0.13°W)"`.
5. A small **"Clear"** link resets to unset state.

If the user already has a location set, the settings page shows the saved display name and coordinates with a "Change location" link that re-opens the search input.

**No raw lat/lon text fields exposed in the UI.** The search flow is the only entry point; this prevents misconfiguration and makes it clear an outbound request is happening (search visibly fetches suggestions).

**Privacy disclosure inline on the page:** A small info banner under the search input reads:
> *City search uses the Open-Meteo geocoding API (open-meteo.com). Your search term is sent once to resolve coordinates. The coordinates are then stored locally and used for weather queries — no further data about your device or queries is sent.*

#### Home Assistant

- URL field (`http://homeassistant.local:8123`) + long-lived token field (masked, show/hide toggle).
- "Test connection" button → `POST /api/skills/ha-test` → returns `{ ok: true, version: "2024.x" }` or error; shown inline.
- Room mapping table: editable list of room name → HA entity ID pairs. Add/remove rows. Example row: `"office"` → `"light.office"`.

#### Volume

- ALSA control selector: populated via `GET /api/skills/alsa-controls` which runs `amixer scontrols` on the host. Dropdown, default "Master".
- Step size: 1–30% slider (default 10%).

#### MPD

- Host and port fields (defaults `127.0.0.1:6600`).
- "Test connection" button → `POST /api/skills/mpd-test` → returns `{ ok: true, mpd_version: "0.23" }` or error.

#### Save

Single "Save settings" button at the bottom → `POST /api/settings/skills` with the full `SkillConfig` JSON body. Page shows a toast on success/failure.

### Updated skill tester

The `/ui/skills` tester already shows matched skill and LLM JSON. After this epic, also show the actual spoken reply (post-execution, not pre-execution) so users can verify real skill output.

### New API routes (skills settings)

```
GET    /api/skills/location-search?q=<name>  → proxy to Open-Meteo geocoding; returns HTML partial
POST   /api/skills/location-confirm          → save { lat, lon, display_name } to SkillConfig
GET    /api/settings/skills                  → current SkillConfig as JSON
POST   /api/settings/skills                  → update SkillConfig
GET    /api/skills/alsa-controls             → list ALSA controls from amixer scontrols
POST   /api/skills/ha-test                   → test HA connection; returns { ok, version } or error
POST   /api/skills/mpd-test                  → test MPD connection; returns { ok, mpd_version } or error
```

---

## New Dependencies

| Crate         | Version | Purpose                       |
|---------------|---------|-------------------------------|
| `reqwest`     | 0.12    | Weather + Home Assistant HTTP |
| `async-trait` | 0.1     | Async Skill trait             |
| `mpd`         | 0.1     | MPD client (or raw TCP, TBD)  |

`reqwest` should use `default-features = false, features = ["json", "rustls-tls"]` to avoid OpenSSL and keep the Docker image lean.

---

## Subprocess Isolation

Volume uses `amixer` as a subprocess. Wrap in a thin abstraction for testability:

```rust
pub trait ShellExecutor: Send + Sync {
    async fn run(&self, cmd: &str, args: &[&str]) -> Result<String>;
}

pub struct RealShell;
// impl ShellExecutor for RealShell — calls tokio::process::Command

pub struct MockShell { output: String }
// impl ShellExecutor for MockShell — returns fixed output
```

Inject into `VolumeSkill` at construction. This keeps volume tests fast and hermetic.

---

## Tasks

### Housekeeping (do first — addresses hidden network issues)
- [ ] Pin `ollama/ollama` and `qdrant/qdrant` to specific image digests in `compose.yml`
- [x] Add `QDRANT__TELEMETRY_DISABLED: "true"` to `compose.yml` Qdrant environment block
- [ ] Research Ollama startup update-check behaviour — document or disable if possible
- [ ] Dashboard: notification badge when Ollama update available; "Check for updates" + "Update now" buttons
- [x] Music bind-mount (`./music:/music:ro`) added to `compose.yml` brain-node service

### Foundation
- [x] Add `async-trait` dependency; make `Skill::handle` async with `SkillContext`
- [x] Add `SkillConfig` struct (incl. `update_check_enabled`, `weather_api_base`); load/save `skills.json` from `config_dir`
- [x] Add `http_client: reqwest::Client` to `AppState` and `BrainService`; share via `Arc`
- [x] Add `pending_tts` queue to `SessionRegistry` (`enqueue_tts`, `drain_pending_tts`)
- [x] Update all `SkillRegistry::dispatch` call sites in `grpc.rs` and `web_ui` to be async
- [ ] Wire `drain_pending_tts` into `grpc.rs` `audio_stream` handler at stream start

### Weather (P0)
- [x] Implement `WeatherSkill` with live Open-Meteo HTTP call via `ctx.http_client`
- [x] Full WMO weathercode → English description table (28 codes)
- [ ] Lat/lon config validation at startup (tracing warn if missing)
- [x] Tests: mock axum server (happy path), missing config, HTTP failure, bad JSON response

### Timer (P0)
- [ ] `friendly_duration(secs: u64) -> String` helper (`"5 minutes"`, `"1 minute and 30 seconds"`, etc.)
- [ ] `TimerSkill` spawns `tokio::spawn`; calls `enqueue_tts` on fire
- [ ] Tests: callback fires, `friendly_duration` edge cases, concurrent timers

### Volume (P1)
- [ ] `ShellExecutor` trait + `RealShell` impl (`tokio::process::Command`)
- [ ] `VolumeSkill` uses `amixer` subprocess; parses output percentage; states new level in reply
- [ ] Docker compose: mount `/dev/snd`
- [ ] Tests: mock executor, clamping, parse output

### Music (P2)
- [x] Add Navidrome to `compose.yml`; all external integrations disabled (Last.fm, ListenBrainz, Spotify, downloads, sharing)
- [x] `./music:/music:ro` bind mount in compose
- [ ] `NavidromeClient` in `brain-node/src/navidrome.rs` — Subsonic API: `search3`, `stream`, `getNowPlaying`
- [ ] Add new `MusicChunk` proto message (or reuse `TtsChunk`) for gRPC audio streaming to Pi
- [ ] `MusicSkill` (shared struct for play/pause/stop) queries `NavidromeClient`, streams to Pi
- [ ] Tests: mock Subsonic HTTP server, unreachable Navidrome, empty search results

### Lights (P2)
- [ ] `LightsSkill` with Home Assistant REST API
- [ ] `room_map` in `SkillConfig` with fallback to `light.all`
- [ ] Tests: mock HA server, missing config, room mapping

### Web UI
- [x] Skills Settings page at `/ui/settings/skills`
- [x] Location city-search UI with button-click result selection
- [x] `GET /api/skills/location-search?q=` — proxies Open-Meteo geocoding API
- [x] `GET /api/settings/skills` + `POST /api/settings/skills` — read/write full `SkillConfig`
- [x] "Skills" entry added to Settings section of sidebar nav
- [ ] Privacy disclosure banner on location search section
- [ ] `GET /api/skills/alsa-controls` — runs `amixer scontrols`, returns control list for dropdown
- [ ] `POST /api/skills/ha-test` — test HA connection; returns `{ ok, version }` or error
- [ ] HA room mapping table (add/remove rows)
- [ ] Navidrome "Test connection" button in music section

---

## Acceptance Criteria

- [ ] "What's the weather?" returns live temperature and conditions (requires lat/lon configured)
- [ ] "Set a timer for 5 minutes" → "Timer set for 5 minutes." → 5 minutes later, next wake word triggers "Your 5-minute timer is up."
- [ ] "Volume up" / "Volume down" adjusts ALSA level and states new percentage in reply
- [ ] "Play Miles Davis" / "Play some jazz" searches Navidrome library and streams audio to Pi speaker
- [ ] Music playback gracefully fails with a helpful message if Navidrome not configured
- [ ] Navidrome Last.fm, ListenBrainz, and Spotify integrations confirmed disabled — no external music calls
- [ ] "Lights on" / "Lights off" calls Home Assistant (requires HA configured)
- [ ] All stubs degrade gracefully with a helpful config-missing message rather than a generic error
- [ ] Skills Settings page saves and reloads config across brain restarts
- [ ] All new code: clippy clean, no `unwrap()` in library code
- [ ] Unit tests cover all skills with mocked I/O; CI stays green

## Status: In Progress
