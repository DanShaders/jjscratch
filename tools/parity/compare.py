# /// script
# requires-python = ">=3.9"
# dependencies = ["pillow>=10", "numpy>=1.26"]
# ///
"""Pixel-parity scorer for jjscratch vs the real lightjj reference screenshots.

Loads two same-size RGB(A) PNGs (a jjscratch render and the matching lightjj
reference), computes an anti-aliasing-tolerant per-pixel mismatch, and reports an
overall mismatch %, per-region mismatch %, and writes a heatmap + side-by-side
PNG.

Run via `uv run`:

    uv run tools/parity/compare.py \
        --candidate docs/parity/jjscratch-revisions.png \
        --reference docs/reference/revisions.png \
        --out-dir   docs/parity \
        --scene     revisions \
        --scale     2

------------------------------------------------------------------------------
AA-TOLERANCE RULE (documented exactly):
------------------------------------------------------------------------------
A pixel at (x, y) is counted as a *mismatch* only if BOTH hold:

  1. HARD DELTA: max over the 3 colour channels of
         |candidate[x,y] - reference[x,y]|  >  THRESHOLD            (default 40/255)
     i.e. the colours differ by more than the threshold on at least one channel.

  2. NO AA-EXCUSE in either direction. We forgive a differing pixel when it is
     merely an anti-aliased / sub-pixel-shifted edge. Concretely, the pixel is
     forgiven (NOT counted) if EITHER:
       (a) some reference pixel within a (2*R+1)^2 neighborhood (default R=1, a
           3x3 window) is within THRESHOLD of candidate[x,y]  -- the candidate's
           colour exists nearby in the reference (edge shifted by <=1px), OR
       (b) some candidate pixel within the same neighborhood is within THRESHOLD
           of reference[x,y]                                   -- symmetric.

In words: a pixel only counts as a real mismatch if the candidate's colour is
absent from the reference's local neighborhood AND vice-versa. A clean edge that
is anti-aliased differently, or shifted by up to one device pixel (kerning /
hinting jitter at @2x), is fully tolerated; a genuinely wrong colour, missing
glyph, or >1px structural shift is not.

The neighborhood check is implemented as a vectorised min-over-shifts of the
per-channel Chebyshev distance, so it stays fast on 2560x1600 images.
"""

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass

import numpy as np
from PIL import Image


@dataclass(frozen=True)
class Region:
    name: str
    # Logical (pre-scale) rect in the 1280x800 layout: (x0, y0, x1, y1).
    x0: float
    y0: float
    x1: float
    y1: float


# Logical-pixel rects from src/ui.rs FrameLayout::compute(1280, 800, 420) and
# docs/spec/ui-spec.md §3. body_top = toolbar(34)+tabbar(26) = 60;
# body_bot = height - statusbar(24) = 776; panel 420 wide; divider 4px.
LOGICAL_W = 1280.0
LOGICAL_H = 800.0
PANEL_W = 420.0
DIVIDER_W = 4.0
TOOLBAR_H = 34.0
TABBAR_H = 26.0
STATUSBAR_H = 24.0
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


def load_rgb(path: str) -> np.ndarray:
    img = Image.open(path).convert("RGB")
    return np.asarray(img, dtype=np.int16)  # HxWx3, signed for diffs


