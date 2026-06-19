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

| File                 | Scene                                                        |
| -------------------- | ------------------------------------------------------------ |
| `revisions.png`      | Default revision graph view                                  |
| `branches.png`       | Bookmarks / Branches panel (`2`)                             |
| `diff-navigated.png` | Revisions view, cursor moved down two rows (`j j`)           |

Viewport: **1280×800, deviceScaleFactor 2** (so the actual PNGs are 2560×1600 —
crisp text for diffing). All are full-window (not full-page) screenshots.

## Regenerate

```bash
cd /home/danklishch/work/jjscratch
./scripts/reference.sh
```

That single command:

1. puts the local jj 0.42, Node, and Chrome libs on PATH/env,
2. starts `lightjj -R fixture/repo --addr localhost:3007 --no-browser` in the
   background (kills any stale instance on the port first),
3. waits until it serves,
4. runs `scripts/lightjj-shot.mjs` (puppeteer-core → cached Chrome 127),
5. stops lightjj and cleans up (idempotent, no lingering processes).

Options:

```bash
PORT=3011 ./scripts/reference.sh            # use a different port
./scripts/reference.sh --scene branches     # capture a single scene
```

Add more scenes by editing the `SCENES` array in `scripts/lightjj-shot.mjs`
(each scene = `{ name, keys[], settle }`; keys are keystrokes sent before the
shot — `1`/`2`/`3` switch views, `j`/`k` move the cursor).

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
