# /// script
# requires-python = ">=3.9"
# dependencies = ["pillow>=10", "numpy>=1.26", "scipy>=1.11"]
# ///
"""compare2 -- AA/kerning-ISOLATING parity scorer (research prototype).

This is a NON-DESTRUCTIVE sibling of tools/parity/compare.py. It does NOT
replace it. Its job is to take the same two same-size PNGs (a jjscratch render
and the matching lightjj reference) and separate *real* differences (missing or
wrong glyphs, wrong colours, shifted/missing UI elements) from the diffuse
~0.5% floor of antialiasing + sub-pixel kerning noise that compare.py still
counts.

The core idea: compare.py already produces an AA-tolerant per-pixel mismatch
mask, but that mask is still ~0.5% full of *thin, scattered* pixels strung
along text edges (kerning/hinting jitter the AA-rule can't fully forgive,
because a 1px window isn't enough for a 2-3px horizontal glyph drift). Those
pixels are NOISE. Real regressions instead form *compact, localised* clusters
(a filled colour block, a missing glyph, a shifted badge).

So compare2:

  1. Builds a base mismatch mask with the SAME AA-tolerant rule as compare.py
     (hard delta + 1px neighborhood excuse), so we start from the same place.

  2. CONNECTED-COMPONENT clusters the BASE mask (8-connectivity) and CLASSIFIES
     each component by size/shape -- crucially WITHOUT first erasing anything,
     so a large filled colour block (e.g. an active-button fill flipped from
     dark to orange) survives as one big compact component:
        - thin & scattered (<= THIN px in min dimension, low fill) -> AA/kerning
          residue -> SUPPRESSED.
        - compact / large (area >= MIN_AREA and reasonably filled) -> REAL.

  3. Adds a per-component KERNING re-test for the borderline survivors. A
     component is re-checked against a kerning-tolerant excuse (the reference /
     candidate colour allowed to slide +-K px horizontally). If almost all of
     the component's pixels are forgiven by that wider horizontal window, it was
     just a glyph that drifted sideways -> demote to NOISE. A *missing* glyph
     (no matching colour anywhere in the band) keeps almost all its pixels and
     stays REAL. This removes the long horizontal text-edge runs the 1px rule
     leaves behind, WITHOUT erasing solid colour blocks.

     The "structural diff" score is the REAL area only. It also lists the
     top-N real components with bounding boxes so a human/agent sees WHERE.

  4. Reports windowed SSIM as an orthogonal perceptual check: SSIM is ~1
     across AA noise and dips only where structure actually differs.

It writes annotated PNGs: red boxes around REAL components over the dimmed
candidate, plus a "noise vs real" split heatmap (suppressed pixels in dim blue,
real pixels in red).

Usage (uv fetches deps from the inline metadata):

    uv run tools/parity/compare2.py \
        --candidate docs/parity/jjscratch-revisions.png \
        --reference docs/reference/revisions.png \
        --out-dir   docs/research \
        --scene     revisions --scale 2
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass, asdict

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage


# --------------------------------------------------------------------------- #
# Region layout (identical to compare.py, kept in sync by hand).
# --------------------------------------------------------------------------- #
@dataclass(frozen=True)
class Region:
    name: str
    x0: float
    y0: float
    x1: float
    y1: float


LOGICAL_W, LOGICAL_H = 1280.0, 800.0
PANEL_W, DIVIDER_W = 420.0, 4.0
TOOLBAR_H, TABBAR_H, STATUSBAR_H = 34.0, 26.0, 24.0
BODY_TOP = TOOLBAR_H + TABBAR_H          # 60
BODY_BOT = LOGICAL_H - STATUSBAR_H       # 776
DIFF_X = PANEL_W + DIVIDER_W             # 424

REGIONS = [
    Region("toolbar", 0.0, 0.0, LOGICAL_W, TOOLBAR_H),
    Region("tab-bar", 0.0, TOOLBAR_H, LOGICAL_W, BODY_TOP),
    Region("revision-panel", 0.0, BODY_TOP, PANEL_W, BODY_BOT),
    Region("diff-panel", DIFF_X, BODY_TOP, LOGICAL_W, BODY_BOT),
    Region("statusbar", 0.0, BODY_BOT, LOGICAL_W, LOGICAL_H),
]


# --------------------------------------------------------------------------- #
# Loading / basic distance.
# --------------------------------------------------------------------------- #
def load_rgb(path: str) -> np.ndarray:
    return np.asarray(Image.open(path).convert("RGB"), dtype=np.int16)


def chebyshev(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    return np.abs(a - b).max(axis=2)


def neighborhood_min_dist(target, source, ry, rx):
    """Min Chebyshev colour distance from target[x,y] to any source pixel in a
    (2*ry+1) x (2*rx+1) window. Anisotropic so we can dilate wider horizontally
    (kerning drifts horizontally) than vertically.
    """
    h, w, _ = target.shape
    best = np.full((h, w), 255, dtype=np.int16)
    padded = np.pad(source, ((ry, ry), (rx, rx), (0, 0)), mode="edge")
    for dy in range(-ry, ry + 1):
        for dx in range(-rx, rx + 1):
            sy, sx = dy + ry, dx + rx
            shifted = padded[sy:sy + h, sx:sx + w, :]
            dist = np.abs(target - shifted).max(axis=2)
            best = np.minimum(best, dist)
    return best


# --------------------------------------------------------------------------- #
# Masks.
# --------------------------------------------------------------------------- #
def base_mask(cand, ref, threshold, radius):
    """compare.py's exact AA-tolerant rule (isotropic radius)."""
    hard = chebyshev(cand, ref) > threshold
    cand_in_ref = neighborhood_min_dist(cand, ref, radius, radius) <= threshold
    ref_in_cand = neighborhood_min_dist(ref, cand, radius, radius) <= threshold
    return hard & ~(cand_in_ref | ref_in_cand)


