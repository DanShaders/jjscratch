# lightjj reference-image harness

Ground-truth screenshots of the **real** lightjj app (the original Go + Svelte
web UI at `/home/danklishch/work/lightjj`), captured headlessly so we can
pixel-compare the jjscratch Rust reimplementation against them.

The harness runs the actual lightjj binary against the **shared fixture repo**
(`fixture/repo`) with the **shared jj config** (`fixture/jjconfig.toml`, which
sets `immutable_heads() = present(main) | tags()`) and the project's jj 0.42
binary on PATH — so the captured UI shows exactly the commits, immutable `◆`
markers, and bookmarks that jjscratch will load.

## Images

Captured PNGs land in this directory (`docs/reference/`):

| File                 | Scene                                              | Trigger                                                        |
| -------------------- | -------------------------------------------------- | ------------------------------------------------------------- |
| `revisions.png`      | Default revision graph view                        | (load; `@` diff auto-opens)                                   |
| `branches.png`       | Bookmarks / Branches panel                         | press `2`                                                     |
| `diff-navigated.png` | Revisions view, cursor moved down two rows         | `1` then `j j`                                               |
| `oplog.png`          | Operation-log bottom drawer                        | press `4`                                                     |
| `evolog.png`         | Evolution-log drawer for the selected `@` revision | `@` (re-select) then `5`                                      |
| `split-diff.png`     | Diff panel toggled to side-by-side **split** view  | `@` `Enter` to open `@`'s diff, then **click** the `≡` toggle |
| `palette.png`        | Cmd+K command palette, filtered by a typed query   | **Ctrl+K**, then type `split`                                |
| `light.png`          | Whole UI in the **light** theme                    | press `t`                                                     |
| `merge.png`          | Merge view: ConflictQueue + 3-pane resolver        | press `3` **against the conflict fixture** (see below)       |

Viewport: **1280×800, deviceScaleFactor 2** (so the actual PNGs are 2560×1600 —
crisp text for diffing). All are full-window (not full-page) screenshots.

### Trigger notes (verified against lightjj `App.svelte`)

- View/drawer keys are from `handleGlobalKeys`: `1` Revisions, `2` Branches,
  `3` Merge, `4` Oplog drawer, `5` Evolog drawer, `t` toggle theme.
  `4`/`5` switch to the log view first, then open the bottom drawer.
- **Evolog** (`5`) is gated on a selected revision. The working-copy `@` is
  selected on load and has evolog history (snapshot + create), so the scene
  presses `@` to re-select it defensively, then `5` — the drawer shows 2 entries.
- **Split diff** has **no keybinding** — it's the `≡`/`◫` toolbar button in the
  diff panel (`aria-label="Switch to split view"` / `"Switch to unified view"`,
  `toggleSplitView` in `DiffPanel.svelte`). The scene opens a diff with `Enter`
  and then clicks that button. The harness clicks it back to unified afterward
  so the diff state doesn't bleed into later scenes (e.g. `light`).
- **Command palette** opens on **Cmd+K / Ctrl+K** (`handleGlobalOverrides`,
  `e.metaKey || e.ctrlKey`). Headless Chrome doesn't reliably register Meta, so
  the harness sends **Control+K**; lightjj binds both, so it works. The scene
  types `split` into `.palette-input` to capture the filtered list.
- **Light theme** (`t`) is a sticky toggle; the harness presses `t` again after
  the shot to restore dark for subsequent scenes.

### Merge view & the conflict fixture (`fixture-conflict/`)

The shared `fixture/repo` is deliberately **conflict-free**, so lightjj's
`switchToMergeView` (revset `conflicts() & mutable()`) finds nothing there and
bails back to the log. The `merge` scene therefore runs against a **separate**
fixture, `fixture-conflict/repo`, built by `scripts/make-conflict-fixture.sh`:

1. base commit adds `greeting.txt` (3 lines) on the immutable `main`;
2. `feature-a` rewrites line two one way;
3. `feature-b` (sibling off `main`) rewrites the **same** line differently;
4. `jj new feature-a feature-b` auto-merges the siblings → jj records an
   unresolved **2-sided conflict** in the working-copy merge commit (`@`),
   verified by `jj status` ("greeting.txt 2-sided conflict").

That `@` conflict is what the Merge view (`3`) surfaces: the ConflictQueue lists
`greeting.txt` ("0/1 resolved") and the 3-pane shows feature-a / Result /
feature-b. The fixture is committed (non-colocated, plain blobs) like
`fixture/repo`.

The `merge` scene is **excluded from the default run** (it needs the conflict
fixture); it's captured with an explicit single-scene pass that points lightjj
at the conflict fixture via the `FIXTURE_REPO`/`JJCONFIG` env overrides:

```bash
FIXTURE_REPO=$PWD/fixture-conflict/repo \
JJCONFIG=$PWD/fixture-conflict/jjconfig.toml \
  ./scripts/reference.sh --scene merge
```

## Regenerate

```bash
cd /home/danklishch/work/jjscratch

# 1. all scenes EXCEPT merge (against the shared fixture):
./scripts/reference.sh

# 2. the merge scene (needs the conflict fixture — rebuild it if missing):
./scripts/make-conflict-fixture.sh           # writes fixture-conflict/repo
FIXTURE_REPO=$PWD/fixture-conflict/repo \
JJCONFIG=$PWD/fixture-conflict/jjconfig.toml \
  ./scripts/reference.sh --scene merge
```

`./scripts/reference.sh`:

1. puts the local jj 0.42, Node, and Chrome libs on PATH/env,
2. starts `lightjj -R <fixture> --addr localhost:3007 --no-browser` in the
   background (kills any stale instance on the port first),
3. waits until it serves,
4. runs `scripts/lightjj-shot.mjs` (puppeteer-core → cached Chrome 127),
5. stops lightjj and cleans up (idempotent, no lingering processes).

The default run skips `merge` (it would be blank against the conflict-free
shared fixture); pass `--scene merge` with the `FIXTURE_REPO`/`JJCONFIG`
overrides above to capture it.

> **Worktrees:** the downloaded toolchains live only in the **shared checkout's**
> `tools/`. `reference.sh` uses this checkout's `tools/` if it holds the lightjj
> binary, otherwise falls back to `/home/danklishch/work/jjscratch/tools`, so the
> harness runs unchanged from a git worktree.

Options:

```bash
PORT=3011 ./scripts/reference.sh            # use a different port
./scripts/reference.sh --scene branches     # capture a single scene
```

Add more scenes by editing the `SCENES` array in `scripts/lightjj-shot.mjs`.
Each scene is `{ name, keys[], settle, waitFor?, act?, requiresConflictFixture? }`:

- `keys[]` — keystrokes sent before the shot. Bare digits map to `Digit*`;
  single punctuation (e.g. `@`) is typed as a character; everything else
  (`Enter`, letters, …) is a puppeteer key name. `1`/`2`/`3` switch views,
  `j`/`k` move the cursor.
- `waitFor` — a CSS selector awaited after the keys, so the shot doesn't fire
  before the view mounts (e.g. `.oplog-panel`, `.evolog-panel`, `.merge-panel`).
- `act(page)` — extra puppeteer driving that bare keys can't express (clicking
  the split toggle, sending Ctrl+K and typing into the palette).
- `requiresConflictFixture` — excludes the scene from the default run; only
  captured via `--scene <name>` (used by `merge`).

## (Re)building lightjj

The harness uses a prebuilt binary at `tools/bin/lightjj`. To rebuild it from
`/home/danklishch/work/lightjj` (e.g. after pulling new lightjj source), use the
locally-downloaded toolchains under `tools/` — **no sudo, no apt, nothing
outside `/home/danklishch/work`**:

```bash
ROOT=/home/danklishch/work/jjscratch
LJ=/home/danklishch/work/lightjj

# --- frontend (writes ../cmd/lightjj/frontend-dist/) ---
export PATH="$ROOT/tools/node/bin:$PATH"        # local Node 22 (see "Versions")
cd "$LJ/frontend"
npx --yes pnpm@10 install
npx --yes pnpm@10 run build

# --- backend (embed tag bundles the frontend into the binary) ---
export GOROOT="$ROOT/tools/go-root"
export GOPATH="$ROOT/tools/gopath"
export GOCACHE="$ROOT/tools/gocache"
export GOMODCACHE="$ROOT/tools/gopath/pkg/mod"
export PATH="$GOROOT/bin:$PATH"
cd "$LJ"
go build -tags embed -o "$ROOT/tools/bin/lightjj" ./cmd/lightjj
```

`lightjj --version` should print `v1.33.0` (or whatever `lightjj/version.txt`
says). Built binary reports `jj 0.42.0` via `/tab/0/api/info` against the
fixture.

## Versions used

| Tool          | Version          | Source                                                       |
| ------------- | ---------------- | ------------------------------------------------------------ |
| Go            | 1.25.9           | official `go1.25.9.linux-amd64.tar.gz` → `tools/go-root`     |
| Node          | 22.12.0 (LTS)    | official `node-v22.12.0-linux-x64` → `tools/node`            |
| pnpm          | 10.34.4          | run on demand via `npx pnpm@10` (no global install)          |
| jj            | 0.42.0           | preexisting `tools/bin/jj`                                   |
| lightjj       | v1.33.0          | built here → `tools/bin/lightjj`                             |
| Chrome        | 127.0.6533.72    | preexisting puppeteer cache (`~/.cache/puppeteer/chrome`)    |
| puppeteer-core| 25.1.0           | `tools/refharness/node_modules` (local `npm install`)        |

## Gotchas

- **System Go is absent and system Node is v18.** lightjj needs Go 1.25+, and
  Vite 8 needs Node ≥ 20.19. Both are downloaded locally into `tools/` and put
  on PATH only for our commands; the system is untouched.
- **pnpm version.** `pnpm@latest` (11.x) requires Node ≥ 22.13 and rejects Node
  18; `pnpm@9` chokes on this repo's `pnpm-workspace.yaml` (no `packages:` key).
  **`pnpm@10`** works with the lockfile-9.0 lockfile and Node 22 — use it.
- **Chrome shared libraries.** The cached Chrome 127 dynamically links a dozen
  system libs (`libnss3`, `libnspr4`, `libatk*`, `libcups2`, `libpango`,
  `libcairo`, `libasound2`, `libatspi`, `libavahi*`, `libxcb-render`, …) that
  are NOT installed in this headless box. We fetched the Ubuntu 24.04 (noble)
  `.deb`s with `apt-get download` (no root) and extracted them with `dpkg-deb -x`
  into `tools/chrome-deps/root`; `reference.sh` puts
  `tools/chrome-deps/root/usr/lib/x86_64-linux-gnu` on `LD_LIBRARY_PATH`.
  If lightjj is ever pointed at a newer Chrome, re-resolve missing libs with
  `ldd <chrome> | grep 'not found'` and `apt-get download` the providers.
- **Chrome must run with `--no-sandbox --headless=new`** in this environment
  (no display, no user namespaces). The script already passes these.
- **puppeteer-core resolution.** `lightjj-shot.mjs` resolves `puppeteer-core`
  from `tools/refharness/` explicitly (ESM ignores `NODE_PATH`), so it runs
  regardless of cwd.
- **Everything downloaded lives under `tools/`**, which is gitignored (`/tools`
  in the repo root `.gitignore`). The reference PNGs and these scripts are not
  ignored; the main session manages committing them.
```
