# Graph / branch differential stress test vs the `jj` oracle

A randomized differential test that verifies jjscratch builds the revision **log
graph** and **bookmark (branch) markings** the same way the real `jj` CLI does.

## What it does

Two new pieces (neither touches the renderer/UI):

- **`scripts/gen-random-repos.sh`** — generates N random jj repos, one per integer
  SEED, under a target dir (default `/tmp/jjstress`). Every choice is drawn from a
  deterministic xorshift64 PRNG seeded by the integer seed, so a failing repo
  reproduces exactly by re-running its seed. Each repo gets
  `jj git init --no-colocate` plus a `JJ_CONFIG` mirroring `fixture/jjconfig.toml`
  (`immutable_heads() = present(main) | tags()`), then a random 5–40 op sequence
  drawn from: `commit` (with a file edit), `jj new` (branch off a random commit),
  `jj new A B` (merge of two random commits), `jj bookmark create`, `jj describe`,
  `jj abandon` (a random mutable leaf), creating the immutable `main` bookmark
  partway through, and manufacturing **divergence** (rewriting one mutable commit
  two ways via `jj describe --at-op <prior-op>` so the two results share a change
  id). Ops that can't apply are skipped rather than aborting the repo.

- **`src/bin/graphstress.rs`** (auto-discovered bin, `--features jjlib`) — for each
  generated repo: loads it in-process via `jjlib::open` + `jjlib::snapshot`, builds
  `graph_layout::layout`, queries the `jj` CLI for ground truth via
  `std::process::Command` (absolute-path binary), and asserts the invariants below.
  Prints `PASS`/`FAIL` per repo plus an aggregate; on `FAIL` it dumps the seed,
  repo path, the specific invariant, and expected/actual so it reproduces.

## Run

```bash
cargo build --features jjlib --bin graphstress
scripts/gen-random-repos.sh /tmp/jjstress $(seq 1 50)
cargo run --features jjlib --bin graphstress -- /tmp/jjstress
```

## Invariants checked (per repo)

1. **Node set & order** — jjscratch's snapshot `commit_id`s, in order, equal the
   oracle's default-revset commit ids **in graph order**. (See the note below: the
   oracle must come from `jj log` *with* the graph, not `--no-graph`.)
2. **Bookmarks** — for every node, jjscratch's bookmark name set equals the
   oracle's `bookmarks.map(|b| b.name())`. Also checked from the oracle side: every
   oracle bookmark lands on the right commit, none missing/extra.
3. **Flags** — `is_working_copy == current_working_copy`,
   `is_immutable == immutable`, `is_divergent == divergent` (all via templates).
4. **change_id encoding** — jjscratch's reverse-hex `change_id` equals the oracle
   `change_id` template (which prints the same reverse-hex form).
5. **graph_layout validity** — rows map 1:1 to nodes in order; each node's own
   gutter cell is a pipe/elided marker (never empty/elbow); node columns are within
   `width_cols`; no two nodes collide at a (row, node-col); every **Direct** parent
   edge references a node present in the snapshot (edges to filtered-out commits are
   `Indirect`/`Missing`, never dangling `Direct`). The layout never panics. (We do
   NOT require jj's exact ASCII lane choice — only internal consistency.)
6. **Render non-degenerate** — each repo is rendered through the real jjscratch UI
   path (`ui::build_scene` → `Headless::render`) and the frame is asserted to be
   more than a single solid color, catching render panics/blank frames.

## Results

Ran **50 repos / seeds (1–50)**. Coverage across the set:

- 599 total visible default-revset rows (largest single repo: 34 rows).
- 79 merge commits across 31 repos.
- 302 divergent commits across 33 repos.
- 179 bookmarks across the set, plus immutable `main` trunks and elided
  (`Indirect`) ancestor edges.

**Final result: 50 / 50 PASS — all invariants hold across all 50 repos/seeds.**

The harness was verified to have teeth: temporarily corrupting the oracle
`change_id` field made every repo `FAIL` with a precise expected/actual dump, as
expected.

## Bugs found & fixed

**None in `src/graph_layout.rs` or `src/model/jjlib.rs`.** Across 50 random repos
exercising linear chains, multiple branches, merges, many bookmarks, immutable
trunks, and heavy divergence, jjscratch's node set/order, bookmark placement,
flags, change-id encoding, and layout were all consistent with the `jj` oracle.

A focused regression test was added to `src/graph_layout.rs`
(`merge_with_elided_parent_is_consistent`, `linear_and_fork_shapes_stay_consistent`,
and the shared `assert_layout_consistent` checker) codifying the layout invariants
the stress harness asserts, so future layout regressions are caught by `cargo test`.

## Important finding / deviation from the original spec

Invariant 1's oracle must use **`jj log` *with* the graph**, not `jj log
--no-graph`. jjscratch's `snapshot()` orders nodes with
`TopoGroupedGraph::prioritize_branch(@)` — the same DFS branch-grouping the `jj`
CLI applies only in **graph** mode. `jj log --no-graph` emits the *raw* revset
order (heads interleaved by index position), which differs (e.g. on a divergent
repo the `--no-graph` head order and the graph order disagree in the first rows).
Comparing against `--no-graph` would therefore produce **false** order failures
even though jjscratch is correct. `graphstress` emits a `\x01`-delimited,
`\x1f`-separated record per node from the **graph** log and parses only the real
records (graph-art and `(elided revisions)` lines carry no `\x01`), so the order
oracle matches jjscratch's intended behavior. This is a deliberate, documented
deviation — jjscratch matches `jj log` (default, with graph), which is the correct
target.

## Known limitations

- The default-log revset (`present(@) | ancestors(immutable_heads().., 2) |
  trunk()`) only shows the working copy, mutable heads, and two generations of
  immutable ancestors; deeper immutable history is intentionally elided, so a few
  short-op-sequence seeds collapse to 2–4 visible rows. This is correct behavior
  and the harness verifies it matches jj exactly.
- Bookmark `conflicted`/`unsynced` markers and remote-tracking bookmarks are not
  specifically stressed (the generator creates only local bookmarks); name/placement
  equality is fully checked.
- The render check only asserts non-degeneracy (not a pixel match) — it catches
  panics/blank frames, not subtle visual regressions.