def kerning_forgiven_mask(cand, ref, threshold, ry, kx):
    """Per-pixel boolean: True where a pixel of the base mask is FORGIVEN once
    we allow a wide HORIZONTAL window (+-kx) and a small vertical one (+-ry) for
    the AA excuse -- i.e. the differing colour exists within +-kx px sideways in
    either direction. A glyph that merely slid sideways up to kx px is forgiven;
    a glyph absent from the whole band is not.
    """
    cand_in_ref = neighborhood_min_dist(cand, ref, ry, kx) <= threshold
    ref_in_cand = neighborhood_min_dist(ref, cand, ry, kx) <= threshold
    return cand_in_ref | ref_in_cand


# --------------------------------------------------------------------------- #
# Connected-component classification.
# --------------------------------------------------------------------------- #
@dataclass
class Comp:
    label: int
    area: int          # mismatched pixels in the component
    x0: int
    y0: int
    x1: int
    y1: int
    bbox_w: int
    bbox_h: int
    fill: float        # area / bbox-area  (compactness)
    min_dim: int       # min(bbox_w, bbox_h)
    kern_frac: float   # fraction of the component's pixels forgiven by kerning slide
    real: bool
    reason: str
    region: str


def classify_components(mask, kern_forgiven, scale, min_area, thin_px,
                        fill_min, kern_demote):
    """Label the BASE mask (8-connectivity), measure each blob, classify.

    Stage A -- shape gate. A component passes the shape gate (candidate REAL)
    when it is NOT a thin scattered run:
      - large enough  (area >= min_area), AND
      - either reasonably 2-D (min bbox dimension > thin_px) OR a dense filled
        run (fill >= fill_min) -- a solid colour block can be short in one axis
        but is densely filled, unlike an AA edge which is sparse.

    Stage B -- kerning re-test. For components that pass the shape gate, measure
    the fraction of their pixels that the wide-horizontal kerning window would
    forgive (`kern_frac`). If that fraction >= kern_demote, the blob was just a
    horizontally-drifted glyph cluster -> demote to NOISE. A missing/wrong glyph
    or a recoloured block has a low kern_frac (its colour is absent from the
    band) and stays REAL.

    Everything that fails the shape gate is NOISE (tiny specks, thin AA runs).
    """
    structure = np.array([[1, 1, 1], [1, 1, 1], [1, 1, 1]])
    labels, n = ndimage.label(mask, structure=structure)
    if n == 0:
        return [], labels
    slices = ndimage.find_objects(labels)
    areas = ndimage.sum_labels(np.ones_like(mask), labels, index=np.arange(1, n + 1))
    # Per-component count of pixels forgiven by the kerning slide.
    kern_sum = ndimage.sum_labels(kern_forgiven, labels, index=np.arange(1, n + 1))

    comps = []
    for i, sl in enumerate(slices, start=1):
        if sl is None:
            continue
        ys, xs = sl
        y0, y1 = ys.start, ys.stop
        x0, x1 = xs.start, xs.stop
        bw, bh = x1 - x0, y1 - y0
        area = int(areas[i - 1])
        bbox_area = max(1, bw * bh)
        fill = area / bbox_area
        min_dim = min(bw, bh)
        kern_frac = (kern_sum[i - 1] / area) if area else 0.0

        is_2d = min_dim > thin_px
        is_dense_run = fill >= fill_min
        passes_shape = (area >= min_area) and (is_2d or is_dense_run)

        if not passes_shape:
            real = False
            if area < min_area:
                reason = "tiny"
            else:
                reason = "thin-sparse-edge"
        elif kern_frac >= kern_demote:
            real = False
            reason = "kerning-slide"
        else:
            real = True
            reason = "compact-2d" if is_2d else "dense-run"

        cx = ((x0 + x1) / 2) / scale
        cy = ((y0 + y1) / 2) / scale
        region = "?"
        for r in REGIONS:
            if r.x0 <= cx < r.x1 and r.y0 <= cy < r.y1:
                region = r.name
                break

        comps.append(Comp(i, area, x0, y0, x1, y1, bw, bh, round(fill, 3),
                          min_dim, round(float(kern_frac), 3), real, reason, region))
    return comps, labels


