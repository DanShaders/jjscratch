// INTEGRATION: move to src/ui/diff.rs (or a submodule); render this when the unified/split toggle is in split mode
//
//! SPLIT (side-by-side) diff renderer for jjscratch (ui-spec §5.7).
//!
//! Self-contained bin-local module. The unified renderer lives in
//! `src/ui/diff.rs`; this file is the SPLIT counterpart and is verified in
//! isolation via `src/bin/preview_split.rs`. Integration (the unified/split
//! toggle in the diff toolbar) happens later — see the INTEGRATION note above.
//!
//! ## Layout (§5.7)
//! Two `.split-col` columns (old | new), each `flex:1`, mono `--fs-md`,
//! line-height 18, and its OWN single line-number gutter (`--diff-gutter-w:6ch`,
//! so unlike the unified view there is exactly one number column per side). The
//! left column (`.split-left`) carries a `border-right: --surface0`. Removed
//! lines render on the left (red), added lines on the right (green), context is
//! aligned on both sides, and `.diff-empty` filler rows (bg `--bg-diff-empty`)
//! pad whichever side is shorter within a change block.
//!
//! ## Alignment (mirrors frontend/src/lib/split-view.ts `toSplitView`)
//! Per hunk: emit a header row spanning both columns, then walk the hunk's lines
//! accumulating removes into `dels` and adds into `adds`. On a context line (or
//! end of hunk) we *flush*: emit `max(dels.len, adds.len)` rows pairing
//! `dels[i]` on the left with `adds[i]` on the right, padding the shorter side
//! with `None` (a filler row). The context line itself is then emitted on both
//! sides. This is exactly lightjj's algorithm.

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill, FontData};
use vello::Scene;

use jjscratch::model::{ChangeStatus, CommitDiff, DiffLine, FileDiff, Hunk, LineKind};
use jjscratch::text;
use jjscratch::theme::{font, layout as L, Palette};
use jjscratch::ui::{baseline_for, border_bottom, fill_rect, stroke_round, RenderCtx};

#[allow(dead_code)]
fn main() {}

const PAD_X: f64 = 12.0;

// Per-column gutter geometry (panel-relative, measured from the column's left
// edge). One line-number column (`--diff-gutter-w:6ch`) plus a gutter divider,
// then the code text. Calibrated to JetBrains Mono advances so the columns read
// like lightjj's `.split-col` even though our mono is a touch wider.
const NUM_RIGHT: f64 = 34.0; // right edge of the (single) line-number column
const GUTTER_BORDER_X: f64 = 43.0; // per-line gutter right divider
const CODE_X: f64 = 51.0; // left edge of code text (prefix sits 1ch to its left)

/// One physical row of the split view: an optional line on each side.
struct SplitRow<'a> {
    left: Option<&'a DiffLine>,
    right: Option<&'a DiffLine>,
}

/// A hunk-header row spans both columns; flagged separately from line rows.
enum Row<'a> {
    Header(&'a Hunk),
    Line(SplitRow<'a>),
}

/// Render the diff in SPLIT layout into `rect`.
pub fn render(scene: &mut Scene, rect: Rect, diff: &CommitDiff, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let mut y = rect.y0;
    y = files_bar(scene, rect, y, diff, ctx);

    for file in &diff.files {
        if y > rect.y1 {
            break;
        }
        y = file_block(scene, rect, y, file, ctx);
    }

    scene.pop_layer();
}

// --- §5.2 Files bar (compact, reused shape) --------------------------------

fn files_bar(scene: &mut Scene, rect: Rect, y: f64, diff: &CommitDiff, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let h = 38.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;

    let n = diff.files.len();
    let mut x = r.x0 + PAD_X;
    x = text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_XS, t.text_faint,
        x, baseline_for(cy, font::FS_XS, &ctx.fonts.ui_bold), &format!("FILES ({n})"),
    );
    x += 12.0;
    let pa = format!("+{}", diff.total_added());
    x = text::draw_text(
        scene, &ctx.fonts.mono_bold, font::FS_SM, t.green,
        x, baseline_for(cy, font::FS_SM, &ctx.fonts.mono_bold), &pa,
    );
    x += 6.0;
    let pr = format!("-{}", diff.total_removed());
    let _ = text::draw_text(
        scene, &ctx.fonts.mono_bold, font::FS_SM, t.red,
        x, baseline_for(cy, font::FS_SM, &ctx.fonts.mono_bold), &pr,
    );

    // "Split" mode indicator on the right (mirrors lightjj's active toggle).
    let label = "Split view";
    let lw = text::measure(&ctx.fonts.ui, font::FS_SM, label) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.amber,
        r.x1 - PAD_X - lw, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), label,
    );

    r.y1
}

