#!/usr/bin/env bash
# Train the rustpotter wake word model from collected WAV samples.
#
# Prerequisites:
#   1. Run scripts/download-models.sh to download Kokoro TTS weights.
#   2. Run `brain-node generate-wake-word-samples` to create synthetic samples.
#   3. Optionally add real recordings to models/wake-word/samples/real/.
#   4. Install rustpotter-cli:  cargo install rustpotter-cli
#
# Usage:
#   ./scripts/train-wake-word.sh
#   SAMPLES_DIR=/path/to/samples OUTPUT=my-model.rpw ./scripts/train-wake-word.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SAMPLES_DIR="${SAMPLES_DIR:-"$REPO_ROOT/models/wake-word/samples"}"
OUTPUT="${OUTPUT:-"$REPO_ROOT/models/wake-word/hey-aether-baseline.rpw"}"
WAKEWORD="${WAKEWORD:-"Hey Aether"}"

if ! command -v rustpotter-cli &>/dev/null; then
    echo "rustpotter-cli not found — installing…"
    cargo install rustpotter-cli
fi

# Collect all WAV samples from synthetic + real subdirectories.
mapfile -t samples < <(find "$SAMPLES_DIR" -name "*.wav" 2>/dev/null | sort)

if [[ ${#samples[@]} -eq 0 ]]; then
    echo "No WAV samples found in $SAMPLES_DIR"
    echo ""
    echo "Run the following first:"
    echo "  brain-node generate-wake-word-samples \\"
    echo "    --kokoro-model models/tts/kokoro-82m.onnx"
    exit 1
fi

echo "Training rustpotter model on ${#samples[@]} samples…"
echo "  Wake word: $WAKEWORD"
echo "  Output:    $OUTPUT"
echo ""

rustpotter-cli train \
    --wakeword "$WAKEWORD" \
    --output "$OUTPUT" \
    "${samples[@]}"

echo ""
echo "Model written to $OUTPUT"
echo ""
echo "Test it with:"
echo "  rustpotter-cli test --model $OUTPUT --threshold 0.5"
echo ""
echo "Deploy by copying to the edge node and setting AETHER_MODEL_PATH=$OUTPUT"
