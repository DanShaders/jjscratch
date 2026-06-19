# Isolating real parity diffs from AA / sub-pixel-kerning noise

**Status:** research prototype. Read-only w.r.t. existing code. New artefacts:
`tools/parity/compare2.py`, this doc, and sample outputs under `docs/research/`.

## Problem

`tools/parity/compare.py` (the production scorer, run via `scripts/parity.sh`)
counts a pixel as a mismatch only when its max-channel colour delta exceeds a
threshold (40/255) **and** the colour is absent from a 1px neighborhood in both
directions. That AA rule cancels clean antialiased / ≤1px-shifted edges, but it
still reports an **overall ~0.55–0.61%** on the `revisions` frame. That floor is
not structural error — it is the residue of two unavoidable differences between
the jjscratch (Vello/skrifa) rasteriser and the lightjj (Chrome) one:

- **Antialiasing**: the two engines distribute edge coverage differently.
- **Sub-pixel kerning / hinting**: JetBrains Mono / Inter advances and hinting
  differ, so glyphs land 1–3 device px to the side of the reference. A 1px
  neighborhood cannot forgive a 2–3px horizontal drift, so a thin run of edge
  pixels survives along *every* line of text.

That diffuse ~0.5% **hides** the small, localised regressions we actually care
about: a missing toolbar icon, a recoloured active button, a 3px-misplaced
badge or underline. A human staring at a uniformly-pink heatmap cannot find
them, and a CI gate on the overall % cannot distinguish "AA drifted slightly"
from "a glyph went missing."

**Goal:** a metric that suppresses the diffuse AA/kerning floor and surfaces
real, localised/structural differences — with bounding boxes so a human or agent
sees *where*.

## Key observation

The noise and the signal have different **spatial shape**, not just magnitude:

| | spatial signature |
|---|---|
| AA / kerning noise | **thin** (1–2px), **scattered** runs hugging text & UI edges; each run is forgiven by a small **horizontal** slide (kerning drifts sideways) |
| Real regression | a **compact / large / filled** cluster — a solid recoloured block, a missing glyph, a shifted element — whose colour is **absent** from the reference's local band entirely |

So the discriminator is connected-component **shape + a kerning-tolerant slide
test**, applied to the *same* base mask compare.py already produces.

## Techniques evaluated

### 1. Supersample normalisation — *discussed, not adopted*

jjscratch can render @4x (`shot --scale 4`) and downscale 4x→2x to average away
*its* rasteriser-specific AA. But the lightjj reference PNGs are fixed @2x and
cannot be re-rendered at higher SS. Downscaling only the jjscratch side makes its
AA *softer* than the reference's, which does not necessarily reduce the per-pixel
delta — it can even raise it, because the two sides' edge profiles now mismatch
in a *new* way. SS normalisation only pays off when **both** sides are
supersampled from the same geometry; here we cannot. **Verdict: low value for
this pipeline; skipped.** (If lightjj references were ever re-captured at @4x and
box-downscaled, revisit — symmetric SS is the cleanest AA killer.)

### 2. Connected-component clustering + shape classification — **ADOPTED (primary)**

Label the base mismatch mask (8-connectivity) and classify each blob:

- **Shape gate.** A component is a *candidate real* only if it is **not** a thin
  scattered run: `area ≥ min_area (80)` **and** either it is reasonably 2-D
  (`min(bbox_w, bbox_h) > thin_px (3)`) **or** it is a dense filled run
  (`fill = area/bbox_area ≥ fill_min (0.55)`). The dense-run clause rescues a
  solid object that is short in one axis (e.g. a 178×2 recoloured underline) — it
  is densely filled, unlike a sparse AA edge.
- Everything failing the gate (tiny specks, 1–2px sparse edge runs) is **NOISE**.

This alone drops `revisions` from 0.591% → 0.044% real, with **10 clean
components**, the toolbar logo ranked #1. A **jjscratch-vs-itself** run yields
**0 real components / 0.000%** — the required sanity check that the rule invents
nothing.

