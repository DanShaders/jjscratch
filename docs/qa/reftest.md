# Golden-image reftest suite

A self-rendering UI regression guard for jjscratch. It re-renders a fixed set of
UI states and compares them to jjscratch's **own** committed golden PNGs under
`tests/golden/`. It does **not** compare against lightjj, and it needs no Chrome,
no network, and (in the default build) no `jjlib`. It is deterministic and fast.

> **Re-bless after the in-flight UI changes land.** These goldens were captured
> from the **current** rendering. The main session will RE-BLESS them once the
> in-flight UI-fix + perf changes are in, so the committed goldens reflect the
> **final** UI. Re-blessing is a single command (see *Bless* below) — run it,
> eyeball the PNG diff in `git diff`, and commit.

## Files

- `tests/reftest.rs` — the harness (the `cargo test` target). Owns the state
  registry, the in-process render path, the subprocess preview path, the
  pixel-compare, and the bless/check driver.
- `tests/golden/*.png` — the committed golden images (one per state, 1280x800).
- `target/reftest-out/` — scratch output on failure (git-ignored via `/target`).

No `src/`, `Cargo.toml`, or render-code edits: the harness is a normal
integration test auto-discovered from `tests/`, and the preview views are
captured by running their existing `preview_*` bins.

## States covered (9)

Rendered at a fixed **1280x800 @1x** (logical == device). @1x is chosen for
speed; these states are layout/color regressions, not sub-pixel hinting, so @1x
has teeth without the 4x readback cost of @2x.

**Integrated states** — rendered *in-process* through the real library path
(`jjscratch::ui::build_scene`), driven by replaying the same key tokens that
`src/bin/shot.rs` accepts via `JJSCRATCH_KEYS`, against the fixture-matching
`mock` data. No subprocess, no `jjlib`.

| Golden        | Keys      | What it shows                                  |
|---------------|-----------|------------------------------------------------|
| `revisions`   | (none)    | default Revisions view, working-copy selected  |
| `diff_nav`    | `j j`     | selection moved down two rows                  |
| `branches`    | `2`       | Branches view                                  |
| `oplog`       | `4`       | Oplog bottom drawer open                       |
| `light`       | `t`       | light theme                                    |
| `palette`     | `ctrl+k`  | command-palette overlay                        |

**Isolated preview views** — captured by *shelling out* to the existing
`preview_*` bins and reading the PNG they write. These renderers
(`merge_view`, `evolog_view`, `split_diff_view`) live as **bin-local modules**
of their `preview_*` bins, so a `tests/` crate cannot `use` them directly. Rather
than refactor render code into the library (out of scope / not owned), the suite
runs the bin as a subprocess — the cleanest way to snapshot a bin-local view.

| Golden           | Bin              | What it shows                       |
|------------------|------------------|-------------------------------------|
| `preview_merge`  | `preview_merge`  | Merge view (conflict rail + 3-pane) |
| `preview_evolog` | `preview_evolog` | Evolog bottom drawer                |
| `preview_split`  | `preview_split`  | side-by-side (split) diff           |

There is no `preview_palette` bin; the palette overlay is an integrated state and
is covered in-process via `ctrl+k` above.

### Feature-specific golden: `oplog`

The Oplog drawer is the one state whose chrome legitimately differs between the
two build configs: the default (mock) build draws a "No operations (mock build)"
placeholder, while the `jjlib` build runs the real `oplog` renderer (which shows
"No operations" for the empty fixture op-log). One image can't satisfy both, so
the `oplog` state's golden is **feature-suffixed**: `oplog.png` for the default
build, `oplog_jjlib.png` for `--features jjlib`. Every other state renders
identically in both configs and shares one golden. `BLESS` writes whichever
variant matches the build it runs in (so re-blessing must be done once per config
if the oplog drawer changes — see *Bless* below).

## Determinism finding + tolerance

lavapipe (the software Vulkan ICD this environment pins) is **almost but not
quite bit-exact**. Re-rendering the same scene flips at most a **single LSB**
(≤1 on any RGBA channel) on a handful of pixels — a readback/accumulation
rounding difference, both across processes and within one process. Measured over
many runs of every state: **max per-channel delta = 1, and ZERO pixels ever
differ by more than 1.**

So the compare uses a small tolerance rather than exact byte-equality:

- `MAX_CHANNEL_DELTA = 2` — a pixel only "differs" if some channel is off by
  **more than 2**. The observed 1-LSB jitter is therefore invisible to CHECK.
- `MAX_MISMATCH_FRACTION = 0.0005` (0.05%) — a state fails only if the fraction
  of differing pixels exceeds this. In practice the differing-pixel count is
  always **0**, so the headroom is enormous (a real regression moves thousands
  of pixels — the sanity check below flips 3.7%).

The `determinism` test renders two representative states twice each and asserts
both that no pixel exceeds the CHECK tolerance **and** that the raw max channel
delta stays ≤ 1. If a future driver introduces larger drift, that test fails
loudly, signalling the tolerance needs revisiting before the goldens flake.

## Commands

Run with `--test-threads=1` so the two GPU-using tests don't contend for the
software adapter.

**CHECK** (the regression guard — this is what CI / `cargo test` runs):

```bash
cargo test --test reftest -- --test-threads=1
# or just: cargo test            # runs reftest alongside the unit tests
```

On mismatch the failing state prints a clear message and writes
`target/reftest-out/<state>.actual.png`, `<state>.golden.png`, and a magenta
`<state>.diff.png` (changed pixels in magenta over a dimmed grayscale base).

**BLESS** (regenerate every golden from the current render — the single command
to re-capture after intended UI changes):

```bash
REFTEST_BLESS=1 cargo test --test reftest -- --test-threads=1 golden_states
```

That writes the default-build goldens (including `oplog.png`). The only golden
that also needs the `jjlib` variant is the oplog drawer; refresh it with:

```bash
REFTEST_BLESS=1 cargo test --features jjlib --test reftest -- --test-threads=1 golden_states
```

(This re-renders every state under `jjlib`; only `oplog_jjlib.png` is unique to
that config. Because lavapipe jitters ±1 LSB, the other goldens get rewritten
with imperceptible noise — discard those with `git checkout -- tests/golden`,
keeping just `oplog_jjlib.png`, or simply run the default bless last.)

Then review `git diff -- tests/golden/` (or open the PNGs) and commit.

## Proving it has teeth (sanity)

Perturb any render input and CHECK must fail. Example: temporarily change the
`diff_nav` state's keys from `"j j"` to `"j j j"` in `states()` — moving the
selected row by one. Running CHECK then fails `diff_nav` with **3.74% of pixels
differing (max channel delta 240)**, far above the 0.05% threshold, and writes
the actual/golden/diff PNGs. Restore the keys and CHECK is green again. This was
verified when the suite was built.

## Notes / gotchas

- **`jjlib` not required.** Integrated states use `mock` data, so the suite runs
  in the default (fast) build. It also passes under `--features jjlib` (the
  feature only adds an unused oplog field to `Frame`; the mock build renders the
  drawer's empty/placeholder state identically).
- **`.gitignore`.** Root has `/*.png` (root-level only) and `/target`, so
  `tests/golden/*.png` are tracked and `target/reftest-out/` is ignored.
- **Adding a state.** Add a row to `states()` in `tests/reftest.rs` (a `Keys`
  variant for an integrated state, a `Preview` variant for a new `preview_*`
  bin), then BLESS.