// --- Per-file block --------------------------------------------------------

fn file_block(scene: &mut Scene, rect: Rect, mut y: f64, file: &FileDiff, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;

    // File header.
    let hh = 7.0 + font::FS_MD as f64 + 8.0 + 7.0;
    let hdr = Rect::new(rect.x0, y, rect.x1, y + hh);
    fill_rect(scene, hdr, t.mantle);
    border_bottom(scene, hdr, t.surface0);
    let cy = hdr.center().y;

    let mut x = hdr.x0 + PAD_X;
    // Collapse chevron.
    x = text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
        x, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), "\u{25be}",
    );
    x += 8.0;

    // File-type badge.
    let (letter, fg, bg) = badge_for(file.status, t);
    let lw = text::measure(&ctx.fonts.ui_bold, font::FS_XS, letter) as f64;
    let bw = lw + 12.0;
    let badge = Rect::new(x, cy - 9.0, x + bw, cy + 9.0);
    fill_rect(scene, badge, bg);
    text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_XS, fg,
        badge.center().x - lw / 2.0, baseline_for(cy, font::FS_XS, &ctx.fonts.ui_bold), letter,
    );
    x = badge.x1 + 10.0;

    // Path: dir faint + name bold.
    let (dir, name) = split_path(&file.path);
    if !dir.is_empty() {
        x = text::draw_text(
            scene, &ctx.fonts.ui, font::FS_MD, t.text_faint,
            x, baseline_for(cy, font::FS_MD, &ctx.fonts.ui), &dir,
        );
    }
    x = text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_MD, t.text,
        x, baseline_for(cy, font::FS_MD, &ctx.fonts.ui_bold), name,
    );

    let _ = x; // path has `flex:1` in lightjj — the stats/actions cluster is
               // laid out from the RIGHT edge, so the path's end x is unused.

    // Right-aligned cluster (lightjj `.diff-file-path{flex:1}` pushes everything
    // after it to the right): +N/-N stats, a reviewed checkbox, then the
    // History / Edit / Discard ghost buttons. Edit is hidden on deleted files.
    let mut rx = hdr.x1 - PAD_X;
    let deleted = matches!(file.status, ChangeStatus::Deleted);
    let buttons: &[(&str, bool)] = if deleted {
        &[("Discard", true), ("History", false)]
    } else {
        &[("Discard", true), ("Edit", false), ("History", false)]
    };
    for (label, danger) in buttons {
        rx = file_action_button(scene, rx, cy, label, *danger, ctx) - 8.0;
    }
    // Reviewed checkbox (empty rounded square).
    rx -= 16.0;
    let cb = Rect::new(rx, cy - 8.0, rx + 16.0, cy + 8.0);
    stroke_round(scene, cb, 3.0, t.surface1, 1.0);
    rx -= 10.0;
    // -N (red) then +N (green), right-to-left.
    if file.removed > 0 {
        let s = format!("-{}", file.removed);
        let w = text::measure(&ctx.fonts.mono_bold, font::FS_SM, &s) as f64;
        rx -= w;
        text::draw_text(
            scene, &ctx.fonts.mono_bold, font::FS_SM, t.red,
            rx, baseline_for(cy, font::FS_SM, &ctx.fonts.mono_bold), &s,
        );
        rx -= 6.0;
    }
    if file.added > 0 {
        let s = format!("+{}", file.added);
        let w = text::measure(&ctx.fonts.mono_bold, font::FS_SM, &s) as f64;
        rx -= w;
        text::draw_text(
            scene, &ctx.fonts.mono_bold, font::FS_SM, t.green,
            rx, baseline_for(cy, font::FS_SM, &ctx.fonts.mono_bold), &s,
        );
    }

    y = hdr.y1;

    // `··· full context` expand row (`.expand-btn`): a full-width centered band
    // with dashed top/bottom borders, shown only when the file has hidden
    // context (mirrors lightjj's `!isExpanded && hasHiddenContext`).
    if has_hidden_context(file) {
        y = full_context_row(scene, rect, y, ctx);
    }

    // The two column rects. The split is at the panel's horizontal midpoint;
    // the left column carries a border-right (--surface0).
    let mid = (rect.x0 + rect.x1) / 2.0;
    let left_col = Rect::new(rect.x0, y, mid, rect.y1);
    let right_col = Rect::new(mid, y, rect.x1, rect.y1);

    // Build the aligned rows, then paint them top-down.
    let rows = build_split_rows(file);
    for row in rows {
        if y > rect.y1 {
            break;
        }
        y = match row {
            Row::Header(hunk) => split_hunk_header(scene, left_col, right_col, mid, y, hunk, ctx),
            Row::Line(sr) => split_line(scene, left_col, right_col, mid, y, &sr, ctx),
        };
    }

    // Bottom rule under the file.
    fill_rect(scene, Rect::new(rect.x0, y, rect.x1, y + 1.0), t.surface0);
    y + 1.0
}

