# Wake Word Training Guide

Aether uses [rustpotter](https://github.com/GiviMAD/rustpotter) for wake word detection — pure Rust, no external account, trainable on your own voice. The shipped baseline model targets "Hey Aether"; this guide explains how to train or retrain it.

## Prerequisites

| Tool | Install |
|---|---|
| Kokoro TTS model | `./scripts/download-models.sh` |
| rustpotter CLI | `cargo install rustpotter-cli` |
| espeak-ng | `apt install espeak-ng` / `brew install espeak` |

## Step 1 — Generate synthetic samples

The `generate-wake-word-samples` subcommand uses Kokoro TTS to produce WAV samples of the wake word at five different speeds. This gives the model variation in pace without requiring any recordings.

```bash
# Build the brain-node binary first (or use the Docker image).
cargo build --release -p brain-node

./target/release/brain-node generate-wake-word-samples \
  --kokoro-model models/tts/kokoro-82m.onnx \
  --output-dir  models/wake-word/samples/synthetic \
  --phrase      "Hey Aether" \
  --count       5
```

This writes 25 WAV files (5 speeds × 5 samples each) into `models/wake-word/samples/synthetic/`.

## Step 2 — Add real recordings (optional but recommended)

Place your own WAV recordings of "Hey Aether" in `models/wake-word/samples/real/`. Each file should be:

- **Format:** 16-bit PCM WAV, mono, 16 kHz
- **Duration:** 1–2 seconds (just the phrase, no long silence)
- **Variety:** different speakers, distances from mic, and background noise levels improve accuracy

Record yourself and ask a few people with different voices and accents. 10–20 real samples alongside the synthetics typically gives a solid model.

Convert any recordings that aren't already 16 kHz mono:

```bash
ffmpeg -i input.wav -ar 16000 -ac 1 -sample_fmt s16 models/wake-word/samples/real/my_sample_01.wav
```

## Step 3 — Train the model

```bash
./scripts/train-wake-word.sh
```

This collects all WAVs from `models/wake-word/samples/` (synthetic + real), trains a rustpotter model, and writes it to `models/wake-word/hey-aether-baseline.rpw`.

To train with custom paths:

```bash
SAMPLES_DIR=/path/to/samples \
OUTPUT=models/wake-word/my-model.rpw \
WAKEWORD="Hey Aether" \
./scripts/train-wake-word.sh
```

## Step 4 — Test the model

```bash
rustpotter-cli test \
  --model models/wake-word/hey-aether-baseline.rpw \
  --threshold 0.5
```

Speak "Hey Aether" into your microphone. A score above the threshold triggers detection. Adjust `--threshold` to trade off false positives vs. missed detections:

| Threshold | Effect |
|---|---|
| 0.3–0.4 | More sensitive — fewer misses, more false triggers |
| 0.5 | Balanced (recommended starting point) |
| 0.6–0.7 | More precise — fewer false triggers, may miss quiet speech |

## Step 5 — Deploy to the edge node

Copy the trained model to the Pi and set the path in your environment:

```bash
scp models/wake-word/hey-aether-baseline.rpw pi@<pi-address>:~/.config/aether/

# On the Pi, set the env var or use the --model-path flag:
AETHER_MODEL_PATH=~/.config/aether/hey-aether-baseline.rpw edge-node run
```

The `deploy-edge.sh` script copies the binary; copy the model alongside it.

## Retraining with your own voice

If the baseline model triggers too often or misses your voice:

1. Record 15–20 samples of yourself saying "Hey Aether" clearly.
2. Place them in `models/wake-word/samples/real/`.
3. Re-run `./scripts/train-wake-word.sh` — it blends all samples automatically.
4. Re-deploy to the Pi.

Keeping the synthetic samples in the mix prevents the model from overfitting to a single voice.