# --------------------------------------------------------------------------- #
# Windowed SSIM (luma), no skimage dependency.
# --------------------------------------------------------------------------- #
def ssim_map(cand, ref, win=7):
    a = cand.astype(np.float64) @ np.array([0.299, 0.587, 0.114])
    b = ref.astype(np.float64) @ np.array([0.299, 0.587, 0.114])
    C1, C2 = (0.01 * 255) ** 2, (0.03 * 255) ** 2
    k = (win, win)
    mu_a = ndimage.uniform_filter(a, k)
    mu_b = ndimage.uniform_filter(b, k)
    mu_a2, mu_b2, mu_ab = mu_a * mu_a, mu_b * mu_b, mu_a * mu_b
    va = ndimage.uniform_filter(a * a, k) - mu_a2
    vb = ndimage.uniform_filter(b * b, k) - mu_b2
    vab = ndimage.uniform_filter(a * b, k) - mu_ab
    s = ((2 * mu_ab + C1) * (2 * vab + C2)) / ((mu_a2 + mu_b2 + C1) * (va + vb + C2))
    return s


def ssim_structural_components(sm, scale, ssim_hard, min_area, thin_px, max_aspect):
    """Find compact regions of SEVERELY low / anti-correlated SSIM.

    SSIM keys on local STRUCTURE (mean/variance/covariance of luma), not colour
    presence, so it catches shape regressions the colour-cluster misses -- e.g.
    a glyph replaced by a tofu box, where both share a lot of ink so the colour
    rule barely fires, but the *structure* is clearly wrong.

    The trick is the THRESHOLD. Kerning/AA merely *shift* structure, so windowed
    SSIM over differently-kerned text still stays moderately positive (~0.8).
    A genuine replacement makes the local luma structure UNCORRELATED or
    INVERTED, driving SSIM to ~0 or negative. So we threshold at ssim_hard=0.0
    by default: only anti-correlated patches qualify. Empirically a tofu/glyph
    swap covers ~27% of its window at SSIM<0 while code-text covers ~1%.

    We then close gaps to merge multi-stroke defects, and keep only blobs that
    are (a) big enough, (b) not thin, and (c) not a long thin row -- text rows
    have a huge width:height aspect; an icon/glyph defect is roughly square. So
    we reject components whose aspect ratio exceeds max_aspect.
    """
    low = sm < ssim_hard
    # Close small gaps so a multi-stroke defect (tofu bars) becomes one blob.
    low = ndimage.binary_closing(low, structure=np.ones((3, 3)), iterations=2)
    structure = np.array([[1, 1, 1], [1, 1, 1], [1, 1, 1]])
    labels, n = ndimage.label(low, structure=structure)
    if n == 0:
        return [], np.zeros_like(low)
    slices = ndimage.find_objects(labels)
    areas = ndimage.sum_labels(np.ones_like(low), labels, index=np.arange(1, n + 1))
    comps = []
    keep_ids = []
    for i, sl in enumerate(slices, start=1):
        if sl is None:
            continue
        ys, xs = sl
        y0, y1, x0, x1 = ys.start, ys.stop, xs.start, xs.stop
        bw, bh = x1 - x0, y1 - y0
        area = int(areas[i - 1])
        aspect = max(bw, bh) / max(1, min(bw, bh))
        if area < min_area or min(bw, bh) <= thin_px or aspect > max_aspect:
            continue
        keep_ids.append(i)
        cx = ((x0 + x1) / 2) / scale
        cy = ((y0 + y1) / 2) / scale
        region = "?"
        for r in REGIONS:
            if r.x0 <= cx < r.x1 and r.y0 <= cy < r.y1:
                region = r.name
                break
        comps.append(Comp(i + 100000, area, x0, y0, x1, y1, bw, bh,
                          round(area / max(1, bw * bh), 3), min(bw, bh), 0.0,
                          True, "ssim-low", region))
    mask = np.isin(labels, keep_ids) if keep_ids else np.zeros_like(low)
    return comps, mask