### 3. Small-shift-tolerant text matching — **ADOPTED (as a per-component re-test)**

Naïvely *pre-dilating* the masks horizontally by ±K px before clustering is a
trap: it also erases solid colour blocks (the active-button fill we *want* to
keep), because every colour in a filled block exists ±K px away within itself.

Instead we run the slide as a **per-component re-test**. For each component that
passes the shape gate, measure `kern_frac` = the fraction of its pixels that a
wide-horizontal AA window (±`kx` = 3 px horizontal, ±`ky` = 1 px vertical)
forgives. If `kern_frac ≥ kern_demote (0.85)`, the blob was just a glyph that
drifted sideways → **demote to NOISE**. A missing/wrong glyph or recoloured block
has a low `kern_frac` (its colour is absent from the band) and stays REAL.

This is what lets the metric keep the logo (`kern_frac` 0.39) and the misplaced
underline (`kern_frac` 0.02) while shedding the long kerning runs.

### 4. Perceptual / structural (SSIM) — **evaluated, kept OPT-IN (`--use-ssim`), not default**

A windowed luma SSIM map is reported for context (mean + low-area %). We also
prototyped promoting **anti-correlated** SSIM patches (`SSIM < 0`, where local
luma structure is inverted/uncorrelated — a true replacement, not a shift) to
REAL, to catch the one diff the colour rule misses: the **Branches** toolbar icon
rendered as a clean `⇄` glyph in jjscratch but as a **tofu box** in the
reference. Findings:

- SSIM *does* light up the tofu (window mean 0.295; 27% of the patch at `SSIM<0`
  vs ~1% for code text) — it keys on shape, not colour, so it sees what the
  colour rule (which barely fires there because both glyphs share dark/orange
  ink) cannot.
- **But** windowed SSIM also dips across every line of differently-hinted text.
  Even at the strict `SSIM<0` threshold with closing + an aspect-ratio gate, the
  SSIM source pulled in ~170 components, most of them commit-id / "Branches" /
  code-text fragments — i.e. it re-introduced exactly the kerning noise we set
  out to suppress. The colour-cluster source, by contrast, holds ~10 components
  with no text false-positives.

**Verdict:** SSIM is a valuable *probe* and a good complementary heatmap, but too
noisy to be a *primary* real-region source in this text-dense UI. It is therefore
**off by default** and available via `--use-ssim` for targeted icon-glyph audits.

> Honest limitation: a glyph→tofu swap where both share most of their ink is the
> known blind spot of any colour-shift metric. It is detectable only by a
> structural metric, which here costs unacceptable text false-positives. If
> catching tofu becomes a gate requirement, the right fix is upstream
> (font-fallback parity) plus a *targeted* SSIM check restricted to the small set
> of known icon bounding boxes — not a global SSIM pass.

### 5. Text-mask separation — *folded into the kerning re-test, not a separate pass*

We considered detecting monospace text regions and scoring them leniently while
scoring chrome strictly. In practice the **per-component kerning re-test (3)**
already does this implicitly and more robustly: text components have high
`kern_frac` and are demoted, chrome blocks have low `kern_frac` and survive — no
region segmentation needed. Region *reporting* (toolbar / tab-bar / panels /
statusbar) is retained from compare.py for localisation.

## The adopted rule (exact)

Given candidate `C`, reference `R`, threshold `T = 40`:

1. **Base mask** `B` = compare.py's rule: `maxch|C−R| > T` AND the colour is
   absent from a 1px neighborhood in both directions. (`compare2`'s "base" line
   reproduces compare.py's overall % exactly — verified 0.591% == 0.59%.)
2. **Label** `B` into 8-connected components.
3. For each component with `area ≥ 80`:
   - **Shape gate:** keep if `min(bbox_w,bbox_h) > 3` **or** `fill ≥ 0.55`.
   - **Kerning re-test:** compute `kern_frac` against a ±3px-horizontal /
     ±1px-vertical AA window; **demote** if `kern_frac ≥ 0.85`.
   - Survivors are **REAL**.
4. **Structural score** = REAL pixel area / total px. **Report** the top-N REAL
   components with `@device` bounding boxes and their host region.

Thresholds are CLI flags (`--min-area --thin-px --fill-min --kx --ky
--kern-demote`) so the gate can be tuned without code edits.

## Results on the current images

Run (default mode):

```
uv run tools/parity/compare2.py \
  --candidate docs/parity/jjscratch-revisions.png \
  --reference docs/reference/revisions.png \
  --out-dir docs/research --scene revisions --scale 2
```

| metric | value |
|---|---|
| base mismatch (== compare.py overall) | **0.591 %** (24,197 px) |
| after kerning slide (±3px) | 0.139 % |
| **REAL / structural (clustered)** | **0.044 %** (1,787 px) |
| **noise suppressed vs compare.py** | **0.547 % — 92.6 % of the base removed** |
| REAL components | **10** |
| jjscratch **vs itself** (sanity) | **0.000 % / 0 components** |

Top REAL components (device coords), all correctly real diffs:

| # | region | area | bbox WxH | what it is |
|---|---|---|---|---|
| 1 | toolbar | 414 | 12×40 | **the "jj" logo glyph** — Vello lightning-bolt shape vs the reference logo |
| 2 | diff-panel | 364 | 182×2 | **"main.rs" active-file underline misplaced** (orange bar a few px lower than the reference — see `c-/r-files` crops) |
| 3 | diff-panel | 320 | 160×2 | second segment of the same misplaced underline |
| 4–5 | revision-panel | ~110 | chip-sized | **`experiment` / `main` bookmark-chip** rendering diffs |
| 6–10 | diff/panel | 85–105 | glyph-sized | individual mis-rendered glyphs / markers that survive the slide |

Across the interaction sequence the suppression is consistent: base 0.33–0.59% →
real 0.04–0.06%, **88–93% of noise removed** each step.

`--use-ssim` on the same frame: real jumps to 1.376 % / **171 components** — the
documented noisy mode (SSIM catches the Branches tofu icon but drags in code-text
rows). Use it only for targeted icon audits.

### Artefacts

- `docs/research/compare2-annotated-revisions.png` — dimmed render with red
  numbered boxes around each REAL component.
- `docs/research/compare2-split-revisions.png` — **split heatmap**: suppressed
  AA/kerning noise in dim **blue**, REAL pixels in **red**. The whole frame is
  blue stipple (the floor) with a handful of red spots (logo, underline, chips) —
  the visual proof that the floor is noise.
- `docs/research/compare2-revisions.json` — machine-readable component list.
- `docs/research/interaction/*` — the same for the interaction steps.

## Recommendation for the parity gate

Adopt **technique 2 + 3** (connected-component shape classification with the
per-component kerning re-test) as the structural-parity signal, **alongside**
compare.py (keep the overall % as a coarse trend line; add the structural score
as the gate).

**Proposed gate rule:**

- **Hard fail** if any REAL component has `area ≥ 300` px @device (a missing
  glyph, recoloured block, or shifted element of icon size or larger), OR if the
  REAL structural score exceeds **0.10 %**.
- **Warn** on REAL score in `0.03 %–0.10 %` (small glyph/chip diffs — the current
  state sits here at 0.044 %).
- The overall base % stays as an informational trend metric only; **do not** gate
  on it (it is dominated by AA/kerning noise).
- `--use-ssim` stays a manual, opt-in audit tool, not part of the gate.

**Integration path (follow-up, not done here):** add a `--structural` mode to
`scripts/parity.sh` that runs `compare2.py`, prints the REAL component table, and
exits non-zero on the gate above. compare.py and its README are unchanged; this
is purely additive. The thresholds above are starting points calibrated to the
current `revisions` frame and should be re-checked once font-fallback parity
(the Branches tofu) and the underline placement are fixed.
