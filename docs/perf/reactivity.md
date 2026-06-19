# Real-time reactivity & performance

How `jjscratch` re-renders when the underlying jj repo changes on disk, the
measured cost of doing so, where the time goes, the gap to the 16.6 ms (60 Hz)
frame budget, and the concrete plan to close it.

The **budget** is end-to-end "decide what to draw" — repo reload + revset/graph
(Snapshot) + diff materialize + `ui::build_scene` — for an *arbitrary next
state*. GPU rasterization is separate; this doc is about the CPU work that
produces the `Scene`.

> All numbers below are from `src/bin/perf_harness.rs` and `src/bin/churn_test.rs`
> run **`--release`** on this environment (`llvmpipe` CPU, no GPU). Reproduce:
> ```
> cargo run --release --features jjlib --bin perf-harness
> PERF_LOG_N=600 cargo run --release --features jjlib --bin perf-harness large-log
> CHURN_SECS=5 CHURN_WRITERS=4 cargo run --release --features jjlib --bin churn-test
> ```

---

## 1. Architecture

```
.jj/ (op log + working-copy state)
   │  recursive notify watch
   ▼
Watcher  ──coalesce (40ms debounce, 250ms cap, latest-wins, bounded)──▶  Tick
   │
   ▼
ReactiveReloader.reload()
   ├─ cheap gate: op heads on disk == loaded op id?  ──yes──▶ skip (≈0)
   ├─ load_at_head()           (reload)
   ├─ jjlib::snapshot()        (revset eval + stream_graph + per-node build)
   ├─ DiffCache.get_or_compute (diff materialize, cached by commit_id)
   └─ ui::build_scene()        (scene construction)
```

* **Watch** (`src/watch.rs`): `notify` recursive watch of `.jj/`. jj records
  every state transition as an *operation* under `.jj/`, so watching it catches
  commits, rebases, working-copy snapshots, bookmark moves, and undo.
* **Coalesce**: a debounce thread collapses a burst into one `Tick`. State is
  **O(1)** — just the burst's earliest instant + a sequence counter, never a
  queue. The output channel is **latest-wins**: consumers call `latest_pending()`
  to skip straight to the newest tick, so a slow consumer never builds a backlog.
* **Cheap gate** (the first cheap win, landed): before doing any real work,
  `op_heads_on_disk()` reads the op-heads pointer (no repo load) and compares it
  to the op id we're already loaded at. Equal ⇒ nothing we draw can have changed
  ⇒ skip everything. Under churn this absorbs the vast majority of events.
* **Diff cache** (the second cheap win, landed): `DiffCache` keys `CommitDiff`s
  by `commit_id`. A commit's contents are immutable once written, so a diff is
  valid for the life of the process regardless of how the op log moves. Bounded
  LRU (cap 64). Re-rendering an unchanged selection is an `Arc` clone.

---

## 2. Measured latencies

### Tiny change (1 file edit + 1 `jj describe`) — n=25

| stage         | p50      | p99      | mean     |
|---------------|----------|----------|----------|
| **end-to-end**| **0.74ms** | **1.78ms** | 0.84ms |
| reload        | 0.13ms   | 0.42ms   | 0.14ms   |
| snapshot      | 0.28ms   | 1.17ms   | 0.34ms   |
| diff          | 0.08ms   | 0.11ms   | 0.08ms   |
| build_scene   | 0.25ms   | 1.23ms   | 0.28ms   |

**Verdict: WITHIN budget (0.1× the frame).** The common case is a non-issue.

### Huge diff (working-copy commit: 20k-line file + 400 files; 10,400 +/- lines) — n=25

| stage         | p50         | p99         | mean       |
|---------------|-------------|-------------|------------|
| **end-to-end**| **1753ms**  | **2147ms**  | 1794ms     |
| reload        | 0.15ms      | 0.24ms      | 0.15ms     |
| snapshot      | 0.36ms      | 1.05ms      | 0.37ms     |
| **diff**      | **1751ms**  | **2144ms**  | 1791ms     |
| build_scene   | 1.99ms      | 6.34ms      | 2.18ms     |