# --------------------------------------------------------------------------- #
# Helpers.
# --------------------------------------------------------------------------- #
def scaled_rect(r: Region, scale, w, h):
    x0 = max(0, min(int(round(r.x0 * scale)), w))
    y0 = max(0, min(int(round(r.y0 * scale)), h))
    x1 = max(0, min(int(round(r.x1 * scale)), w))
    y1 = max(0, min(int(round(r.y1 * scale)), h))
    return x0, y0, x1, y1


def pct(num, den):
    return 100.0 * num / den if den else 0.0


def make_split_heatmap(cand, base, real_mask):
    """Dimmed candidate; suppressed-noise pixels in dim blue, REAL pixels red."""
    dim = (cand.astype(np.float32) * 0.30).astype(np.uint8)
    out = dim.copy()
    noise = base & ~real_mask
    out[noise] = (40, 70, 160)   # suppressed AA/kerning noise -> blue
    out[real_mask] = (255, 30, 30)  # real -> red
    return Image.fromarray(out, "RGB")


def make_annotated(cand, real_comps):
    img = Image.fromarray((cand.astype(np.float32) * 0.55).astype(np.uint8), "RGB")
    d = ImageDraw.Draw(img)
    real = sorted(real_comps, key=lambda c: c.area, reverse=True)
    for rank, c in enumerate(real, start=1):
        pad = 4
        d.rectangle([c.x0 - pad, c.y0 - pad, c.x1 + pad, c.y1 + pad],
                    outline=(255, 40, 40), width=3)
        d.text((c.x0 - pad, max(0, c.y0 - pad - 16)), f"#{rank}",
               fill=(255, 220, 0))
    return img


