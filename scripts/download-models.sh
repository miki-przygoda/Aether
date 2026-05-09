#!/usr/bin/env bash
# Download brain-node model weights into ./models before docker compose up.
#
# Usage:
#   ./scripts/download-models.sh            # downloads to ./models/
#   ./scripts/download-models.sh /path/dir  # downloads to a custom directory
#
# Models downloaded (CPU defaults -- GPU builds use the same weights):
#   Whisper medium GGUF       ~1.5 GB   primary STT model
#   Whisper large-v3-turbo    ~1.5 GB   high-confidence STT fallback
#   Kokoro-82M ONNX (v0.19)   ~325 MB   TTS model (kokoro-v0_19.onnx -> tts/kokoro-82m.onnx)
#   vocab.json                  ~3 KB   Kokoro phoneme -> token-ID map (from kokoro-onnx config)
#   voice_style.bin             1024 B  256 x f32le default voice embedding (af voice, t=0)
#
# The ./models directory is bind-mounted read-only into the brain-node container.
# Ollama model weights are pulled automatically by the ollama service on first boot.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODELS_DIR="${1:-"$REPO_ROOT/models"}"

mkdir -p "$MODELS_DIR/whisper" "$MODELS_DIR/tts"

WHISPER_BASE="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"
# kokoro-onnx GitHub releases -- public, no HuggingFace auth required.
export KOKORO_RELEASE="https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files"
export KOKORO_CONFIG_URL="https://raw.githubusercontent.com/thewh1teagle/kokoro-onnx/main/src/kokoro_onnx/config.json"
export MODELS_DIR

download() {
    local url="$1" dest="$2" label="$3"
    if [[ -f "$dest" ]]; then
        echo "  already present -- skipping: $label"
        return
    fi
    echo "  downloading $label..."
    curl -fL --progress-bar "$url" -o "$dest"
}

# -- Whisper medium -----------------------------------------------------------

echo "==> Whisper models"
download \
    "$WHISPER_BASE/ggml-medium.bin" \
    "$MODELS_DIR/whisper/ggml-medium.bin" \
    "Whisper medium (~1.5 GB)"

download \
    "$WHISPER_BASE/ggml-large-v3-turbo.bin" \
    "$MODELS_DIR/whisper/large-v3-turbo.bin" \
    "Whisper large-v3-turbo fallback (~1.5 GB)"

# -- Kokoro-82M ---------------------------------------------------------------

echo "==> Kokoro TTS model"
download \
    "$KOKORO_RELEASE/kokoro-v0_19.onnx" \
    "$MODELS_DIR/tts/kokoro-82m.onnx" \
    "Kokoro-82M ONNX v0.19 (~325 MB)"

# vocab.json -- phoneme char -> token ID map, extracted from kokoro-onnx config.
VOCAB_DEST="$MODELS_DIR/tts/vocab.json"
if [[ -f "$VOCAB_DEST" ]]; then
    echo "  already present -- skipping: vocab.json"
else
    echo "  downloading vocab.json..."
    python3 - <<'PYEOF'
import urllib.request, json, os, sys
url = os.environ.get("KOKORO_CONFIG_URL")
dest = os.path.join(os.environ["MODELS_DIR"], "tts", "vocab.json")
with urllib.request.urlopen(url) as r:
    config = json.load(r)
vocab = config["vocab"]
with open(dest, "w") as f:
    json.dump(vocab, f)
print(f"  vocab.json written ({len(vocab)} entries)")
PYEOF
fi

# voice_style.bin -- 256 x f32le default style embedding (af voice, first timestep).
# Extracted from the first 20 KB of voices.json via an HTTP range request.
STYLE_DEST="$MODELS_DIR/tts/voice_style.bin"
if [[ -f "$STYLE_DEST" ]]; then
    echo "  already present -- skipping: voice_style.bin"
else
    echo "  extracting voice_style.bin from voices.json..."
    python3 - <<'PYEOF'
import urllib.request, json, struct, os, re
url = os.environ["KOKORO_RELEASE"] + "/voices.json"
dest = os.path.join(os.environ["MODELS_DIR"], "tts", "voice_style.bin")
# Range request -- we only need the first af voice embedding (~7 KB of JSON)
req = urllib.request.Request(url, headers={"Range": "bytes=0-20479"})
with urllib.request.urlopen(req) as r:
    chunk = r.read().decode(errors="replace")
m = re.search(r'"af"\s*:\s*\[\s*\[\s*\[([^\]]+)', chunk)
if not m:
    print("ERROR: could not parse af voice from voices.json", file=__import__("sys").stderr)
    raise SystemExit(1)
floats = [float(x.strip()) for x in m.group(1).split(",") if x.strip()][:256]
if len(floats) != 256:
    print(f"ERROR: expected 256 floats, got {len(floats)}", file=__import__("sys").stderr)
    raise SystemExit(1)
data = struct.pack("<256f", *floats)
with open(dest, "wb") as f:
    f.write(data)
print(f"  voice_style.bin written ({len(data)} bytes)")
PYEOF
fi

# -- Done ---------------------------------------------------------------------

echo ""
echo "All models ready in $MODELS_DIR"
echo ""
echo "Next steps:"
echo "  docker compose up --build"
echo ""
echo "For GPU (requires nvidia-container-toolkit):"
echo "  docker compose -f compose.yml -f compose.gpu.yml up --build"
