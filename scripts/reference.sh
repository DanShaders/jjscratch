#!/usr/bin/env bash
# Orchestrate reference-image capture against the REAL lightjj app.
#
# Starts lightjj headlessly against the SHARED fixture repo, waits for it to
# serve, runs the puppeteer screenshot script, then tears lightjj down. It is
# idempotent and leaves no lingering processes (kills any prior instance on the
# port, and cleans up its own on exit / interrupt).
#
# Usage:
#   scripts/reference.sh                 # capture all scenes
#   PORT=3011 scripts/reference.sh       # custom port
#   scripts/reference.sh --scene branches
#
# All downloaded toolchains live under tools/ and are put on PATH here, so this
# works in a headless environment with no system Go/Node.
set -euo pipefail

# ---- paths -----------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"           # /home/danklishch/work/jjscratch

# Toolchains (jj/node/lightjj/chrome-deps/refharness) only ever live in the
# SHARED checkout's tools/ — git worktrees don't get their own copy. Prefer this
# checkout's tools/ if it actually holds the lightjj binary; otherwise fall back
# to the shared checkout. Overridable via $TOOLS.
TOOLS="${TOOLS:-$ROOT/tools}"
if [ ! -x "$TOOLS/bin/lightjj" ]; then
  TOOLS="/home/danklishch/work/jjscratch/tools"
fi

# Fixture is per-checkout (committed), so it stays relative to $ROOT. Both the
# repo and its jj config are overridable so the merge scene can point lightjj at
# the conflict fixture (fixture-conflict/) instead of the shared one.
FIXTURE_REPO="${FIXTURE_REPO:-$ROOT/fixture/repo}"
JJCONFIG="${JJCONFIG:-$ROOT/fixture/jjconfig.toml}"
OUT_DIR="$ROOT/docs/reference"

# Local toolchains (downloaded into tools/, never system-wide).
NODE_BIN="$TOOLS/node/bin"
LIGHTJJ_BIN="$TOOLS/bin/lightjj"
JJ_BIN_DIR="$TOOLS/bin"                          # jj 0.42 lives here

PORT="${PORT:-3007}"
ADDR="localhost:${PORT}"
URL="http://${ADDR}"

# ---- env -------------------------------------------------------------------
# jj binary (0.42) + fixture config on PATH/env so the captured UI matches what
# jjscratch will load (same commits, immutable markers, bookmarks).
export PATH="$JJ_BIN_DIR:$NODE_BIN:$PATH"
export JJ_CONFIG="$JJCONFIG"

# The cached puppeteer Chrome needs a handful of system shared libraries
# (libnss3, libatk, libcups, libpango, ...) that are not installed system-wide
# in this headless environment. They were extracted from Ubuntu .deb packages
# into tools/chrome-deps/root (no sudo/apt-install). Put them on the loader path.
CHROME_LIBS="$TOOLS/chrome-deps/root/usr/lib/x86_64-linux-gnu"
if [ -d "$CHROME_LIBS" ]; then
  export LD_LIBRARY_PATH="$CHROME_LIBS${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

# ---- preflight -------------------------------------------------------------
[ -x "$LIGHTJJ_BIN" ]   || { echo "ERROR: lightjj binary missing at $LIGHTJJ_BIN (build it; see docs/reference/README.md)"; exit 1; }
[ -x "$JJ_BIN_DIR/jj" ] || { echo "ERROR: jj binary missing at $JJ_BIN_DIR/jj"; exit 1; }
[ -x "$NODE_BIN/node" ] || { echo "ERROR: node missing at $NODE_BIN/node"; exit 1; }
[ -d "$FIXTURE_REPO" ]  || { echo "ERROR: fixture repo missing at $FIXTURE_REPO"; exit 1; }
[ -d "$TOOLS/refharness/node_modules/puppeteer-core" ] || {
  echo "ERROR: puppeteer-core not installed in $TOOLS/refharness (run: cd $TOOLS/refharness && npm install)"; exit 1; }

mkdir -p "$OUT_DIR"

# ---- kill any stale lightjj on this port (idempotency) ---------------------
kill_port() {
  local pids
  pids="$(pgrep -f "lightjj .*--addr ${ADDR}" 2>/dev/null || true)"
  if [ -n "$pids" ]; then
    echo "[ref] killing stale lightjj on ${ADDR}: $pids"
    # shellcheck disable=SC2086
    kill $pids 2>/dev/null || true
    sleep 0.5
    # shellcheck disable=SC2086
    kill -9 $pids 2>/dev/null || true
  fi
}
kill_port

# ---- start lightjj ---------------------------------------------------------
LOG="$(mktemp -t lightjj-ref.XXXXXX.log)"
echo "[ref] starting lightjj on ${URL} (log: $LOG)"
"$LIGHTJJ_BIN" -R "$FIXTURE_REPO" --addr "$ADDR" --no-browser >"$LOG" 2>&1 &
LJ_PID=$!

cleanup() {
  local code=$?
  if kill -0 "$LJ_PID" 2>/dev/null; then
    echo "[ref] stopping lightjj (pid $LJ_PID)"
    kill "$LJ_PID" 2>/dev/null || true
    wait "$LJ_PID" 2>/dev/null || true
  fi
  kill_port  # belt-and-suspenders: nothing should linger
  exit $code
}
trap cleanup EXIT INT TERM

# ---- wait for readiness ----------------------------------------------------
echo "[ref] waiting for ${URL} ..."
ready=0
for _ in $(seq 1 60); do
  if ! kill -0 "$LJ_PID" 2>/dev/null; then
    echo "[ref] ERROR: lightjj exited early. Log:"; cat "$LOG"; exit 1
  fi
  if curl -fsS "$URL/" >/dev/null 2>&1; then ready=1; break; fi
  sleep 0.3
done
[ "$ready" = 1 ] || { echo "[ref] ERROR: lightjj did not become ready. Log:"; cat "$LOG"; exit 1; }
echo "[ref] lightjj is serving."

# ---- capture ---------------------------------------------------------------
# puppeteer-core lives in the local refharness package, not next to the script.
export NODE_PATH="$TOOLS/refharness/node_modules"
echo "[ref] capturing reference screenshots ..."
node "$SCRIPT_DIR/lightjj-shot.mjs" --url "$URL" --out "$OUT_DIR" "$@"
shot_rc=$?

echo "[ref] done. Images in: $OUT_DIR"
ls -la "$OUT_DIR"/*.png 2>/dev/null || true

exit $shot_rc
