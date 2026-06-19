#!/usr/bin/env bash
# compare-interaction.sh — cross-driver INTERACTION parity harness.
#
# Replays ONE shared interaction script (see docs/parity/interaction/nav.txt) on
# BOTH apps and scores each paired step:
#   * jjscratch: native `drive --features jjlib` binary renders step-NN-<name>.png
#     against the SHARED fixture repo (Vello @2x = 2560x1600).
#   * lightjj:   real web app driven through headless Chrome by
#     tools/parity/drive-lightjj.mjs, same script, same viewport (@2x).
# Then tools/parity/compare.py scores each paired step-NN, writing per-step
# heatmaps + side-by-sides + a score table into docs/parity/interaction/.
#
# Usage:
#   scripts/compare-interaction.sh [script]        # default: docs/parity/interaction/nav.txt
#   PORT=3011 scripts/compare-interaction.sh <script>
#
# Idempotent; cleanly starts/stops lightjj (reuses reference.sh's trap/cleanup)
# and leaves no lingering processes.
set -euo pipefail

# ---- paths -----------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"            # this worktree's root

SCRIPT_FILE="${1:-$ROOT/docs/parity/interaction/nav.txt}"
# Allow a bare relative path from the repo root too.
if [[ ! -f "$SCRIPT_FILE" && -f "$ROOT/$SCRIPT_FILE" ]]; then
  SCRIPT_FILE="$ROOT/$SCRIPT_FILE"
fi

# The downloaded toolchains (lightjj/jj/node/chrome-deps/refharness) live under
# the MAIN checkout's tools/, which is gitignored and therefore absent from this
# worktree. Use it directly. Override with TOOLS=... if your layout differs.
TOOLS="${TOOLS:-/home/danklishch/work/jjscratch/tools}"
FIXTURE_REPO="$ROOT/fixture/repo"
JJCONFIG="$ROOT/fixture/jjconfig.toml"

OUT_DIR="$ROOT/docs/parity/interaction"
JJS_OUT="$OUT_DIR/jjscratch"
LJ_OUT="$OUT_DIR/lightjj"

NODE_BIN="$TOOLS/node/bin"
LIGHTJJ_BIN="$TOOLS/bin/lightjj"
JJ_BIN_DIR="$TOOLS/bin"
DRIVE_LIGHTJJ="$ROOT/tools/parity/drive-lightjj.mjs"
COMPARE_PY="$TOOLS/parity/compare.py"
# compare.py is committed in this worktree too; prefer the worktree copy.
[[ -f "$ROOT/tools/parity/compare.py" ]] && COMPARE_PY="$ROOT/tools/parity/compare.py"

PORT="${PORT:-3017}"
ADDR="localhost:${PORT}"
URL="http://${ADDR}"

# ---- env (mirror reference.sh) ---------------------------------------------
export PATH="$JJ_BIN_DIR:$NODE_BIN:$PATH"
export JJ_CONFIG="$JJCONFIG"
CHROME_LIBS="$TOOLS/chrome-deps/root/usr/lib/x86_64-linux-gnu"
if [ -d "$CHROME_LIBS" ]; then
  export LD_LIBRARY_PATH="$CHROME_LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

# ---- preflight -------------------------------------------------------------
[ -f "$SCRIPT_FILE" ]   || { echo "ERROR: interaction script not found: $SCRIPT_FILE"; exit 1; }
[ -x "$LIGHTJJ_BIN" ]   || { echo "ERROR: lightjj binary missing at $LIGHTJJ_BIN"; exit 1; }
[ -x "$JJ_BIN_DIR/jj" ] || { echo "ERROR: jj binary missing at $JJ_BIN_DIR/jj"; exit 1; }
[ -x "$NODE_BIN/node" ] || { echo "ERROR: node missing at $NODE_BIN/node"; exit 1; }
[ -d "$FIXTURE_REPO" ]  || { echo "ERROR: fixture repo missing at $FIXTURE_REPO"; exit 1; }
[ -f "$DRIVE_LIGHTJJ" ] || { echo "ERROR: lightjj driver missing at $DRIVE_LIGHTJJ"; exit 1; }
[ -f "$COMPARE_PY" ]    || { echo "ERROR: comparator missing at $COMPARE_PY"; exit 1; }
[ -d "$TOOLS/refharness/node_modules/puppeteer-core" ] || {
  echo "ERROR: puppeteer-core not installed in $TOOLS/refharness"; exit 1; }
command -v uv >/dev/null 2>&1 || { echo "ERROR: uv not found (needed to run compare.py)"; exit 1; }