def chebyshev(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    """Per-pixel max-over-channels |a-b| -> HxW int16."""
    return np.abs(a - b).max(axis=2)


def neighborhood_min_dist(target: np.ndarray, source: np.ndarray, radius: int) -> np.ndarray:
    """For each pixel, the minimum Chebyshev colour distance between
    target[x,y] and any source pixel within the (2r+1)^2 window centered at
    (x,y). Vectorised: for each (dy,dx) shift, roll `source`, compute the
    per-pixel channel-max distance to `target`, and keep the running min.

    Edges use replicate padding (via np.pad + slicing) so border pixels don't
    falsely match wrapped-around content.
    """
    h, w, _ = target.shape
    best = np.full((h, w), 255, dtype=np.int16)
    padded = np.pad(source, ((radius, radius), (radius, radius), (0, 0)), mode="edge")
    for dy in range(-radius, radius + 1):
        for dx in range(-radius, radius + 1):
            # window slice of padded source aligned to target's (x,y)
            sy = dy + radius
            sx = dx + radius
            shifted = padded[sy : sy + h, sx : sx + w, :]
            dist = np.abs(target - shifted).max(axis=2)
            best = np.minimum(best, dist)
    return best


def compute_mismatch(cand: np.ndarray, ref: np.ndarray, threshold: int, radius: int) -> np.ndarray:
    """Boolean HxW mask: True where the pixel is a real (non-AA) mismatch."""
    hard = chebyshev(cand, ref) > threshold

    # AA excuse (a): candidate colour present in reference neighborhood.
    cand_in_ref = neighborhood_min_dist(cand, ref, radius) <= threshold
    # AA excuse (b): reference colour present in candidate neighborhood.
    ref_in_cand = neighborhood_min_dist(ref, cand, radius) <= threshold

    forgiven = cand_in_ref | ref_in_cand
    return hard & ~forgiven


def scaled_rect(r: Region, scale: float, w: int, h: int):
    x0 = int(round(r.x0 * scale))
    y0 = int(round(r.y0 * scale))
    x1 = int(round(r.x1 * scale))
    y1 = int(round(r.y1 * scale))
    x0 = max(0, min(x0, w))
    x1 = max(0, min(x1, w))
    y0 = max(0, min(y0, h))
    y1 = max(0, min(y1, h))
    return x0, y0, x1, y1


def pct(num: int, den: int) -> float:
    return 100.0 * num / den if den else 0.0


def make_heatmap(cand: np.ndarray, mask: np.ndarray) -> Image.Image:
    """Dimmed jjscratch render with mismatches painted solid red."""
    dim = (cand.astype(np.float32) * 0.35).astype(np.uint8)
    out = dim.copy()
    out[mask] = (255, 0, 0)
    return Image.fromarray(out, "RGB")


def make_side_by_side(cand: np.ndarray, ref: np.ndarray) -> Image.Image:
    gap = 16
    h = cand.shape[0]
    w = cand.shape[1]
    canvas = np.zeros((h, w * 2 + gap, 3), dtype=np.uint8)
    canvas[:, :w] = cand.astype(np.uint8)
    canvas[:, w + gap :] = ref.astype(np.uint8)
    return Image.fromarray(canvas, "RGB")


def main() -> int:
    ap = argparse.ArgumentParser(description="jjscratch <-> lightjj pixel-parity scorer")
    ap.add_argument("--candidate", required=True, help="jjscratch render PNG")
    ap.add_argument("--reference", required=True, help="lightjj reference PNG")
    ap.add_argument("--out-dir", default="docs/parity")
    ap.add_argument("--scene", default="revisions")
    ap.add_argument("--scale", type=float, default=2.0, help="device scale of the images vs logical 1280x800")
    ap.add_argument("--threshold", type=int, default=40, help="per-channel hard-mismatch threshold (0-255)")
    ap.add_argument("--radius", type=int, default=1, help="AA-tolerance neighborhood radius in px")
    args = ap.parse_args()

    cand = load_rgb(args.candidate)
    ref = load_rgb(args.reference)
    if cand.shape != ref.shape:
        print(
            f"ERROR: size mismatch: candidate {cand.shape[1]}x{cand.shape[0]} "
            f"!= reference {ref.shape[1]}x{ref.shape[0]}",
            file=sys.stderr,
        )
        return 2
    h, w, _ = cand.shape

    mask = compute_mismatch(cand, ref, args.threshold, args.radius)

    overall = pct(int(mask.sum()), mask.size)

    # Per-region scores.
    rows = []
    for r in REGIONS:
        x0, y0, x1, y1 = scaled_rect(r, args.scale, w, h)
        if x1 <= x0 or y1 <= y0:
            rows.append((r.name, 0.0, 0))
            continue
        sub = mask[y0:y1, x0:x1]
        rows.append((r.name, pct(int(sub.sum()), sub.size), sub.size))

    # Write artefacts.
    import os

    os.makedirs(args.out_dir, exist_ok=True)
    heat_path = os.path.join(args.out_dir, f"heatmap-{args.scene}.png")
    sbs_path = os.path.join(args.out_dir, f"sidebyside-{args.scene}.png")
    make_heatmap(cand, mask).save(heat_path)
    make_side_by_side(cand, ref).save(sbs_path)

    # Score table.
    print()
    print(f"  PIXEL-PARITY  scene={args.scene}  {w}x{h}  "
          f"threshold={args.threshold}/255  AA-radius={args.radius}px")
    print(f"  candidate: {args.candidate}")
    print(f"  reference: {args.reference}")
    print("  " + "-" * 46)
    print(f"  {'region':<18}{'mismatch %':>12}{'px':>14}")
    print("  " + "-" * 46)
    for name, p, n in rows:
        print(f"  {name:<18}{p:>11.2f}%{n:>14,}")
    print("  " + "-" * 46)
    print(f"  {'OVERALL':<18}{overall:>11.2f}%{mask.size:>14,}")
    print("  " + "-" * 46)
    print(f"  heatmap     -> {heat_path}")
    print(f"  side-by-side-> {sbs_path}")
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