/// Mirror of lightjj's `hasHiddenContext`: true when the file's hunks don't
/// cover its full span (context above the first hunk or gaps between hunks).
fn has_hidden_context(file: &FileDiff) -> bool {
    let Some(first) = file.hunks.first() else {
        return false;
    };
    if first.new_start > 1 {
        return true;
    }
    for w in file.hunks.windows(2) {
        if w[1].new_start > w[0].new_start + w[0].new_len {
            return true;
        }
    }
    false
}

/// The `··· full context` expand band (`.expand-btn`): full width, centered
/// `···` dots + "full context" label, text-faint, with dashed top/bottom rules.
fn full_context_row(scene: &mut Scene, rect: Rect, y: f64, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    // padding 2px top/bottom around an fs-sm line.
    let h = 2.0 + L::DIFF_LINE_H + 2.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    fill_rect(scene, r, t.base);
    // Dashed top & bottom borders (--border-hunk-header ~= surface1).
    dashed_hline(scene, rect.x0, rect.x1, y, t.surface1);
    dashed_hline(scene, rect.x0, rect.x1, y + h - 1.0, t.surface1);

    let cy = r.center().y;
    let dots = "\u{22ef}"; // ⋯
    let label = "full context";
    let dw = text::measure(&ctx.fonts.ui, font::FS_SM, dots) as f64;
    let gap = 6.0;
    let lw = text::measure(&ctx.fonts.ui, font::FS_SM, label) as f64;
    let total = dw + gap + lw;
    let start_x = r.center().x - total / 2.0;
    let baseline = baseline_for(cy, font::FS_SM, &ctx.fonts.ui);
    let x = text::draw_text(scene, &ctx.fonts.ui, font::FS_SM, t.text_faint, start_x, baseline, dots);
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
        x + gap, baseline, label,
    );
    r.y1
}

/// A 1px dashed horizontal rule (approximates CSS `border: 1px dashed`).
fn dashed_hline(scene: &mut Scene, x0: f64, x1: f64, y: f64, color: Color) {
    let dash = 4.0;
    let gap = 3.0;
    let mut x = x0;
    while x < x1 {
        let xe = (x + dash).min(x1);
        fill_rect(scene, Rect::new(x, y, xe, y + 1.0), color);
        x += dash + gap;
    }
}

