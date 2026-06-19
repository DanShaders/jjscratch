# Pixel-parity harness

A repeatable scorer for how close **jjscratch** renders to the real **lightjj**
reference screenshots (`docs/reference/*.png`), with an anti-aliasing-tolerant
metric so that font hinting / kerning / sub-pixel edge shifts don't dominate the
signal. This is the feedback loop for UI-tuning agents.

## Run it

```bash
scripts/parity.sh              # scene = revisions (default)
scripts/parity.sh branches     # any docs/reference/<scene>.png
```

`scripts/parity.sh`:

1. builds `shot` with `--features jjlib`,
2. renders the real fixture (`fixture/repo`) at **@2x** (logical 1280x800 →
   device **2560x1600**, matching the reference's `deviceScaleFactor 2`) to
   `docs/parity/jjscratch-<scene>.png`,
3. scores it against `docs/reference/<scene>.png` and prints a per-region table,
4. writes a heatmap and a side-by-side PNG into `docs/parity/`.

It is idempotent and resolves the repo root from its own location, so it runs
from any cwd.

Env overrides: `THRESHOLD=40` (per-channel hard-mismatch, 0–255), `RADIUS=1`
(AA neighborhood radius in px).

### Render @2x directly

```bash
JJSCRATCH_REPO=$PWD/fixture/repo \
  cargo run --features jjlib --bin shot -- out.png 1280 800 --scale 2
# or 4th positional:  ... -- out.png 1280 800 2
```

`width`/`height` are logical px; `--scale N` lays the UI out at the logical size
into a child `Scene`, then appends it under `Affine::scale(N)` and rasterises at
`width*N x height*N`. Default scale is 1, so the legacy `shot -- out.png 1280
800` (mock, @1x) is unchanged.

### Score two arbitrary PNGs

```bash
uv run tools/parity/compare.py \
  --candidate docs/parity/jjscratch-revisions.png \
  --reference docs/reference/revisions.png \
  --scene revisions --scale 2
```

(`uv` fetches Pillow + numpy automatically from the inline script metadata.)

## How to read the scores

```
  region              mismatch %            px
  ----------------------------------------------
  toolbar                  4.44%       174,080
  tab-bar                  0.18%       133,120
  revision-panel           1.17%     1,202,880
  diff-panel               0.94%     2,451,584
  statusbar                0.28%       122,880
  ----------------------------------------------
  OVERALL                  1.11%     4,096,000
```

- **mismatch %** = fraction of pixels in that region counted as a *real* (non-AA)
  difference. Lower is closer. **0% is a perfect copy** of the reference for that
  region (a jjscratch-vs-itself run scores exactly 0.00% everywhere — that's the
  sanity check that the AA rule isn't hiding real diffs or inventing fake ones).
- Per-region scores localise where to tune. Regions:

  | region          | logical rect (x0,y0 → x1,y1) @1x | @2x device rect |
  |-----------------|----------------------------------|-----------------|
  | toolbar         | (0,0) → (1280,34)                | (0,0) → (2560,68) |
  | tab-bar         | (0,34) → (1280,60)               | (0,68) → (2560,120) |
  | revision-panel  | (0,60) → (420,776)               | (0,120) → (840,1552) |
  | diff-panel      | (424,60) → (1280,776)            | (848,120) → (2560,1552) |
  | statusbar       | (0,776) → (1280,800)             | (0,1552) → (2560,1600) |

  (From `src/ui.rs` `FrameLayout::compute(1280,800,420)` and ui-spec §3: toolbar
  34, tab-bar 26, revision panel 420 wide, 4px divider, statusbar 24. The 4px
  divider column 420–424 is excluded from both panels.)

- **heatmap-`<scene>`.png** — the jjscratch render dimmed to 35%, with every
  counted mismatch painted solid red. Open it to see *what* differs; AA-only
  edges stay dark, so red marks real structural/colour/glyph differences.
- **sidebyside-`<scene>`.png** — jjscratch (left) next to lightjj (right) at full
  @2x for eyeballing.

### Expected (un-tuned) state

The current scores are NOT 0 and that's expected: jjscratch ships DejaVu fonts
(lightjj uses Inter/JetBrains Mono) and the graph row ordering isn't fixed yet —
other agents own those. The harness exists to measure that gap shrinking.

## The AA-tolerance rule (exact)

A pixel `(x,y)` counts as a **mismatch** only if BOTH hold:

1. **Hard delta:** `max_channel |candidate - reference| > THRESHOLD` (default
   `40/255`).
2. **No AA excuse**, in either direction. The pixel is *forgiven* (not counted)
   if EITHER:
   - some **reference** pixel in the `(2*RADIUS+1)^2` window (default `RADIUS=1`,
     a 3×3 neighborhood) is within `THRESHOLD` of `candidate[x,y]`  — the
     candidate's colour exists nearby in the reference (edge shifted ≤1px), OR
   - some **candidate** pixel in the same window is within `THRESHOLD` of
     `reference[x,y]` (symmetric).

So a pixel is a real mismatch only when the candidate's colour is **absent** from
the reference's local neighborhood **and** vice-versa. Differently-anti-aliased
or ≤1px-shifted edges (hinting/kerning jitter at @2x) are tolerated; wrong
colours, missing glyphs, and >1px structural shifts are not. The neighborhood
check is a vectorised min-over-shifts of the per-channel Chebyshev distance
(edge-replicated padding), so it stays fast on 2560×1600 images.