**Verdict: OVER budget (129× the frame).** The entire cost is **diff
materialize**. `build_scene` over the same 10,400 lines is only ~2 ms.

### Large log (600 commits → 602 visible rows) — n=25

| stage         | p50      | p99      | mean     |
|---------------|----------|----------|----------|
| **end-to-end**| **14.9ms** | **17.3ms** | 15.1ms |
| reload        | 0.17ms   | 0.29ms   | 0.18ms   |
| **snapshot**  | **12.3ms** | **13.8ms** | 12.3ms |
| diff          | 1.83ms   | 4.50ms   | 2.09ms   |
| build_scene   | 0.45ms   | 1.11ms   | 0.51ms   |

**Verdict: at the edge (≈1.0× the frame at p99).** The cost is **snapshot**
(revset + per-node build), scaling ~linearly with row count (≈20 µs/row).

### Windowing micro-benchmark (203 graph rows)

| build_scene        | p50      |
|--------------------|----------|
| @ viewport (800px) | 0.28ms   |
| @ full (4054px)    | 0.80ms   |
| **off-screen**     | **0.52ms (65% of full build)** |

`build_scene` builds the **whole** graph list (and, for diffs, all hunk lines).
65% of the work at 203 rows is off-screen — and it grows linearly, so at
thousands of rows / hundreds of thousands of diff lines it dominates. It is small
in absolute terms today only because the inputs are; O(viewport) windowing makes
it O(1) in input size.

---

## 3. Churn resilience (measured)

`churn-test`: 4 writer threads, 5 s, mixing rapid file writes + concurrent
`jj describe` ops, while the watcher + reload loop runs.

```
raw fs events observed : 33,614
file writes issued     : 5,598
jj ops issued          : 11        (concurrent jj ops contend on the op lock)
ticks consumed         : 20
reloads performed      : 1   (skipped, op-unchanged: 19)
max event->reload lag  : 250.87ms  (== the max_defer cap, by design)
max single reload cost : 0.77ms
diff-cache size        : 1         (bounded; cap 64)
recovery               : 0 real reloads post-churn, 39 no-op ticks gated ✓
```

* **Coalescing**: 33,614 raw fs events collapsed to 20 ticks and **1** real
  reload — a ~33,000× reduction. The cheap op-id gate absorbed 19 of 20 ticks.
* **Bounded memory**: debounce state is O(1); the diff cache is capped. Nothing
  grows with churn volume.
* **Bounded lag**: worst event→reload lag is 251 ms, exactly the `max_defer`
  cap — under a continuous storm the watcher still forces a tick on schedule so
  the consumer is never starved.
* **Recovery**: when churn stops the op log stops moving; the loop converges to
  zero real reloads (residual `.jj/` touches become cheap gated no-ops). No
  deadlock, no backlog.

The loop never deadlocks or grows unboundedly. ✓

---

## 4. The gap to 16.6 ms, and the plan

| scenario   | p99 today | budget | gap        | dominant stage |
|------------|-----------|--------|------------|----------------|
| tiny       | 1.8ms     | 16.6ms | ✅ 0.1×    | — |
| large-log  | 17.3ms    | 16.6ms | ⚠️ ~1.0×   | **snapshot** |
| huge-diff  | 2147ms    | 16.6ms | ❌ 129×    | **diff** |

`reload` (`load_at_head`) is **never** a bottleneck (≤0.4 ms everywhere): the
stores stay warm across reloads, so re-resolving the head op is cheap. The cheap
op-id gate already removes it from the hot path entirely when nothing changed.

### Biggest bottleneck: diff materialize (the huge-diff case, 129× over)

The diff stage in `model/jjlib.rs` does two expensive things per file:

1. **Reads & UTF-8-splits both full blobs** into `Vec<String>` line vectors.
2. **`diff_lines`** runs a **classic O(n·m) LCS DP with an `O(n·m)` `Vec<Vec<u32>>`
   table** (`src/model/jjlib.rs::diff_lines`). For a 20k-line file that is
   ~4×10⁸ cells and ~1.6 GB of transient allocation per file — this single
   function is essentially the whole 1.7 s.

**Plan (in priority order):**

