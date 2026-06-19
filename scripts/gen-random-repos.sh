#!/usr/bin/env bash
# Generate N random jj repositories for differential stress-testing jjscratch's
# log/graph + bookmark rendering against the real `jj` CLI oracle.
#
# Each repo is deterministic in its SEED: every random choice is drawn from a
# tiny bash LCG seeded by the integer seed, so a failing repo reproduces exactly
# by re-running with that seed.
#
# Usage:
#   scripts/gen-random-repos.sh [TARGET_DIR] [SEED...]
#   SEEDS="1 2 3" scripts/gen-random-repos.sh /tmp/jjstress
#
# Defaults: TARGET_DIR=/tmp/jjstress, seeds 1..30 if none given.
#
# Each generated repo lands at  $TARGET_DIR/repo-<seed>/  and gets its own
# jjconfig.toml (mirroring fixture/jjconfig.toml) written to
# $TARGET_DIR/repo-<seed>.jjconfig.toml. graphstress reads both.
set -uo pipefail

# jj lives at this absolute path; the worktree has no local tools/bin.
JJ="/home/danklishch/work/jjscratch/tools/bin/jj"

TARGET_DIR="${1:-/tmp/jjstress}"
shift || true

# Seed list: explicit args, then $SEEDS env, then default 1..30.
SEED_ARGS=("$@")
if [[ ${#SEED_ARGS[@]} -eq 0 && -n "${SEEDS:-}" ]]; then
  # shellcheck disable=SC2206
  SEED_ARGS=(${SEEDS})
fi
if [[ ${#SEED_ARGS[@]} -eq 0 ]]; then
  # shellcheck disable=SC2207
  SEED_ARGS=($(seq 1 30))
fi

mkdir -p "$TARGET_DIR"

# ---------------------------------------------------------------------------
# Deterministic PRNG (xorshift64 + multiply mix). RAND_STATE/RND are global.
# ---------------------------------------------------------------------------
RAND_STATE=0
RND=0
# rnd <bound>  -> sets global $RND to a value in [0,bound) and advances state.
# IMPORTANT: this mutates globals, so it must NOT be called via $(...) (a
# command substitution forks a subshell and the advanced state would be lost).
rnd() {
  local bound="$1"
  # xorshift64 — good bit distribution across all windows, unlike a bare LCG
  # whose low/high bits mod small bounds cluster badly. Bash math is 64-bit
  # signed; mask to 63 bits to stay positive.
  local x=$RAND_STATE
  x=$(( x ^ (x << 13) ))
  x=$(( (x ^ (x >> 7)) & 0x7FFFFFFFFFFFFFFF ))
  x=$(( x ^ (x << 17) ))
  RAND_STATE=$(( x & 0x7FFFFFFFFFFFFFFF ))
  # multiply-mix then take high bits for the reduction.
  local out=$(( (RAND_STATE * 2685821657736338717) & 0x7FFFFFFFFFFFFFFF ))
  RND=$(( (out >> 33) % bound ))
}
# Seed the state with a non-zero, well-mixed value derived from the seed.
seed_rng() {
  RAND_STATE=$(( ( ($1 + 1) * 2654435761 ) & 0x7FFFFFFFFFFFFFFF ))
  (( RAND_STATE == 0 )) && RAND_STATE=88172645463325252
  rnd 2; rnd 2; rnd 2   # warm up
}

# Fixed-but-advancing timestamps so generated history is reproducible.
TS_COUNTER=0
bump_ts() {
  TS_COUNTER=$((TS_COUNTER + 1))
  local total=$((10 + TS_COUNTER))
  local hh=$((10 + total / 60))
  local mm=$((total % 60))
  printf '2026-03-01T%02d:%02d:00+00:00' "$hh" "$mm"
}

gen_one() {
  local seed="$1"
  local repo="$TARGET_DIR/repo-$seed"
  local cfg="$TARGET_DIR/repo-$seed.jjconfig.toml"

  rm -rf "$repo"
  seed_rng "$seed"
  TS_COUNTER=0

  cat > "$cfg" <<'TOML'
[user]
name = "Stress Tester"
email = "stress@example.com"

[revset-aliases]
"immutable_heads()" = "present(main) | tags()"

[ui]
color = "never"
TOML

  export JJ_CONFIG="$cfg"
  export JJ_USER="Stress Tester"
  export JJ_EMAIL="stress@example.com"
  export JJ_OP_HOSTNAME="stress-host"
  export JJ_OP_USERNAME="stress"

  "$JJ" git init --no-colocate "$repo" >/dev/null 2>&1 || { echo "  init failed for seed $seed" >&2; return 1; }
  cd "$repo" || return 1

  # Visible non-root commit ids -> CIDS.
  refresh_cids() {
    mapfile -t CIDS < <("$JJ" log -r 'all() ~ root()' --no-graph -T 'commit_id ++ "\n"' 2>/dev/null)
  }
  # Mutable head commit ids -> MLEAVES.
  refresh_mleaves() {
    mapfile -t MLEAVES < <("$JJ" log -r 'heads(all()) ~ immutable()' --no-graph -T 'commit_id ++ "\n"' 2>/dev/null)
  }
  # Mutable non-root, non-@ commit ids -> MUTABLE (safe rewrite/abandon targets).
  refresh_mutable() {
    mapfile -t MUTABLE < <("$JJ" log -r '(all() ~ root() ~ @) ~ immutable()' --no-graph -T 'commit_id ++ "\n"' 2>/dev/null)
  }

  local nops bk_counter file_counter main_created i
  rnd 36; nops=$(( 5 + RND ))   # 5..40
  bk_counter=0
  file_counter=0
  main_created=0

  for (( i=0; i<nops; i++ )); do
    JJ_TIMESTAMP="$(bump_ts)"; export JJ_TIMESTAMP
    JJ_OP_TIMESTAMP="$JJ_TIMESTAMP"; export JJ_OP_TIMESTAMP
    refresh_cids
    local ncids=${#CIDS[@]}

    # Around the halfway point, create the immutable `main` bookmark. Target an
    # EARLY commit (lower in history) so most of the tree stays mutable/visible
    # rather than collapsing under the 2-generation immutable-ancestors limit.
    if [[ $main_created -eq 0 && $i -ge $((nops / 2)) && $ncids -ge 2 ]]; then
      # pick from the oldest third of commits (jj log is newest-first, so tail).
      local third=$(( (ncids + 2) / 3 ))
      (( third < 1 )) && third=1
      rnd "$third"
      local pick=$(( ncids - 1 - RND ))
      (( pick < 0 )) && pick=0
      local mr=${CIDS[$pick]}
      "$JJ" bookmark create main -r "$mr" >/dev/null 2>&1 && main_created=1
      continue
    fi

    rnd 100
    local op=$RND

    if   (( op < 30 )); then
      # commit: edit a file, then commit.
      file_counter=$((file_counter + 1))
      local f="file_${file_counter}.txt"
      printf 'line %d for seed %d op %d\n' "$file_counter" "$seed" "$i" >> "$f"
      "$JJ" commit -m "commit op $i (seed $seed)" >/dev/null 2>&1 || true

    elif (( op < 47 )); then
      # jj new: branch off a random existing commit.
      if (( ncids > 0 )); then
        rnd "$ncids"; local r=${CIDS[$RND]}
        "$JJ" new "$r" >/dev/null 2>&1 || true
      fi

    elif (( op < 60 )); then
      # jj new A B: merge of two random distinct commits.
      if (( ncids >= 2 )); then
        local a b
        rnd "$ncids"; a=${CIDS[$RND]}
        rnd "$ncids"; b=${CIDS[$RND]}
        if [[ "$a" != "$b" ]]; then
          "$JJ" new "$a" "$b" >/dev/null 2>&1 || true
        fi
      fi

    elif (( op < 73 )); then
      # bookmark create on a random rev.
      if (( ncids > 0 )); then
        bk_counter=$((bk_counter + 1))
        rnd "$ncids"; local r=${CIDS[$RND]}
        "$JJ" bookmark create "bk${bk_counter}" -r "$r" >/dev/null 2>&1 || true
      fi

    elif (( op < 81 )); then
      # describe the working copy.
      "$JJ" describe -m "described at op $i (seed $seed)" >/dev/null 2>&1 || true

    elif (( op < 89 )); then
      # abandon a random mutable leaf.
      refresh_mleaves
      local nl=${#MLEAVES[@]}
      if (( nl > 0 )); then
        rnd "$nl"; local r=${MLEAVES[$RND]}
        "$JJ" abandon "$r" >/dev/null 2>&1 || true
      fi

    else
      # divergence: rewrite the SAME mutable commit two ways from two operations
      # so the two results share a change id but differ -> divergent. This is the
      # reliable jj way to manufacture divergence (`--at-op` on the prior op).
      refresh_mutable
      local nm=${#MUTABLE[@]}
      if (( nm > 0 )); then
        rnd "$nm"; local r=${MUTABLE[$RND]}
        local opid
        opid=$("$JJ" op log --no-graph -T 'id ++ "\n"' 2>/dev/null | head -1)
        "$JJ" describe "$r" -m "diverge A op $i" >/dev/null 2>&1 || true
        if [[ -n "$opid" ]]; then
          "$JJ" describe "$r" -m "diverge B op $i" --at-op "$opid" >/dev/null 2>&1 || true
        fi
      fi
    fi
  done

  local rcount
  rcount=$("$JJ" log --no-graph -T '"x\n"' 2>/dev/null | wc -l)
  echo "  seed $seed -> $repo (${nops} ops, ${rcount} visible rows)"
  return 0
}

echo "Generating ${#SEED_ARGS[@]} repos into $TARGET_DIR ..."
for s in "${SEED_ARGS[@]}"; do
  ( gen_one "$s" ) || echo "  WARN: seed $s generation had errors (continuing)"
done
echo "Done. Repos under $TARGET_DIR (config alongside each as repo-<seed>.jjconfig.toml)."