mkdir -p "$JJS_OUT" "$LJ_OUT"
# Idempotency: clear prior step PNGs so a removed `shot` can't leave stragglers.
rm -f "$JJS_OUT"/step-*.png "$LJ_OUT"/step-*.png "$OUT_DIR"/heatmap-step-*.png "$OUT_DIR"/sidebyside-step-*.png

echo "==> interaction script: $SCRIPT_FILE"

# ---- (1) jjscratch native driver -------------------------------------------
echo
echo "==> building jjscratch drive (--features jjlib)"
( cd "$ROOT" && cargo build --quiet --features jjlib --bin drive )

echo "==> driving jjscratch over the script (@2x)"
JJSCRATCH_REPO="$FIXTURE_REPO" \
  "$ROOT/target/debug/drive" "$SCRIPT_FILE" "$JJS_OUT" 1280 800 --scale 2

# ---- (2) lightjj: launch + drive (reuse reference.sh launch/cleanup) --------
kill_port() {
  local pids
  pids="$(pgrep -f "lightjj .*--addr ${ADDR}" 2>/dev/null || true)"
  if [ -n "$pids" ]; then
    echo "[ix] killing stale lightjj on ${ADDR}: $pids"
    # shellcheck disable=SC2086
    kill $pids 2>/dev/null || true
    sleep 0.5
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
}
kill_port

LOG="$(mktemp -t lightjj-ix.XXXXXX.log)"
echo
echo "==> starting lightjj on ${URL} (log: $LOG)"
"$LIGHTJJ_BIN" -R "$FIXTURE_REPO" --addr "$ADDR" --no-browser >"$LOG" 2>&1 &
LJ_PID=$!

cleanup() {
  local code=$?
  if kill -0 "$LJ_PID" 2>/dev/null; then
    echo "[ix] stopping lightjj (pid $LJ_PID)"
    kill "$LJ_PID" 2>/dev/null || true
    wait "$LJ_PID" 2>/dev/null || true
  fi
  kill_port
  exit $code
}
trap cleanup EXIT INT TERM

echo "==> waiting for ${URL} ..."
ready=0
for _ in $(seq 1 60); do
  if ! kill -0 "$LJ_PID" 2>/dev/null; then
    echo "[ix] ERROR: lightjj exited early. Log:"; cat "$LOG"; exit 1
  fi
  if curl -fsS "$URL/" >/dev/null 2>&1; then ready=1; break; fi
  sleep 0.3
done
[ "$ready" = 1 ] || { echo "[ix] ERROR: lightjj did not become ready. Log:"; cat "$LOG"; exit 1; }
echo "[ix] lightjj is serving."

echo "==> driving lightjj over the SAME script (@2x)"
export NODE_PATH="$TOOLS/refharness/node_modules"
node "$DRIVE_LIGHTJJ" --url "$URL" --script "$SCRIPT_FILE" --out "$LJ_OUT"

# ---- (3) score each paired step --------------------------------------------
echo
echo "==> scoring paired steps"
declare -a NAMES SCORES
fail=0
for cand in "$JJS_OUT"/step-*.png; do
  [ -e "$cand" ] || continue
  base="$(basename "$cand")"            # step-NN-name.png
  ref="$LJ_OUT/$base"
  step="${base%.png}"                   # step-NN-name
  if [[ ! -f "$ref" ]]; then
    echo "  [skip] no lightjj counterpart for $base"
    continue
  fi
  echo
  echo "----- $step -----"
  # compare.py writes heatmap-<scene>.png / sidebyside-<scene>.png; scene=$step.
  out="$(uv run --quiet "$COMPARE_PY" \
      --candidate "$cand" \
      --reference "$ref" \
      --out-dir "$OUT_DIR" \
      --scene "$step" \
      --scale 2 \
      --threshold "${THRESHOLD:-40}" \
      --radius "${RADIUS:-1}")" || { echo "$out"; fail=1; continue; }
  echo "$out"
  overall="$(printf '%s\n' "$out" | awk '/OVERALL/ {gsub(/%/,"",$2); print $2}')"
  NAMES+=("$step")
  SCORES+=("$overall")
done

# ---- (4) summary table ------------------------------------------------------
echo
echo "================================================================"
echo "  INTERACTION PARITY SUMMARY   script=$(basename "$SCRIPT_FILE")"
echo "  jjscratch: $JJS_OUT"
echo "  lightjj:   $LJ_OUT"
echo "  artefacts: $OUT_DIR/heatmap-step-*.png"
echo "----------------------------------------------------------------"
printf "  %-26s %12s\n" "step" "overall %"
echo "----------------------------------------------------------------"
for i in "${!NAMES[@]}"; do
  printf "  %-26s %11s%%\n" "${NAMES[$i]}" "${SCORES[$i]}"
done
echo "================================================================"

exit $fail
