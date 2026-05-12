# Concept: Music Integration via Navidrome

## What this is

Notes on the v1 music architecture decision and the reasoning behind it. The active implementation tasks are in `todos/epic-skills-integration.md`. This document covers the why, what was evaluated, and what comes next.

For the longer-term custom standalone music player concept, see `todos/concept-standalone-music-app.md`.

---

## What was evaluated

| Option | Verdict | Reason |
|--------|---------|--------|
| **Plain MPD** | Rejected for v1 | No HTTP API, no library search. Would require building a search and indexing layer from scratch on top of it. |
| **Navidrome** | **Selected for v1** | Open source, no accounts, single Docker container, Subsonic API, handles indexing/metadata/artwork. Aether adds a thin integration layer. |
| **Plex** | Eliminated | Requires a plex.tv account to claim the server. Cloud metadata matching even for local files. Incompatible with the privacy stance. |
| **Spotify (librespot)** | Deferred | librespot violates Spotify's ToS; they've killed third-party clients before. Fragile first-party dependency. Privacy compromise on top. May return as an unsupported community extension if there's demand. |
| **Jellyfin** | Not chosen | Feature-complete but heavy; music is secondary to video; more complexity than needed. |
| **Custom app** | Future product | Worth building eventually but large scope. Not the right call when Navidrome solves v1 cleanly. See `concept-standalone-music-app.md`. |

---

## How Navidrome fits into Aether

Navidrome runs as an additional service in `compose.yml`. It watches the `./music` bind mount, indexes files, and exposes the Subsonic REST API on `:4533` within the Docker network.

The brain-node contains a `NavidromeClient` that speaks Subsonic. When the music skill fires, it:
1. Calls `search3` with the LLM-extracted query, genre, or artist
2. Gets back a track list from Navidrome's local SQLite database
3. Calls `stream` to pull raw audio bytes — no re-encoding if the source is already a compatible format
4. Repackages audio as a `MusicChunk` gRPC stream and forwards to the Pi
5. Pi plays via cpal, same pipeline as TTS

The key point: **no internet requests at any step**. Navidrome only looks at local files. The Subsonic API call is local network only. The gRPC stream to the Pi is mTLS on the local network.

---

## Privacy configuration for Navidrome

Navidrome supports Last.fm scrobbling and ListenBrainz — both send play data to external servers. Both are disabled explicitly in `compose.yml`:

```yaml
navidrome:
  environment:
    ND_LASTFM_ENABLED: "false"
    ND_LISTENBRAINZ_ENABLED: "false"
    ND_SPOTIFY_ID: ""
    ND_ENABLEDOWNLOADS: "false"
    ND_ENABLESHARING: "false"
```

With these set, Navidrome makes **zero outbound network requests**. It reads local files and responds to local API calls only.

---

## What the user needs to do

1. Drop music files (FLAC, MP3, OPUS, AAC) into the `./music` folder alongside `compose.yml`
2. Navidrome scans automatically on a schedule (default: every hour) and on startup
3. Voice commands work immediately once files are indexed

No tagging tool required — Navidrome reads whatever tags are already in the files. Sparse or missing tags show "Unknown Artist / Unknown Album" rather than reaching out to fill them in.

---

## Future path

Once v1 is stable, there are two directions:

1. **Deeper Navidrome integration** — playlist management via voice, "what's in my library?", playback history, per-room queues for multiple Pi nodes
2. **Custom standalone app** — a purpose-built privacy-first music player that integrates natively with Aether. Documented in `todos/concept-standalone-music-app.md`. This would eventually replace Navidrome as the backend if built, but is not required for a clean launch.

## Status: Design complete — implementation tracked in epic-skills-integration.md
