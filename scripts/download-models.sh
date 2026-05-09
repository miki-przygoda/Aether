#!/usr/bin/env bash
# Download brain-node model weights into ./models before docker compose up.
#
# Usage:
#   ./scripts/download-models.sh            # downloads to ./models/
#   ./scripts/download-models.sh /path/dir  # downloads to a custom directory
#
# Models downloaded (CPU defaults — GPU builds use the same weights):
#   Whisper medium GGUF       ~1.5 GB   primary STT model
#   distil-whisper-large-v3   ~1.5 GB   high-confidence STT fallback
#   Kokoro-82M ONNX           ~300 MB   TTS model
#   vocab.json                  ~4 KB   Kokoro phoneme → token-ID map
#   voice_style.bin             1024 B  256 × f32le default voice embedding
#
# The ./models directory is bind-mounted read-only into the brain-node container.
# Ollama model weights are pulled automatically by the ollama service on first boot.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODELS_DIR="${1:-"$REPO_ROOT/models"}"

mkdir -p "$MODELS_DIR/whisper" "$MODELS_DIR/tts"

WHISPER_BASE="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
KOKORO_HF="https://huggingface.co/hexgrad/Kokoro-82M/resolve/main"
# Community ONNX export of Kokoro-82M.  Verify the URL is current at:
#   https://huggingface.co/onnx-community/Kokoro-82M-v1.0
KOKORO_ONNX_URL="https://huggingface.co/onnx-community/Kokoro-82M-v1.0/resolve/main/onnx/model.onnx"

download() {
    local url="$1" dest="$2" label="$3"
    if [[ -f "$dest" ]]; then
        echo "  already present — skipping: $label"
        return
    fi
    echo "  downloading $label…"
    curl -fL --progress-bar "$url" -o "$dest"
}

# ── Whisper medium ────────────────────────────────────────────────────────────

echo "==> Whisper models"
download \
    "$WHISPER_BASE/ggml-medium.bin" \
    "$MODELS_DIR/whisper/ggml-medium.bin" \
    "Whisper medium (~1.5 GB)"

download \
    "$WHISPER_BASE/ggml-distil-large-v3.bin" \
    "$MODELS_DIR/whisper/distil-large-v3.bin" \
    "distil-whisper-large-v3 (~1.5 GB)"

# ── Kokoro-82M ────────────────────────────────────────────────────────────────

echo "==> Kokoro TTS model"
download \
    "$KOKORO_ONNX_URL" \
    "$MODELS_DIR/tts/kokoro-82m.onnx" \
    "Kokoro-82M ONNX (~300 MB)"

download \
    "$KOKORO_HF/vocab.json" \
    "$MODELS_DIR/tts/vocab.json" \
    "vocab.json"

# voice_style.bin: first 256 × f32le from the default American-English voice.
# The full voices/af_heart.bin is ~2 MB; we take only the first 1024 bytes
# (one style embedding) — that is all KokoroTts::new() expects.
STYLE_DEST="$MODELS_DIR/tts/voice_style.bin"
if [[ -f "$STYLE_DEST" ]]; then
    echo "  already present — skipping: voice_style.bin"
else
    echo "  downloading voice_style.bin (first 1024 bytes of af_heart.bin)…"
    curl -fsSL "$KOKORO_HF/voices/af_heart.bin" | head -c 1024 > "$STYLE_DEST"
    echo "  done"
fi

# ── Done ─────────────────────────────────────────────────────────────────────

echo ""
echo "All models ready in $MODELS_DIR"
echo ""
echo "Next steps:"
echo "  docker compose up --build"
echo ""
echo "For GPU (requires nvidia-container-toolkit):"
echo "  docker compose -f compose.yml -f compose.gpu.yml up --build"
