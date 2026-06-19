#!/usr/bin/env bash
# scripts/sizecheck.sh — High-DPI + window-size robustness harness.
#
# Renders a matrix of (logical width x height x scale x state) via src/bin/shot.rs
# against the real fixture repo, then runs tools/sizecheck/check.py over each PNG
# to flag layout breakage. Prints a PASS/WARN/FAIL table and writes the same
# results into docs/qa/. Rerunnable: it regenerates everything from scratch.
#
# Usage:  scripts/sizecheck.sh
# Output: docs/qa/*.png  (one per matrix cell)
#         table on stdout; non-zero exit if any cell FAILs.
set -uo pipefail

# Resolve repo root from this script's location (worktree-safe).
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

OUT="docs/qa"
mkdir -p "$OUT"
export JJSCRATCH_REPO="$ROOT/fixture/repo"

echo "Building shot (jjlib)…"
cargo build --features jjlib --bin shot 2>&1 | tail -1
SHOT="$ROOT/target/debug/shot"

# Matrix rows: "name W H SCALE KEYS THEME".
#   KEYS  = value for JJSCRATCH_KEYS ("-" = none)
#   THEME = dark|light (must match KEYS; "t" flips to light)
MATRIX=(
  # --- size/DPI sweep (Revisions, dark) ---
  "tiny_640x480@1            640  480  1  -  dark"
  "small_800x600@1          800  600  1  -  dark"
  "default_1280x800@1      1280  800  1  -  dark"
  "default_1280x800@2      1280  800  2  -  dark"
  "default_1280x800@3      1280  800  3  -  dark"
  "tallnarrow_600x900@2     600  900  2  -  dark"
  "wide_2560x1080@2        2560 1080  2  -  dark"
  "huge_3440x1440@2        3440 1440  2  -  dark"
  # --- awkward / stress ---
  "belowmin_500x400@1       500  400  1  -  dark"
  "veryshort_1000x300@2    1000  300  2  -  dark"
  # --- key states @ 1280x800@2 ---
  "revisions_1280x800@2    1280  800  2  1  dark"
  "branches_1280x800@2     1280  800  2  2  dark"
  "oplog_1280x800@2        1280  800  2  4  dark"
  "light_1280x800@2        1280  800  2  t  light"
  # --- key states @ 800x600@1 ---
  "revisions_800x600@1      800  600  1  1  dark"
  "branches_800x600@1       800  600  1  2  dark"
  "oplog_800x600@1          800  600  1  4  dark"
  "light_800x600@1          800  600  1  t  light"
)

printf '\n%-26s %-12s %-5s %-6s %-6s %s\n' "CELL" "SIZE" "SCALE" "STATE" "RESULT" "NOTES"
printf '%s\n' "------------------------------------------------------------------------------------------------"

PASS=0; WARN=0; FAIL=0
RESULTS_TSV="$OUT/results.tsv"
: > "$RESULTS_TSV"

for row in "${MATRIX[@]}"; do
  # shellcheck disable=SC2086
  set -- $row
  NAME="$1"; W="$2"; H="$3"; SCALE="$4"; KEYS="$5"; THEME="$6"
  PNG="$OUT/${NAME}.png"

  if [ "$KEYS" = "-" ]; then
    JJSCRATCH_KEYS="" "$SHOT" "$PNG" "$W" "$H" "$SCALE" >/dev/null 2>&1
  else
    JJSCRATCH_KEYS="$KEYS" "$SHOT" "$PNG" "$W" "$H" "$SCALE" >/dev/null 2>&1
  fi
  RC=$?

  STATE="${KEYS}"
  [ "$KEYS" = "-" ] && STATE="def"

  if [ $RC -ne 0 ] || [ ! -s "$PNG" ]; then
    RESULT="FAIL"; NOTES="shot exited $RC / no PNG (PANIC?)"
    FAIL=$((FAIL+1))
  else
    JSON="$(uv run --quiet "$ROOT/tools/sizecheck/check.py" "$PNG" --scale "$SCALE" --theme "$THEME" --json 2>/dev/null)"
    RESULT="$(printf '%s' "$JSON" | python3 -c 'import sys,json;print(json.load(sys.stdin)["status"])' 2>/dev/null)"
    NOTES="$(printf '%s' "$JSON" | python3 -c 'import sys,json;print(" | ".join(json.load(sys.stdin)["messages"]))' 2>/dev/null)"
    case "$RESULT" in
      PASS) PASS=$((PASS+1));;
      WARN) WARN=$((WARN+1));;
      *)    RESULT="${RESULT:-FAIL}"; FAIL=$((FAIL+1));;
    esac
  fi

  printf '%-26s %-12s @%-4s %-6s %-6s %s\n' "$NAME" "${W}x${H}" "$SCALE" "$STATE" "$RESULT" "${NOTES:0:90}"
  printf '%s\t%dx%d\t@%s\t%s\t%s\t%s\n' "$NAME" "$W" "$H" "$SCALE" "$STATE" "$RESULT" "$NOTES" >> "$RESULTS_TSV"
done

echo
echo "Summary: PASS=$PASS  WARN=$WARN  FAIL=$FAIL  (total ${#MATRIX[@]})"
echo "Screenshots + results.tsv in $OUT/"
[ $FAIL -eq 0 ]
