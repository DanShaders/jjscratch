#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["pillow"]
# ///
"""
sizecheck_check.py — programmatic layout-breakage checker for jjscratch renders.

Loads a single rendered PNG and probes expected chrome bands (toolbar / tab bar /
statusbar / panel divider / content) at theme- and scale-aware offsets. Emits a
PASS line, or one or more FAIL/WARN lines describing the specific breakage, so the
size matrix can be triaged without eyeballing every screenshot.

This is a TEST tool: it never edits the renderer. It only reads pixels. Lives in
scripts/ (not tools/, which is .gitignored) so it is committed with the harness.

Layout reference (src/theme.rs::layout, src/ui.rs::FrameLayout::compute), in
LOGICAL px (multiply by `scale` for device px):
  toolbar    y 0..34          crust
  tab bar    y 34..60         base
  body       y 60..(H-24)
  statusbar  y (H-24)..H      crust
  divider    vertical, x = panel_w (default 420, clamped [280,600]), width 4, surface1
The panel does NOT auto-shrink below its default 420 for narrow windows; it only
clamps to [280,600]. So for W < ~424 the right column can vanish (we detect/flag).

Usage:
  uv run scripts/sizecheck_check.py <png> --scale N [--theme dark|light] [--json]
Exit code: 0 if PASS/WARN (no FAILs), 1 if any FAIL.
"""
import argparse
import json
import sys
from PIL import Image

# Palette colors that matter for band probing (src/theme.rs). RGB tuples.
DARK = {
    "base": (0x0F, 0x0F, 0x13),
    "crust": (0x0A, 0x0A, 0x0E),
    "surface1": (0x1E, 0x1E, 0x21),  # divider fill
    "surface2": (0x4E, 0x4E, 0x58),
}
LIGHT = {
    "base": (0xF8, 0xF8, 0xF6),
    "crust": (0xEE, 0xEE, 0xEC),
    "surface1": (0xE8, 0xE8, 0xE7),  # divider fill
    "surface2": (0xA1, 0xA1, 0xAA),
}

# theme::layout constants (logical px).
TOOLBAR_H = 34
TAB_BAR_H = 26
STATUSBAR_H = 24
BODY_TOP = TOOLBAR_H + TAB_BAR_H  # 60
PANEL_DEFAULT_W = 420
PANEL_MIN_W = 280
PANEL_MAX_W = 600
DIVIDER_W = 4


def close(a, b, tol=10):
    return all(abs(x - y) <= tol for x, y in zip(a, b))


def near_any(c, colors, tol=10):
    return any(close(c, t, tol) for t in colors)


class Probe:
    def __init__(self, path, scale, theme):
        self.im = Image.open(path).convert("RGB")
        self.W, self.H = self.im.size
        self.scale = scale
        self.pal = LIGHT if theme == "light" else DARK
        self.lw = self.W // scale
        self.lh = self.H // scale

    def px(self, lx, ly):
        """Sample at logical (lx, ly) -> device px (center of the scaled cell)."""
        dx = int(lx * self.scale + self.scale / 2)
        dy = int(ly * self.scale + self.scale / 2)
        dx = max(0, min(dx, self.W - 1))
        dy = max(0, min(dy, self.H - 1))
        return self.im.getpixel((dx, dy))

    def row_band(self, lx_list, ly):
        return [self.px(lx, ly) for lx in lx_list]


def status_y(lh):
    return lh - STATUSBAR_H // 2


def _scan_divider_row(p, ly):
    """One horizontal scan at logical y; return the divider's left x or None."""
    pal = p.pal
    lo = max(0, PANEL_MIN_W - 4)
    hi = min(p.lw - 1, PANEL_MAX_W + DIVIDER_W + 4)
    run_start = None
    for lx in range(lo, hi + 1):
        is_div = close(p.px(lx, ly), pal["surface1"], tol=8)
        if is_div and run_start is None:
            run_start = lx
        elif not is_div and run_start is not None:
            if (lx - run_start) >= DIVIDER_W - 1:
                return run_start
            run_start = None
    if run_start is not None and (hi + 1 - run_start) >= DIVIDER_W - 1:
        return run_start
    return None


