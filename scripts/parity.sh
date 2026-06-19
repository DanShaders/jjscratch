#!/usr/bin/env bash
# parity.sh — pixel-parity harness for jjscratch vs the real lightjj reference.
#
# Builds the `shot` binary (with the jjlib feature), renders the real fixture at
# @2x (2560x1600, matching the reference's deviceScaleFactor 2), then scores the
# render against docs/reference/<scene>.png and prints a per-region table.
#
# Usage:
#   scripts/parity.sh [scene]          # default scene: revisions
#   scripts/parity.sh branches
#
# Env overrides:
#   THRESHOLD=40   per-channel hard-mismatch threshold (0-255)
#   RADIUS=1       AA-tolerance neighborhood radius (px)
#
# Idempotent: rebuilds only if needed and overwrites its own output PNGs.
set -euo pipefail

SCENE="${1:-revisions}"
THRESHOLD="${THRESHOLD:-40}"
RADIUS="${RADIUS:-1}"

# Resolve repo root from this script's location (works from any cwd).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

REFERENCE="docs/reference/${SCENE}.png"
CANDIDATE="docs/parity/jjscratch-${SCENE}.png"
FIXTURE="$ROOT/fixture/repo"

if [[ ! -f "$REFERENCE" ]]; then
    echo "ERROR: no reference image for scene '$SCENE' at $REFERENCE" >&2
    echo "       available references:" >&2
    ls docs/reference/*.png 2>/dev/null | sed 's/^/         /' >&2 || true
    exit 1
fi
if [[ ! -d "$FIXTURE" ]]; then
    echo "ERROR: fixture repo not found at $FIXTURE" >&2
    exit 1
fi

mkdir -p docs/parity

echo "==> building shot (--features jjlib)"
cargo build --quiet --features jjlib --bin shot

echo "==> rendering jjscratch real fixture @2x -> $CANDIDATE"
# Logical 1280x800, scale 2 -> 2560x1600 device px, comparable to the reference.
JJSCRATCH_REPO="$FIXTURE" \
    cargo run --quiet --features jjlib --bin shot -- "$CANDIDATE" 1280 800 --scale 2

echo "==> scoring vs $REFERENCE"
uv run --quiet "$ROOT/tools/parity/compare.py" \
    --candidate "$CANDIDATE" \
    --reference "$REFERENCE" \
    --out-dir docs/parity \
    --scene "$SCENE" \
    --scale 2 \
    --threshold "$THRESHOLD" \
    --radius "$RADIUS"