1. **Replace the O(n·m) LCS with a linear-space Myers diff** (or pull in
   `similar`/`imara-diff`). Myers is O((n+m)·d) where d = edit distance, and
   linear memory. For typical diffs this is 10–100× faster and bounded memory.
   *This is the single highest-impact change* and lives entirely in the file I
   own (`model/jjlib.rs`). Estimated: huge-diff drops from ~1.7 s toward tens of
   ms.
2. **Cap / lazily materialize per-file diffs.** Most "huge diffs" are huge
   because of *many* files or a few *enormous* ones. For a file over N lines (or
   N bytes), show a collapsed "large change — N±" summary and compute the hunks
   only when that file is expanded. Combined with the existing `DiffCache`, only
   the *visible, expanded* files ever pay.
3. **Off-thread the diff.** Diff materialize is pure and cacheable. Run it on a
   worker; render the graph immediately with a "diff loading" placeholder and
   swap in the diff when ready. This keeps the frame responsive even when a
   single file is genuinely enormous. (Requires a worker in the render files,
   which this round does not own — flagged as a follow-up.)

### Second bottleneck: snapshot build (the large-log case, ~1.0× at 600 rows)

`jjlib::snapshot()` does per-node work that scales with row count (≈20 µs/row):
`get_commit`, two `shortest_unique_*_prefix_len` calls, two `containing_fn`
membership probes, and `bookmarks_at`. At ~800+ rows this exceeds the frame.

**Plan:**

1. **Viewport windowing of the Snapshot itself.** The graph list is the obvious
   candidate: build `CommitNode`s only for the visible window (+ a small
   overscan), streaming the revset lazily and stopping past the viewport. The
   graph topology/lane layout for off-screen rows is not needed until scrolled
   to. This makes snapshot O(viewport), not O(history).
2. **Cache per-commit node metadata by `commit_id`.** Like the diff cache:
   change/commit prefix lengths and bookmarks for a commit are stable for the op
   (prefix lengths can shift as the visible set grows, but are cheap to refresh
   lazily). Reuse across reloads so only *new* rows are built.
3. **Avoid the two full-history membership maps.** `collect_membership` evaluates
   immutable/divergent over *all* ids; restrict to the visible window.

### Third: build_scene off-screen work (windowing)

`ui::build_scene` (and `ui/graph.rs` / `ui/diff.rs`) currently build the **whole**
graph list and **all** diff lines regardless of scroll. The windowing benchmark
shows 65% of build at 203 rows is off-screen, growing linearly.

**Recommendation for the render files (not owned this round):** make the graph
and diff renderers **O(viewport)** — given `graph_scroll` / `diff_scroll` and the
panel height, compute the first/last visible row (fixed 18 px rows make this
exact arithmetic) and build only `[first-overscan, last+overscan]`. This caps
build_scene cost independent of input size and is the natural partner to the
Snapshot windowing in §4.2. The harness's `windowing` scenario can be reused to
verify the speedup once landed.

---

## 5. Cheap wins already landed (this round)

* **Op-id skip gate** — `ReactiveReloader::reload()` reads the op-heads pointer
  cheaply (no repo load) and skips reload+snapshot+diff+scene entirely when the
  op id is unchanged. In the churn test this turned 33,614 fs events into a
  single real reload. (`src/watch.rs`, `src/model/jjlib.rs::op_heads_on_disk`.)
* **Per-commit diff cache** — `DiffCache` reuses a `CommitDiff` by `commit_id`
  (immutable once written) across reloads, bounded LRU. The reactive reload pays
  the diff cost once per distinct selection, not once per tick.
* **Reuse the open `Workspace` on reload** — `reload_at_head()` re-resolves only
  the head op against the already-open workspace (warm stores), keeping reload
  at ≤0.4 ms instead of paying a fresh `Workspace::load` per change.
* **Bounded, latest-wins coalescing** — keeps the app responsive and memory flat
  under arbitrary fs churn (the core requirement).

The path to <16.6 ms for every scenario is clear and lives mostly in code we own
or have flagged: **(1) Myers diff + lazy/large-file capping** closes the huge-diff
gap; **(2) viewport windowing of the Snapshot and of `build_scene`** closes the
large-log gap and caps everything at O(viewport).
