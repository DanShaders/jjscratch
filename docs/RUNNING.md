# Running jjscratch (the interactive window)

`jjscratch` is the real, on-screen client: a [winit] window with a Vello-on-wgpu
surface render loop, keyboard navigation, and a live `.jj/` watch that
re-renders the moment the repo changes on disk.

> This is distinct from `shot`, the headless screenshot harness (which renders to
> a PNG and never opens a window). Use `jjscratch` on a machine with a display;
> use `shot` for the dev/CI screenshot loop.

## Build & run

The window needs the in-process jj-lib backend, so it is gated on the `jjlib`
cargo feature:

```bash
# Open a repo explicitly:
cargo run --bin jjscratch --features jjlib -- -R /path/to/repo

# Open the current directory (the default when -R is omitted):
cd /path/to/repo && cargo run --bin jjscratch --features jjlib

# Try it against the bundled fixture:
cargo run --bin jjscratch --features jjlib -- -R fixture/repo
```

A release build is smoother for live interaction:

```bash
cargo run --release --bin jjscratch --features jjlib -- -R /path/to/repo
```

If you build the binary without `--features jjlib`, cargo skips it (it has
`required-features = ["jjlib"]`); a forced build prints a clear error telling you
to add the feature.

### CLI

```
jjscratch [-R <repo>] [--smoke]

  -R, --repo <path>   repo to open (default: current directory)
  --smoke             one HEADLESS offscreen render to jjscratch-smoke.png,
                      then exit — no window. Proves the build/data wiring on a
                      display-less machine (used in CI / this dev environment).
  -h, --help          show help
```

## Controls (keyboard)

These mirror lightjj's keymap, routed through the shared `jjscratch::input`
router (the same tokens that drive lightjj for pixel-parity testing):

| Key                    | Action                                            |
| ---------------------- | ------------------------------------------------- |
| `j` / `↓`              | move selection down one revision                  |
| `k` / `↑`              | move selection up one revision                    |
| `1`                    | Revisions view                                    |
| `2`                    | Branches view                                     |
| `3`                    | Merge view                                         |
| `4`                    | toggle the Oplog bottom drawer                    |
| `5`                    | toggle the Evolog bottom drawer                   |
| `t`                    | toggle theme (dark ⇄ light)                        |
| `Ctrl+K` / `Cmd+K`     | open the command palette                          |
| (type) / `Backspace`   | edit the palette query while it is open           |
| `Esc`                  | close the command palette                         |

When the selection moves to a different revision, the selected commit's diff is
recomputed (cached by commit id) and the diff panel updates.

> Mouse is **not** wired yet. A clearly-marked hook is left in
> `src/bin/jjscratch.rs` (`// TODO(mouse): route winit cursor/click/wheel to
> jjscratch::input::handle_mouse once merged`) for the mouse agent to fill in.

## Live watch (reactivity)

The app starts a `jjscratch::watch::Watcher` on the repo's `.jj/` directory. A
small forwarding thread turns its coalesced, debounced change ticks into
non-blocking `RepoChanged` user events on the winit event loop (so the loop is
never blocked polling the watcher). On each tick the `ReactiveReloader` does an
**op-id-gated** reload: if the operation log did not actually move, the reload is
a near-free no-op; otherwise it reloads the snapshot + selected diff + oplog and
requests a redraw.

Try it: run `jjscratch` against a repo, then in another terminal run a jj command
(e.g. `jj describe -m hi`, `jj new`, `jj abandon`). The window updates within a
few tens of milliseconds — no manual refresh.

The event loop runs in `ControlFlow::Wait`, so the app is idle (no busy-loop)
between input and filesystem events.

## GPU / `VK_ICD_FILENAMES` caveat (IMPORTANT)

The repo's `.cargo/config.toml` pins:

```toml
[env]
VK_ICD_FILENAMES = "/usr/share/vulkan/icd.d/lvp_icd.json"
```

This forces the Vulkan loader to **lavapipe** (a CPU software rasterizer) so the
display-less CI/dev environment can render headlessly. On a real machine with a
GPU this would route the window through software rendering — slow, and possibly
unable to present to a real surface.

This binary requests a **real** adapter (HighPerformance power preference,
default backends — it does **not** force lavapipe). But the `.cargo/config.toml`
`[env]` pin is applied by cargo to every process it launches, including this one.
So to use your real GPU, **override `VK_ICD_FILENAMES`** when you run:

```bash
# Unset the pin for this run so the Vulkan loader finds your hardware ICD:
VK_ICD_FILENAMES= cargo run --bin jjscratch --features jjlib -- -R /path/to/repo
```

(An empty value makes the loader fall back to its normal ICD discovery. On
macOS/Windows there is no Vulkan ICD pin to worry about — wgpu uses Metal/DX12
and this caveat does not apply.)

At startup the app logs the adapter it actually selected, e.g.:

```
jjscratch: GPU adapter <name> (DiscreteGpu, Vulkan)
```

If that says `llvmpipe ... (Cpu, ...)` you are still on the software pin —
re-run with `VK_ICD_FILENAMES=` as above.

[winit]: https://docs.rs/winit/0.30
