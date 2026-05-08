#!/usr/bin/env bash
# Deploy edge-node binary to a Pi over SSH.
#
# Required env vars:
#   AETHER_PI_HOST   — SSH host (user@hostname or user@ip)
#   AETHER_PI_ARCH   — cross target: aarch64-unknown-linux-gnu (default) or
#                      armv7-unknown-linux-gnueabihf
#
# Usage:
#   AETHER_PI_HOST=pi@raspberrypi.local ./scripts/deploy-edge.sh

set -euo pipefail

TARGET="${AETHER_PI_ARCH:-aarch64-unknown-linux-gnu}"
BINARY="target/${TARGET}/release/edge-node"

if [[ -z "${AETHER_PI_HOST:-}" ]]; then
    echo "error: AETHER_PI_HOST is not set" >&2
    echo "  example: AETHER_PI_HOST=pi@raspberrypi.local $0" >&2
    exit 1
fi

if [[ ! -f "$BINARY" ]]; then
    echo "Binary not found at $BINARY — building now..."
    cross build --release -p edge-node --target "$TARGET"
fi

echo "Deploying $BINARY → ${AETHER_PI_HOST}:/usr/local/bin/edge-node"
scp "$BINARY" "${AETHER_PI_HOST}:/tmp/edge-node-new"
ssh "$AETHER_PI_HOST" 'sudo mv /tmp/edge-node-new /usr/local/bin/edge-node && sudo chmod +x /usr/local/bin/edge-node'
echo "Done."