/// Mirror of `toSplitView` (frontend/src/lib/split-view.ts): pair up
/// removes-left / adds-right within each hunk, emit context on both sides, and
/// pad the shorter side with `None` (filler rows).
fn build_split_rows(file: &FileDiff) -> Vec<Row<'_>> {
    let mut out: Vec<Row> = Vec::new();
    for hunk in &file.hunks {
        out.push(Row::Header(hunk));

        let mut dels: Vec<&DiffLine> = Vec::new();
        let mut adds: Vec<&DiffLine> = Vec::new();

        // Flush a pending change block: pair dels[i] (left) with adds[i] (right),
        // padding the shorter side. Mirrors split-view.ts's `flush()`.
        fn flush<'a>(
            out: &mut Vec<Row<'a>>,
            dels: &mut Vec<&'a DiffLine>,
            adds: &mut Vec<&'a DiffLine>,
        ) {
            let max = dels.len().max(adds.len());
            for i in 0..max {
                out.push(Row::Line(SplitRow {
                    left: dels.get(i).copied(),
                    right: adds.get(i).copied(),
                }));
            }
            dels.clear();
            adds.clear();
        }

        for line in &hunk.lines {
            match line.kind {
                LineKind::Remove => dels.push(line),
                LineKind::Add => adds.push(line),
                LineKind::Context => {
                    flush(&mut out, &mut dels, &mut adds);
                    out.push(Row::Line(SplitRow { left: Some(line), right: Some(line) }));
                }
            }
        }
        flush(&mut out, &mut dels, &mut adds);
    }
    out
}

/// A hunk-header row spanning both columns (`-a,b +c,d`).
fn split_hunk_header(
    scene: &mut Scene,
    left_col: Rect,
    right_col: Rect,
    mid: f64,
    y: f64,
    hunk: &Hunk,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let h = L::DIFF_LINE_H;
    let l = Rect::new(left_col.x0, y, left_col.x1, y + h);
    let r = Rect::new(right_col.x0, y, right_col.x1, y + h);
    fill_rect(scene, l, t.bg_hunk_header);
    fill_rect(scene, r, t.bg_hunk_header);
    border_bottom(scene, l, t.surface1);
    border_bottom(scene, r, t.surface1);
    // Left-column border-right (.split-left).
    fill_rect(scene, Rect::new(mid - 1.0, y, mid, y + h), t.surface0);

    let cy = l.center().y;
    // lightjj shows the FULL `@@ -a,b +c,d @@` header string on BOTH columns
    // (`.diff-hunk-header` renders `hunk.header` verbatim, font-size --fs-sm,
    // color --overlay0, padding-left 12px). See DiffFileView.svelte split branch.
    let header = format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len
    );
    let baseline = baseline_for(cy, font::FS_SM, &ctx.fonts.mono);
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_SM, t.overlay0,
        l.x0 + PAD_X, baseline, &header,
    );
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_SM, t.overlay0,
        r.x0 + PAD_X, baseline, &header,
    );
    l.y1
}

/// One physical split row: paint each side (or a `.diff-empty` filler).
fn split_line(
    scene: &mut Scene,
    left_col: Rect,
    right_col: Rect,
    mid: f64,
    y: f64,
    row: &SplitRow,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let h = L::DIFF_LINE_H;

    let l = Rect::new(left_col.x0, y, left_col.x1, y + h);
    let r = Rect::new(right_col.x0, y, right_col.x1, y + h);

    // OLD side (left): context shows old_no; a removed line is red; a `None`
    // side is a `.diff-empty` filler.
    paint_side(scene, l, row.left, Side::Old, ctx);
    // NEW side (right): context shows new_no; an added line is green.
    paint_side(scene, r, row.right, Side::New, ctx);

    // Left-column border-right (.split-left, border-right --surface0).
    fill_rect(scene, Rect::new(mid - 1.0, y, mid, y + h), t.surface0);

    l.y1
}

#[derive(Clone, Copy, PartialEq)]
enum Side {
    Old,
    New,
}

/// `.diff-empty` filler color. lightjj's `--bg-diff-empty` is
/// `color-mix(in srgb, var(--text) 2%, transparent)` — i.e. `text` at 2% alpha
/// laid over the panel `base`. We composite it once here so the filler reads as
/// the same near-base wash lightjj paints.
fn empty_bg(t: &Palette) -> Color {
    mix(t.base, t.text, 0.02)
}

/// Composite `over` at `alpha` on top of opaque `under` (srgb-ish lerp).
fn mix(under: Color, over: Color, alpha: f32) -> Color {
    let u = under.to_rgba8();
    let o = over.to_rgba8();
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * alpha) as u8;
    Color::from_rgb8(lerp(u.r, o.r), lerp(u.g, o.g), lerp(u.b, o.b))
}