# --------------------------------------------------------------------------- #
# Main.
# --------------------------------------------------------------------------- #
def main() -> int:
    ap = argparse.ArgumentParser(description="AA/kerning-isolating parity scorer (prototype)")
    ap.add_argument("--candidate", required=True)
    ap.add_argument("--reference", required=True)
    ap.add_argument("--out-dir", default="docs/research")
    ap.add_argument("--scene", default="revisions")
    ap.add_argument("--scale", type=float, default=2.0)
    ap.add_argument("--threshold", type=int, default=40, help="per-channel hard-mismatch threshold")
    ap.add_argument("--radius", type=int, default=1, help="isotropic AA radius (base mask)")
    ap.add_argument("--kx", type=int, default=3, help="horizontal kerning tolerance (px @device)")
    ap.add_argument("--ky", type=int, default=1, help="vertical tolerance for the kerning pass")
    ap.add_argument("--min-area", type=int, default=80, help="min component area (px) to be REAL")
    ap.add_argument("--thin-px", type=int, default=3, help="bbox min-dim <= this is 'thin' (edge-like)")
    ap.add_argument("--fill-min", type=float, default=0.55, help="fill ratio to rescue a dense short run")
    ap.add_argument("--kern-demote", type=float, default=0.85,
                    help="demote a shape-passing blob to NOISE if this fraction of its pixels slide <=kx px")
    ap.add_argument("--use-ssim", action="store_true",
                    help="ALSO add SSIM anti-correlated patches as REAL (off by default: "
                         "SSIM is noisy on dense text and tends to false-positive on hinting; "
                         "the colour-cluster source is the robust primary signal)")
    ap.add_argument("--ssim-hard", type=float, default=0.0,
                    help="SSIM below this (anti-correlated, compact patch) is a structural defect")
    ap.add_argument("--ssim-min-area", type=int, default=120,
                    help="min area for an SSIM-low structural component")
    ap.add_argument("--ssim-max-aspect", type=float, default=6.0,
                    help="reject SSIM-low blobs wider/taller than this ratio (text rows)")
    ap.add_argument("--top-n", type=int, default=12)
    ap.add_argument("--json", default=None, help="optional path to dump component JSON")
    args = ap.parse_args()

    cand = load_rgb(args.candidate)
    ref = load_rgb(args.reference)
    if cand.shape != ref.shape:
        print(f"ERROR: size mismatch {cand.shape} != {ref.shape}", file=sys.stderr)
        return 2
    h, w, _ = cand.shape
    scale = args.scale

    base = base_mask(cand, ref, args.threshold, args.radius)               # compare.py-equivalent
    kern_forgiven = kerning_forgiven_mask(cand, ref, args.threshold, args.ky, args.kx)

    # Source A: colour-cluster reals (catches recolours, missing/extra glyphs
    # whose ink colour is absent from the reference -- e.g. the toolbar logo).
    comps, labels = classify_components(base, kern_forgiven, scale, args.min_area,
                                        args.thin_px, args.fill_min, args.kern_demote)
    color_real_ids = {c.label for c in comps if c.real}
    color_real_mask = np.isin(labels, list(color_real_ids)) if color_real_ids else np.zeros_like(base)

    # Source B (OPT-IN): SSIM structural reals (catches shape regressions where
    # the two glyphs share ink so the colour rule barely fires -- e.g. a glyph
    # -> tofu). Off by default because SSIM also dips on differently-hinted text
    # and adds false positives in this text-dense UI.
    sm = ssim_map(cand, ref)
    if args.use_ssim:
        ssim_comps, ssim_mask = ssim_structural_components(
            sm, scale, args.ssim_hard, args.ssim_min_area, args.thin_px, args.ssim_max_aspect)
    else:
        ssim_comps, ssim_mask = [], np.zeros_like(base)

    # Union the two evidence sources.
    real_mask = color_real_mask | ssim_mask
    # Merge component lists; drop SSIM comps that just duplicate a colour comp
    # (bbox overlaps an existing colour-real) to avoid double reporting.
    real_color_comps = [c for c in comps if c.real]
    def overlaps(a, b):
        return not (a.x1 <= b.x0 or b.x1 <= a.x0 or a.y1 <= b.y0 or b.y1 <= a.y0)
    merged = list(real_color_comps)
    for sc in ssim_comps:
        if not any(overlaps(sc, cc) for cc in real_color_comps):
            merged.append(sc)

    base_pct = pct(int(base.sum()), base.size)
    # "kern" line: base mismatch still standing after the kerning slide is allowed.
    kmask = base & ~kern_forgiven
    kern_pct = pct(int(kmask.sum()), kmask.size)
    real_pct = pct(int(real_mask.sum()), real_mask.size)
    noise_suppressed = base_pct - real_pct

    ssim_mean = float(sm.mean())
    ssim_low_frac = pct(int((sm < 0.6).sum()), sm.size)

    real_comps = sorted(merged, key=lambda c: c.area, reverse=True)

    # Artefacts.
    os.makedirs(args.out_dir, exist_ok=True)
    heat = os.path.join(args.out_dir, f"compare2-split-{args.scene}.png")
    anno = os.path.join(args.out_dir, f"compare2-annotated-{args.scene}.png")
    make_split_heatmap(cand, base, real_mask).save(heat)
    make_annotated(cand, real_comps).save(anno)

    # Per-region REAL area.
    region_rows = []
    for r in REGIONS:
        x0, y0, x1, y1 = scaled_rect(r, scale, w, h)
        if x1 <= x0 or y1 <= y0:
            region_rows.append((r.name, 0.0, 0.0))
            continue
        sub_b = base[y0:y1, x0:x1]
        sub_r = real_mask[y0:y1, x0:x1]
        region_rows.append((r.name, pct(int(sub_b.sum()), sub_b.size),
                            pct(int(sub_r.sum()), sub_r.size)))

    # Report.
    print()
    print(f"  COMPARE2 (AA/kerning-isolating)  scene={args.scene}  {w}x{h}")
    print(f"  candidate: {args.candidate}")
    print(f"  reference: {args.reference}")
    print(f"  params: thr={args.threshold} base-R={args.radius} kern=+-{args.kx}x/{args.ky}y "
          f"min-area={args.min_area} thin<={args.thin_px} fill>={args.fill_min}")
    print("  " + "-" * 62)
    print(f"  base mismatch (compare.py rule) : {base_pct:6.3f} %   {int(base.sum()):>10,} px")
    print(f"  after kerning slide (+-{args.kx}px)     : {kern_pct:6.3f} %   {int(kmask.sum()):>10,} px")
    print(f"  REAL / structural (clustered)   : {real_pct:6.3f} %   {int(real_mask.sum()):>10,} px")
    print(f"  noise suppressed vs compare.py  : {noise_suppressed:6.3f} %  "
          f"({(noise_suppressed/base_pct*100 if base_pct else 0):.1f}% of base removed)")
    print(f"  SSIM mean={ssim_mean:.4f}   low(<0.6) area={ssim_low_frac:.3f}%")
    print("  " + "-" * 62)
    print(f"  {'region':<16}{'base %':>10}{'REAL %':>10}")
    print("  " + "-" * 62)
    for name, b, rr in region_rows:
        print(f"  {name:<16}{b:>9.3f}%{rr:>9.3f}%")
    print("  " + "-" * 62)
    print(f"  REAL components: {len(real_comps)}  (top {min(args.top_n, len(real_comps))})")
    print(f"  {'#':>3} {'region':<15}{'area':>7}{'bbox(WxH)':>12}{'fill':>6}{'kfrac':>7}  bbox @device (x0,y0,x1,y1)")
    for i, c in enumerate(real_comps[:args.top_n], start=1):
        print(f"  {i:>3} {c.region:<15}{c.area:>7}{c.bbox_w:>5}x{c.bbox_h:<5}{c.fill:>6}{c.kern_frac:>7}  "
              f"({c.x0},{c.y0},{c.x1},{c.y1})  [{c.reason}]")
    print("  " + "-" * 62)
    print(f"  split heatmap -> {heat}")
    print(f"  annotated     -> {anno}")
    print()

    if args.json:
        with open(args.json, "w") as f:
            json.dump({
                "scene": args.scene,
                "base_pct": base_pct,
                "kern_pct": kern_pct,
                "real_pct": real_pct,
                "noise_suppressed_pct": noise_suppressed,
                "ssim_mean": ssim_mean,
                "real_components": [asdict(c) for c in real_comps],
            }, f, indent=2)
        print(f"  json -> {args.json}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
