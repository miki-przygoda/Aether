# Concept: Aether Music — Standalone Privacy-First Music Player

## Status: Future Product — Not Started

This is a future product concept, not an active development epic. It is documented here so the idea is preserved with enough detail to pick up cleanly when the time comes.

**Current music integration uses Navidrome** (see `todos/concept-music-app.md`). This standalone app would eventually replace Navidrome as the backend for users who want a single cohesive Aether ecosystem, while also working completely independently for users who don't run Aether at all.

---

## The idea in one paragraph

A lightweight, self-hosted music player you run on your own hardware. Your music files live on your machine. No accounts, no cloud metadata lookup, no analytics. You control playback from a browser on your local network or via voice through Aether. It speaks the Subsonic protocol so existing mobile apps (DSub, Symfonium, Ultrasonic) connect to it out of the box. If you run Aether, your Pi nodes automatically use it as the music source. If you don't, it works as a standalone product.

---

## Why build it when Navidrome exists

Navidrome is good. It covers library management, metadata, artwork, and the Subsonic API cleanly. For v1 Aether, Navidrome is the right answer.

The case for a custom app comes down to a few things Navidrome can't do by design:

**1. Aether-native gRPC streaming**
Navidrome serves audio over HTTP. Aether's Pi integration works by pulling audio from Navidrome over HTTP and re-streaming it over gRPC. A custom app built for Aether could speak gRPC natively and remove that translation layer. Less moving parts, lower latency, cleaner architecture.

**2. Per-node queue management**
If you have three Pi nodes in different rooms, you want independent queues. Navidrome has no concept of "rooms" or "nodes" — it has a single play queue. A custom app could model the node topology directly: "play in the office", "sync music across the house".

**3. Voice-first design**
A regular music player is designed for mouse/touch interaction first, then adapted for voice. A music app built alongside Aether's voice system could be designed the other way: voice as the primary interface, with the UI as a secondary control surface. Difference in things like: "add to queue" vs "play now", how shuffle works, how it handles ambiguous requests.

**4. Single ecosystem, one UI**
Right now, the Aether web UI lives at `:8080` and Navidrome lives at `:4533`. Two ports, two authentication systems, two visual languages. A custom app would be a section of the Aether web UI — same nav, same style, same auth.

**5. Branding and distribution**
A standalone "Aether Music" Docker image (`docker run aether-music`) could attract users who want a privacy-first music server but aren't interested in a smart speaker. It grows the ecosystem and the brand without adding complexity to the core product.

---

## What it would contain

### Library manager
- Watches a configured folder for music files (FLAC, MP3, OPUS, AAC, WAV, AIFF)
- Indexes metadata from file tags — no external lookup, no MusicBrainz, no Last.fm. What's in the file is what you get.
- SQLite database (no Qdrant, no vector search — relational queries are sufficient for music)
- Artwork extracted from embedded tags or `cover.jpg` / `folder.jpg` in the album directory
- User can edit tags in the UI if they're wrong or missing

### Stream server
- HTTP chunked audio for browser playback
- gRPC `MusicChunk` stream for Aether Pi nodes (direct, no HTTP intermediary)
- Subsonic API for third-party clients (DSub, Symfonium, etc.)
- Transcode on-the-fly if needed: e.g. FLAC → OPUS for bandwidth-constrained connections

### Web UI
- Runs as part of the Aether brain's web server at `/ui/music`, or as a standalone server if deployed independently
- Artist → Album → Track tree view
- Search: fuzzy match across title/artist/album
- Queue: add, reorder, clear, shuffle, repeat
- Now-playing card with artwork
- Per-node playback control (which Pi is playing what)
- Mobile-friendly layout

### Aether voice integration
- "Play Miles Davis" → library search → queue → stream to Pi
- "Play something relaxing" → genre/mood search → shuffle → stream
- "What's playing?" → now-playing state → spoken reply
- "Skip this" / "Pause" / "Stop" → playback control
- "Add this to my favourites" → playlist management

---

## Privacy model

| Data             | Where it lives                   | Who can see it |
|------------------|----------------------------------|----------------|
| Music files      | Local filesystem / Docker volume | You only       |
| Track metadata   | SQLite on your machine           | You only       |
| Embedded artwork | Local filesystem                 | You only       |
| Play history     | SQLite, can be disabled          | You only       |
| Nothing          | Any external server              | Nobody         |

Zero outbound network requests under any circumstances. No accounts. No telemetry. No metadata enrichment from the internet. The app knows exactly as much about your music as your file tags say, and nothing more.

---

## Spotify as an explicit opt-in add-on

For users who subscribe to Spotify and want to mix streaming with their local library, the app could support `librespot` as a second source. This is treated differently from local files in every respect:

- Clearly labelled "Streaming" in the UI, visually distinct from local library
- Explicit disclosure on the connection screen: *"Spotify sees your listening activity. Your Spotify credentials are stored locally and only sent to Spotify's servers. Aether has no access to them."*
- `librespot` has historically violated Spotify's Terms of Service; this is disclosed and the integration is flagged as community-supported, not first-party
- Local files always take priority if a match exists — Spotify is a fallback, not the default
- Disabling Spotify removes all librespot processes and clears stored credentials immediately

This is an opt-in for users who already have Spotify and want convenience. It is not positioned as a privacy feature and not marketed as such.

---

## Standalone distribution

```
docker run -p 8080:8080 -v ./music:/music ghcr.io/aether-project/aether-music:latest
```

That's it. No other dependencies. Browse to `http://localhost:8080`, add music files to `./music`, done.

The image is built from the same Rust codebase as the brain-node, sharing the `aether-core` crate. The music app is a feature-flagged build target, not a separate repository. This keeps maintenance cost low and ensures the gRPC protocol stays in sync between the app and the brain.

---

## Integration contract with Aether

When Aether Music is running alongside the brain, the brain detects it (via a configurable URL in `compose.yml`) and switches the music skill from Navidrome to Aether Music automatically. The Subsonic API surface stays the same, so the skill code doesn't change — only the configured backend URL.

When Aether Music is deployed as a standalone product (no brain), it operates as a fully self-contained music server. The gRPC Pi streaming feature is simply unused.

---

## When to build this

Not before the following are true:

1. Aether v1 is stable and the skills epic is complete (Navidrome integration working)
2. There is clear user demand for features Navidrome can't provide (per-node queues, native gRPC, unified UI)
3. The scope is confirmed to be a focused music app — not scope-creeping into video, podcasts, or other media

Navidrome is the right answer for as long as it meets user needs. This app exists for when it doesn't.