fn paint_side(scene: &mut Scene, cell: Rect, line: Option<&DiffLine>, side: Side, ctx: &RenderCtx) {
    let t = ctx.theme;
    let cy = cell.center().y;

    let Some(line) = line else {
        // `.diff-empty` filler row.
        fill_rect(scene, cell, empty_bg(t));
        return;
    };

    let (bg, base_fg, prefix, border): (Option<Color>, Color, &str, Option<Color>) =
        match line.kind {
            LineKind::Add => (Some(t.diff_add_bg), t.green, "+", Some(t.green)),
            LineKind::Remove => (Some(t.diff_remove_bg), t.red, "-", Some(t.red)),
            LineKind::Context => (None, t.subtext0, " ", None),
        };

    if let Some(bg) = bg {
        fill_rect(scene, cell, bg);
    }
    if let Some(bc) = border {
        fill_rect(scene, Rect::new(cell.x0, cell.y0, cell.x0 + 3.0, cell.y1), bc);
    }

    // Single line-number column for this side.
    let num = match side {
        Side::Old => line.old_no,
        Side::New => line.new_no,
    };
    let num_s = num.map(|n| n.to_string()).unwrap_or_default();
    let num_sz = font::FS_SM;
    let num_right = cell.x0 + NUM_RIGHT;
    let nw = text::measure(&ctx.fonts.mono, num_sz, &num_s) as f64;
    text::draw_text(
        scene, &ctx.fonts.mono, num_sz, t.text_faint,
        num_right - nw, baseline_for(cy, num_sz, &ctx.fonts.mono), &num_s,
    );

    // Per-column gutter divider (only on context lines, like the unified view's
    // `--line-gutter-border`).
    if matches!(line.kind, LineKind::Context) {
        let bx = cell.x0 + GUTTER_BORDER_X;
        fill_rect(scene, Rect::new(bx, cell.y0, bx + 1.0, cell.y1), t.surface0);
    }

    // Code cell: dim prefix then the syntax-highlighted source.
    let text_x = cell.x0 + CODE_X;
    let pw = text::measure(&ctx.fonts.mono, font::FS_MD, prefix) as f64;
    let baseline = baseline_for(cy, font::FS_MD, &ctx.fonts.mono);
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_MD, base_fg.multiply_alpha(0.5),
        text_x - pw, baseline, prefix,
    );
    draw_highlighted(
        scene, &ctx.fonts.mono, font::FS_MD, text_x, baseline,
        &line.text, line.kind, cell.x1 - 6.0, t,
    );
}

// --- helpers ---------------------------------------------------------------

fn badge_for(status: ChangeStatus, t: &Palette) -> (&'static str, Color, Color) {
    match status {
        ChangeStatus::Added => ("A", t.green, t.green.multiply_alpha(0.12)),
        ChangeStatus::Modified => ("M", t.amber, t.amber.multiply_alpha(0.12)),
        ChangeStatus::Deleted => ("D", t.red, t.red.multiply_alpha(0.12)),
        ChangeStatus::Renamed => ("R", t.amber, t.amber.multiply_alpha(0.12)),
        ChangeStatus::Copied => ("C", t.red, t.red.multiply_alpha(0.12)),
    }
}

/// A `.btn.btn-sm` ghost pill in the file header, laid out right-to-left from
/// `right_x`. Returns the pill's LEFT x. `danger` ⇒ `.btn-danger` (red outline,
/// red text); otherwise the neutral ghost (surface1 border, subtext0 text).
fn file_action_button(
    scene: &mut Scene,
    right_x: f64,
    cy: f64,
    label: &str,
    danger: bool,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    // .btn-sm: padding 2px 8px, fs-xs.
    let pad_x = 8.0;
    let tw = text::measure(&ctx.fonts.ui, font::FS_XS, label) as f64;
    let bw = tw + pad_x * 2.0;
    let bh = 18.0;
    let r = Rect::new(right_x - bw, cy - bh / 2.0, right_x, cy + bh / 2.0);
    let (border, fg) = if danger {
        (t.red, t.red)
    } else {
        (t.surface1, t.subtext0)
    };
    stroke_round(scene, r, 4.0, border, 1.0);
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_XS, fg,
        r.x0 + pad_x, baseline_for(cy, font::FS_XS, &ctx.fonts.ui), label,
    );
    r.x0
}

