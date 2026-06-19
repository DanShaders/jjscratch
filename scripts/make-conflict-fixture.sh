#!/usr/bin/env bash
# Build a SECOND jj fixture repo that contains a real, unresolved merge conflict,
# used only to capture lightjj's Merge view (`3`) / ConflictQueue reference shot.
#
# The shared fixture (fixture/repo) is deliberately conflict-free, so the merge
# view there is empty. This repo creates a genuine 2-sided conflict the way jj
# records one: two sibling commits rewrite the SAME line of the SAME file off a
# common base, then `jj new A B` auto-merges them into a working-copy merge commit
# that jj marks `(conflict)`. lightjj's `switchToMergeView` revset
# (`conflicts() & mutable()`) then surfaces it.
#
# Built non-colocated (only .jj/, no workdir .git) like make-fixture.sh, so the
# outer git stores it as plain blobs rather than a submodule gitlink.
#
# The jj binary lives in the SHARED checkout's tools/ (worktrees don't get one);
# JJ can be overridden via the JJ env var.
#
# Re-run to regenerate from scratch (deterministic timestamps/author).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Prefer this checkout's tools/bin/jj; fall back to the shared checkout (the one
# that actually holds the downloaded toolchains) when running from a worktree.
JJ="${JJ:-$ROOT/tools/bin/jj}"
[ -x "$JJ" ] || JJ="/home/danklishch/work/jjscratch/tools/bin/jj"
REPO="$ROOT/fixture-conflict/repo"
CFG="$ROOT/fixture-conflict/jjconfig.toml"

# Deterministic identity/timestamps so the repo (and thus the reference shot)
# is byte-stable across regenerations. Do NOT set JJ_RANDOMNESS_SEED (would make
# every commit share one change_id).
export JJ_TIMESTAMP="2026-01-15T10:00:00+00:00"
export JJ_OP_TIMESTAMP="2026-01-15T10:00:00+00:00"
export JJ_USER="Ada Lovelace"
export JJ_EMAIL="ada@example.com"
export JJ_OP_HOSTNAME="fixture-host"
export JJ_OP_USERNAME="ada"

rm -rf "$REPO"
mkdir -p "$ROOT/fixture-conflict"

# Same config as the shared fixture: main is the immutable frontier.
cat > "$CFG" <<'TOML'
[user]
name = "Ada Lovelace"
email = "ada@example.com"

[revset-aliases]
# Make the `main` bookmark and everything under it immutable (◆ in the UI).
"immutable_heads()" = "present(main) | tags()"

[ui]
color = "never"
TOML
export JJ_CONFIG="$CFG"

# --no-colocate: keep the git backend inside .jj/ only (see make-fixture.sh).
"$JJ" git init --no-colocate "$REPO" >/dev/null
cd "$REPO"

# --- common base on the immutable trunk ----------------------------------
printf 'line one\nline two\nline three\n' > greeting.txt
"$JJ" commit -m "base: add greeting" >/dev/null
"$JJ" bookmark create main -r @- >/dev/null

# --- branch A: rewrite line two -------------------------------------------
printf 'line one\nFEATURE A change\nline three\n' > greeting.txt
"$JJ" describe -m "feature-a: rewrite line two" >/dev/null
"$JJ" bookmark create feature-a -r @ >/dev/null

# --- branch B (sibling off main): rewrite the SAME line differently -------
"$JJ" new main -m "feature-b: rewrite line two" >/dev/null
printf 'line one\nFEATURE B change\nline three\n' > greeting.txt
"$JJ" bookmark create feature-b -r @ >/dev/null

# --- merge the divergent siblings -> recorded conflict --------------------
# `jj new A B` auto-merges; the same-line edits cannot reconcile, so jj records
# a 2-sided conflict in the new working-copy merge commit (@).
"$JJ" new feature-a feature-b -m "merge: combine features" >/dev/null

echo "=== final graph ==="
"$JJ" log --no-pager -r 'all()' || true
echo "=== status (should show greeting.txt 2-sided conflict) ==="
"$JJ" status --no-pager || true
echo "conflict fixture built at $REPO"