def find_divider_x(p):
    """Scan the body for a vertical surface1 band ~DIVIDER_W wide; return its
    logical left x, or None. Restricted to the plausible (clamped) panel range so
    incidental surface1 content elsewhere is not mistaken for the divider.

    Probes SEVERAL y-rows (including one high in the body, below the panel header)
    so an open bottom drawer — which shortens the body — doesn't hide the divider.
    The TRUE divider is a continuous vertical band, so it shows up at the SAME x
    in every probed row; incidental surface1 content (graph fills, hover rows)
    appears in at most one. We therefore return the x seen in the most rows
    (ties -> rightmost, since the divider sits right of the panel content)."""
    body_h = p.lh - BODY_TOP - STATUSBAR_H
    if body_h <= 0:
        return None
    candidates = [
        BODY_TOP + min(110, max(2, body_h // 2)),   # high: clears any open drawer
        BODY_TOP + max(2, body_h // 5),
        BODY_TOP + max(2, body_h // 2),
        BODY_TOP + min(body_h - 2, (body_h * 3) // 4),
    ]
    seen = {}  # x -> count of rows that found a divider band at ~x
    for ly in candidates:
        ly = min(ly, p.lh - STATUSBAR_H - 1)
        x = _scan_divider_row(p, ly)
        if x is None:
            continue
        # Bucket near-equal x (sub-pixel/aa wobble) together.
        key = next((k for k in seen if abs(k - x) <= 2), x)
        seen[key] = seen.get(key, 0) + 1
    if not seen:
        return None
    best = max(seen, key=lambda k: (seen[k], k))
    # Require the winner to appear in >=2 rows when we have >=2 successful probes,
    # so a lone incidental match cannot masquerade as the divider.
    if sum(seen.values()) >= 2 and seen[best] < 2:
        return None
    return best


def check(path, scale, theme):
    """Return (status, messages). status in {PASS, WARN, FAIL}."""
    fails, warns = [], []
    try:
        p = Probe(path, scale, theme)
    except Exception as e:
        return "FAIL", [f"cannot open PNG: {e}"]

    pal = p.pal
    W, H, lw, lh = p.W, p.H, p.lw, p.lh

    # 0. degenerate-image guard: coarse grid; flag if all one color.
    grid = [p.im.getpixel((W * fx // 8, H * fy // 8))
            for fy in range(1, 8) for fx in range(1, 8)]
    uniq = set(grid)
    if len(uniq) == 1:
        return "FAIL", [f"degenerate: entire sampled grid is flat {grid[0]}"]
    if len(uniq) <= 2:
        warns.append(f"near-flat image: only {len(uniq)} distinct sampled colors {uniq}")

    # 1. toolbar band (top): crust.
    ty = TOOLBAR_H // 2
    tb = p.row_band([2, lw // 2, lw - 3], ty)
    if not any(near_any(c, [pal["crust"]]) for c in tb):
        fails.append(f"toolbar band missing crust at y={ty} (got {tb})")

    # 2. tab bar band: base (active-tab highlight may tint it -> warn only).
    tabby = TOOLBAR_H + TAB_BAR_H // 2
    tab = p.row_band([2, lw // 2, lw - 3], tabby)
    if not any(near_any(c, [pal["base"]]) for c in tab):
        warns.append(f"tab-bar band not base at y={tabby} (got {tab})")

    # 3. statusbar band (bottom): crust. Clean columns avoid statusbar glyphs.
    sy = status_y(lh)
    sb = p.row_band([2, lw - 3, lw // 2], sy)
    if not any(near_any(c, [pal["crust"]]) for c in sb):
        fails.append(f"statusbar band missing crust at y={sy} (got {sb}); "
                     "drawer may overlap statusbar or layout collapsed")

    # 4. panel divider present (surface1 vertical band in body).
    div_x = find_divider_x(p)
    if div_x is None:
        if lw < PANEL_MIN_W + DIVIDER_W:
            warns.append(f"no divider (window width {lw} < panel-min "
                         f"{PANEL_MIN_W}+{DIVIDER_W}); expected at tiny width")
        elif lh - BODY_TOP - STATUSBAR_H < 10:
            warns.append(f"no divider (body height {lh-BODY_TOP-STATUSBAR_H}px too short)")
        else:
            fails.append(f"panel divider not found (window {lw}x{lh}); "
                         "left/right split may have collapsed")
    else:
        right_w = lw - (div_x + DIVIDER_W)
        if right_w <= 0:
            fails.append(f"right content column zero/negative width ({right_w}px); "
                         f"divider at x={div_x} off-screen")
        elif right_w < 120:
            warns.append(f"right content column only {right_w}px wide "
                         f"(panel pinned at x={div_x}, no auto-shrink) — squeezed")

    # 5. left panel body not entirely empty.
    if lh - BODY_TOP - STATUSBAR_H > 40:
        body, y0, y1 = [], BODY_TOP + 5, lh - STATUSBAR_H - 5
        step = max(1, (y1 - y0) // 40)
        for ly in range(y0, y1, step):
            for lx in (8, 20, 40, 80, 140):
                if lx < lw:
                    body.append(p.px(lx, ly))
        if body and sum(1 for c in body if not close(c, pal["base"], 6)) == 0:
            fails.append("left panel body is 100% base color — graph/content did not render")

    # 6. statusbar not overdrawn by drawer: bottom row predominantly crust.
    bottom = [p.px(lx, lh - 1) for lx in (2, lw // 4, lw // 2, 3 * lw // 4, lw - 3)]
    if sum(1 for c in bottom if near_any(c, [pal["crust"]])) < 3:
        warns.append(f"bottom edge not predominantly crust ({bottom}); "
                     "possible overlap into statusbar")

    if fails:
        return "FAIL", fails + warns
    if warns:
        return "WARN", warns
    return "PASS", []


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("png")
    ap.add_argument("--scale", type=int, default=1)
    ap.add_argument("--theme", choices=["dark", "light"], default="dark")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    status, msgs = check(args.png, args.scale, args.theme)
    if args.json:
        print(json.dumps({"png": args.png, "scale": args.scale, "theme": args.theme,
                          "status": status, "messages": msgs}))
    else:
        print(f"{status}: {args.png}")
        for m in msgs:
            print(f"   - {m}")
    sys.exit(1 if status == "FAIL" else 0)


if __name__ == "__main__":
    main()
