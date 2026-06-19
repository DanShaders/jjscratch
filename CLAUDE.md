# jjscratch

A native, GPU-rendered [Jujutsu](https://jj-vcs.dev) client — a close UI copy of
**lightjj** (a Svelte web client), reimplemented in Rust with **Vello on wgpu**
for performance. Backend reads repos **in-process via `jj-lib`** (no shelling out).

User-visible name: **jjscratch**.

## Stack & environment

- **Renderer:** Vello `0.9` (re-exports wgpu `29`, peniko, kurbo). Text via
  `skrifa 0.42` (glyph metrics/charmap) + bundled DejaVu fonts in `assets/fonts/`.
- **Backend:** `jj-lib 0.42` (matches the `jj 0.42` that built the fixture),
  behind the opt-in `jjlib` cargo feature. Its API is async — wrap in
  `pollster::block_on`. No `protoc` needed (generated protos are vendored).
- **Headless GPU:** this environment has no display/Vulkan-HW. We render
  offscreen to a texture and read back to PNG. `.cargo/config.toml` pins
  `VK_ICD_FILENAMES` to the **lavapipe** software ICD so `cargo run` just works.
  Adapter reports `llvmpipe ... (Cpu, Vulkan)`.

## Dev / screenshot loop

There is no window. To "see" the app, render a PNG:

```bash
cargo run --bin shot -- out.png 1280 800        # mock data (fast, default)
JJSCRATCH_REPO=$PWD/fixture/repo \
  cargo run --features jjlib --bin shot -- out.png   # real repo via jj-lib
```

Then view `out.png`. The mock (`src/model/mock.rs`) mirrors the committed
fixture, so mock and real renders are directly comparable.

### Ground-truth reference (real lightjj)

`scripts/reference.sh` builds the actual lightjj app (Go+Svelte, toolchains under
`tools/`) and screenshots it via headless Chrome against the SAME fixture →
`docs/reference/*.png`. These are the pixel targets. Compare jjscratch output
against them.

## Fixture

`fixture/repo` is a committed jj repo (regenerate with `scripts/make-fixture.sh`,
which needs `tools/bin/jj`). Built **non-colocated** (only `.jj/`, no workdir
`.git`) so the outer git stores it as normal blobs, not a submodule. Shape:
working-copy `@` (live edits, no description) → `wip: cli flags` → `refactor` →
fork: `experiment` bookmark / immutable `main`=`feat` → `docs` → `init` → root.
Exercises @, immutable ◆, mutable ○, a fork, bookmarks, and a working-copy diff.

## Module map

```
src/render.rs        — headless wgpu device (lavapipe) + Vello Scene -> PNG readback
src/text.rs          — bundled fonts + skrifa glyph layout + draw_text/measure
src/theme.rs         — DARK palette + layout/type metrics (from docs/spec/ui-spec.md)
src/model.rs         — render-ready data contract (Snapshot/CommitNode/CommitDiff)
  model/mock.rs      — fixture-matching mock data (renderer dev target)
  model/jjlib.rs     — real jj-lib loader (feature = "jjlib")
src/graph_layout.rs  — PURE lane-assignment algorithm (DAG -> gutter cells)
src/ui.rs            — chrome shell (toolbar/tabbar/headers/statusbar) + frame
                       layout + build_scene composition + shared paint helpers
  ui/graph.rs        — revision graph renderer (consumes graph_layout)
  ui/diff.rs         — diff panel renderer
src/bin/shot.rs      — headless screenshot harness
docs/spec/ui-spec.md — exhaustive lightjj visual spec (colors/layout/typography)
docs/spec/jjlib-api.md — jj-lib 0.42 integration guide
docs/reference/      — real-lightjj ground-truth screenshots
```

## Conventions

- **The data contract (`model.rs`) is the boundary.** No jj-lib types leak into
  the UI; the loader and the mock fill the SAME plain value types. Build renderers
  against the mock; they work unchanged on real data.
- **Renderers paint only within their `rect`** and clip to it. The shell + frame
  layout in `ui.rs` are stable; content panels plug into `render(scene, rect, …)`.
- **Match the spec numerically.** Use `theme::layout::*` / `theme::font::*`
  constants, never magic px. Fixed 18px graph/diff rows are load-bearing.
- **Keep `jjlib` opt-in.** Default builds use the mock so renderer iteration is
  fast (no heavy jj-lib compile).

## Status

Phase 0 (infra) + architecture + first integrated frame are done. Full-fidelity
graph, diff, and the jj-lib loader were built by parallel worktree agents — see
git log. Not yet done: interactivity/mutations, Branches/Merge views, split diff,
real windowing (winit), theme switching/light theme.