/// Split into (dir-with-trailing-slash, name).
fn split_path(path: &str) -> (String, &str) {
    match path.rfind('/') {
        Some(i) => (path[..=i].to_string(), &path[i + 1..]),
        None => (String::new(), path),
    }
}

// --- Lightweight Rust syntax highlighting (self-contained) ------------------

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
    "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match",
    "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "type", "union", "unsafe", "use", "where", "while",
];
const ATOMS: &[&str] = &["true", "false", "None", "Some", "Ok", "Err"];

/// Draw `text` as syntax-colored runs, clipped at `clip_x`.
fn draw_highlighted(
    scene: &mut Scene,
    font_data: &FontData,
    size: f32,
    x: f64,
    baseline_y: f64,
    text_str: &str,
    kind: LineKind,
    clip_x: f64,
    t: &Palette,
) {
    let base = match kind {
        LineKind::Context => t.text.multiply_alpha(0.7),
        LineKind::Add => t.green,
        LineKind::Remove => t.red,
    };
    let mut pen = x;
    for (s, color) in tokenize(text_str, t, base) {
        if pen >= clip_x {
            break;
        }
        pen = text::draw_text(scene, font_data, size, color, pen, baseline_y, &s);
    }
}

/// Tokenize one line of Rust-ish source into colored runs.
fn tokenize(line: &str, t: &Palette, base: Color) -> Vec<(String, Color)> {
    let mut out: Vec<(String, Color)> = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut i = 0;

    fn push(out: &mut Vec<(String, Color)>, s: String, c: Color) {
        if !s.is_empty() {
            out.push((s, c));
        }
    }

    while i < n {
        let c = chars[i];

        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            push(&mut out, chars[i..].iter().collect(), t.syn_comment);
            break;
        }
        if c == '"' {
            let mut j = i + 1;
            let mut buf = String::from(c);
            while j < n {
                buf.push(chars[j]);
                if chars[j] == '\\' && j + 1 < n {
                    buf.push(chars[j + 1]);
                    j += 2;
                    continue;
                }
                if chars[j] == '"' {
                    j += 1;
                    break;
                }
                j += 1;
            }
            push(&mut out, buf, t.syn_string);
            i = j;
            continue;
        }
        if c.is_whitespace() {
            let mut j = i;
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            push(&mut out, chars[i..j].iter().collect(), base);
            i = j;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            push(&mut out, chars[i..j].iter().collect(), t.syn_number);
            i = j;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let is_macro = j < n && chars[j] == '!';
            let is_call = j < n && chars[j] == '(';
            let color = if KEYWORDS.contains(&word.as_str()) {
                t.syn_keyword
            } else if ATOMS.contains(&word.as_str()) {
                t.syn_atom
            } else if is_macro {
                t.syn_keyword
            } else if word.chars().next().map(char::is_uppercase).unwrap_or(false) {
                t.syn_type
            } else if is_call {
                t.syn_property
            } else {
                base
            };
            push(&mut out, word, color);
            i = j;
            continue;
        }

        const PUNCT: &str = ":;,.(){}[]<>+-*/%=&|!?^~@#";
        if PUNCT.contains(c) {
            let mut j = i;
            while j < n && PUNCT.contains(chars[j]) {
                if chars[j] == '/' && j + 1 < n && chars[j + 1] == '/' {
                    break;
                }
                j += 1;
            }
            let run: String = chars[i..j].iter().collect();
            let color = if run.chars().all(|c| "(){}[];,.".contains(c)) {
                t.syn_punct
            } else if run.chars().all(|c| "#@".contains(c)) {
                t.syn_keyword
            } else {
                t.syn_operator
            };
            push(&mut out, run, color);
            i = j;
            continue;
        }

        push(&mut out, c.to_string(), base);
        i += 1;
    }

    if out.is_empty() {
        out.push((String::new(), base));
    }
    out
}
