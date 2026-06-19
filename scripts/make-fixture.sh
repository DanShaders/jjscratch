#!/usr/bin/env bash
# Build the canonical jj test repo used by BOTH jjscratch (via jj-lib) and the
# lightjj reference harness (via the jj CLI). Committed into git so every
# worktree and the Chrome reference agent see byte-identical history.
#
# Re-run to regenerate from scratch. Deterministic-ish via JJ_RANDOMNESS_SEED.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JJ="$ROOT/tools/bin/jj"
REPO="$ROOT/fixture/repo"
CFG="$ROOT/fixture/jjconfig.toml"

export JJ_CONFIG="$CFG"
# NB: do NOT set JJ_RANDOMNESS_SEED — each jj subprocess would re-seed identically
# and every commit would get the same change_id (universal divergence).
export JJ_TIMESTAMP="2026-01-15T10:00:00+00:00"
export JJ_OP_TIMESTAMP="2026-01-15T10:00:00+00:00"
export JJ_USER="Ada Lovelace"
export JJ_EMAIL="ada@example.com"
export JJ_OP_HOSTNAME="fixture-host"
export JJ_OP_USERNAME="ada"

rm -rf "$REPO"
mkdir -p "$ROOT/fixture"

cat > "$CFG" <<'TOML'
[user]
name = "Ada Lovelace"
email = "ada@example.com"

[revset-aliases]
# Make the `main` bookmark and everything under it immutable (◆ in the UI).
# present() tolerates the bookmark not existing yet (e.g. before it's created).
"immutable_heads()" = "present(main) | tags()"

[ui]
color = "never"
TOML

# --no-colocate: keep the git backend inside .jj/ only. A colocated `.git` in the
# workdir would make the OUTER git repo treat fixture/repo as a submodule gitlink,
# so the actual files would NOT land in our history (breaks worktree parity).
"$JJ" git init --no-colocate "$REPO" >/dev/null
cd "$REPO"

commit() { "$JJ" commit -m "$1" >/dev/null; }
desc()   { "$JJ" describe -m "$1" >/dev/null; }

# --- immutable trunk history ---------------------------------------------
cat > README.md <<'EOF'
# demo

A small project used as a jjscratch fixture.
EOF
mkdir -p src
cat > src/main.rs <<'EOF'
fn main() {
    println!("hello");
}
EOF
commit "init project"

cat >> README.md <<'EOF'

## Building

Run `cargo build`.
EOF
commit "docs: add building section"

cat > src/parser.rs <<'EOF'
pub fn parse(input: &str) -> Vec<&str> {
    input.split_whitespace().collect()
}
EOF
cat > src/main.rs <<'EOF'
mod parser;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tokens = parser::parse(&args.join(" "));
    println!("{} tokens", tokens.len());
}
EOF
commit "feat: add whitespace parser"

# main bookmark marks the immutable frontier.
"$JJ" bookmark create main -r @- >/dev/null

# --- mutable stack on top of main ----------------------------------------
cat > src/parser.rs <<'EOF'
/// Split input into tokens, collapsing repeated whitespace.
pub fn parse(input: &str) -> Vec<&str> {
    input.split_whitespace().filter(|s| !s.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits() {
        assert_eq!(parse("a  b"), vec!["a", "b"]);
    }
}
EOF
commit "refactor: document and test parser"

# wip commit continues the main stack.
cat > src/cli.rs <<'EOF'
pub struct Flags {
    pub verbose: bool,
}
EOF
commit "wip: cli flags"

# The working-copy commit (@) carries live edits, so the diff panel has content.
# In jj the working copy IS a commit; these edits show as @'s diff vs its parent.
cat >> src/main.rs <<'EOF'

// scratch: trying out a new entry point
fn run() {}
EOF
rm -f src/parser.rs   # a deletion shown in @
WC_CHANGE="$("$JJ" log --no-pager --no-graph -r @ -T 'change_id.short()')"

# A side branch (experiment bookmark) off main. `jj new` snapshots @'s edits
# first, so returning with `jj edit` preserves them.
"$JJ" new main -m "experiment: alternative tokenizer" >/dev/null
cat > src/tokenizer.rs <<'EOF'
pub struct Tokenizer<'a> { rest: &'a str }

impl<'a> Tokenizer<'a> {
    pub fn new(s: &'a str) -> Self { Self { rest: s } }
}
EOF
"$JJ" bookmark create experiment -r @ >/dev/null

# Return @ to the main-stack working copy (with its live edits).
"$JJ" edit "$WC_CHANGE" >/dev/null

echo "=== final graph ==="
"$JJ" log --no-pager -r 'all()' || true
echo "fixture built at $REPO"
