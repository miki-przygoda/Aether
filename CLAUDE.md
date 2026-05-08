# Aether — Claude Instructions

This is a **public open-source repository**. Everything in this file and the repo root may be seen by anyone.

## Critical Rule: Always Confirm Before Committing

Before staging, committing, or pushing anything, explicitly list the files to be committed and ask the user to confirm. No exceptions. This project is public — secrets, hardware specifics, or private notes must never reach the remote.

## Project Summary

Aether is a local-first, privacy-centric smart speaker built in Rust. It runs all AI inference (STT, LLM, TTS) on the user's own hardware with no external API calls.

**Core concept:** decouple always-on edge listening (ARM SBC nodes) from heavy AI inference (a local x86 GPU machine), connected via gRPC over an encrypted Tailscale network.

## What's Safe to Be Public

- Architecture diagrams and general system design
- Rust source code and crate dependencies
- Generic hardware roles (edge node, brain node, auxiliary node) without specific model names
- Tool and library names (Porcupine, Whisper, Ollama, Piper, Kokoro, cpal, tonic, rppal)
- The development roadmap at a phase level
- Privacy and security principles

## What Must Stay Private

- Specific hardware model names and specs (see `private/CLAUDE.md`)
- Network topology, IP addresses, Tailscale node names
- Any API keys, access tokens, or credentials
- Porcupine `.ppn` model files or access keys
- Personal use-case details (home/office layout, etc.)
- Anything in the `private/` directory

## Tech Stack (reference)

- **Language:** Rust (memory safety + zero-cost async for audio buffers)
- **Audio:** `cpal`
- **Wake Word:** Porcupine (`pvporcupine`)
- **Networking:** `tonic` (gRPC) / Tailscale
- **STT:** `whisper-rs` (Whisper.cpp bindings)
- **LLM:** Ollama
- **TTS:** Piper or Kokoro-82M
- **GPIO:** `rppal`
- **Cross-compile:** `cross-rs`

## Coding Standards

- Rust edition 2021
- `async`/`await` via Tokio throughout
- Prefer `thiserror` for error types, `tracing` for structured logs
- No `unwrap()` in library code — propagate errors properly
- Keep edge-node binary lean; it runs on constrained hardware

## Full Private Context

See `private/CLAUDE.md` for hardware specifics, internal architecture decisions, and detailed implementation notes. That file is gitignored and never committed.
