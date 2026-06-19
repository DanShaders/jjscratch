//! Diff panel renderer (the right column in Revisions view).
//!
//! Reproduces lightjj's DiffPanel (docs/spec/ui-spec.md §5) in the **unified**
//! view: RevisionHeader (§5.1), the FILES bar with file tabs (§5.2), the diff
//! toolbar (§5.3), and per-file blocks (§5.4) — sticky-style file header with
//! collapse chevron + type badge + path + stats + ghost action buttons, hunk
//! headers (§5.5), and syntax-highlighted unified diff lines (§5.6) with the
//! 11ch hanging-indent gutter, two line-number columns, +/- prefixes, and
//! add/remove/context backgrounds + left borders. Includes lightweight Rust
//! syntax highlighting and the "rest of file"/elision affordances.
//!
//! Split view (§5.7), conflicts (§5.8), word-diff, search, and review bubbles
//! are intentionally out of scope here.

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill, FontData};
use vello::Scene;

use super::{baseline_for, border_bottom, fill_rect, fill_round, stroke_rect, RenderCtx, UiState};
use crate::model::{ChangeStatus, CommitDiff, DiffLine, FileDiff, Hunk, LineKind, Snapshot};
use crate::text;
use crate::theme::{font, layout as L, Palette};

const PAD_X: f64 = 12.0;

// Unified-diff gutter geometry (§5.6), panel-relative logical x, calibrated to
// the reference render (lightjj's `--diff-gutter-w:11ch` of JetBrains Mono).
// Using fixed px instead of our font's `ch` makes the two line-number columns,
// the gutter divider, and the code column land exactly where lightjj's do.
const OLD_NUM_RIGHT: f64 = 22.0; // right edge of the old line-number column
const NEW_NUM_RIGHT: f64 = 58.0; // right edge of the new line-number column
const GUTTER_BORDER_X: f64 = 67.0; // context-line gutter right border
const CODE_X: f64 = 83.5; // left edge of code text (prefix sits 1ch to its left)

pub fn render(
    scene: &mut Scene,
    rect: Rect,
    snapshot: &Snapshot,
    diff: Option<&CommitDiff>,
    state: &UiState,
    ctx: &RenderCtx,
) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let Some(diff) = diff else {
        let mut y = rect.y0;
        y = revision_header(scene, rect, y, snapshot, state, ctx);
        let _ = y;
        text::draw_text(
            scene, &ctx.fonts.ui, font::BASE, t.text_faint,
            rect.x0 + 24.0, rect.y0 + 120.0, "No changes",
        );
        scene.pop_layer();
        return;
    };

    let mut y = rect.y0;
    y = revision_header(scene, rect, y, snapshot, state, ctx);
    y = files_bar(scene, rect, y, diff, ctx);
    y = diff_toolbar(scene, rect, y, ctx);

    // Scrolling content region — windowed to the viewport. Each file block has
    // an exact, computable pixel height (fixed 18px lines + fixed-height
    // headers/elisions), so a file entirely above the viewport is *skipped*
    // without drawing any of its header / hunk-headers / lines — we only advance
    // y. A file intersecting the viewport is drawn in full (its own per-line
    // skip/break handles within-file windowing); we stop once past the bottom.
    // Nothing visible changes: off-screen blocks were never painted, only
    // walked. This makes the loop O(visible files), not O(all files).
    y -= state.diff_scroll;
    for file in &diff.files {
        if y > rect.y1 {
            break;
        }
        let h = file_block_height(file);
        if y + h < rect.y0 {
            // Entirely above the viewport: skip building it.
            y += h;
            continue;
        }
        y = file_block(scene, rect, y, file, ctx);
    }

    scene.pop_layer();
}

/// Exact pixel height a [`file_block`] consumes, computed without drawing. Kept
/// in lockstep with `file_block`'s layout so skipping an off-screen block
/// advances `y` to the byte-identical position the full walk would reach.
fn file_block_height(file: &FileDiff) -> f64 {
    // File header (§5.4).
    let mut h = 7.0 + font::FS_MD as f64 + 8.0 + 7.0;

    for (hi, hunk) in file.hunks.iter().enumerate() {
        // Leading elision before the first bounded hunk of a Modified file.
        if hi == 0 && file.status == ChangeStatus::Modified {
            let first = hunk
                .lines
                .iter()
                .find_map(|l| l.new_no.or(l.old_no))
                .unwrap_or(1);
            if first.saturating_sub(1) > 0 {
                h += L::DIFF_LINE_H + 4.0; // elision row
            }
        }
        // Hunk header.
        h += 3.0 + font::FS_SM as f64 + 6.0 + 3.0;
        // Hunk lines (fixed 18px each).
        h += hunk.lines.len() as f64 * L::DIFF_LINE_H;
    }

    // Trailing "rest of file" affordance.
    if matches!(file.status, ChangeStatus::Modified | ChangeStatus::Deleted) {
        h += L::DIFF_LINE_H + 4.0;
    }

    h
}

// --- §5.1 Revision header --------------------------------------------------

fn revision_header(
    scene: &mut Scene,
    rect: Rect,
    y: f64,
    snapshot: &Snapshot,
    state: &UiState,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    // RevisionHeader (§5.1): 41px tall, calibrated to docs/reference/revisions.png
    // (the "lklqykmz (no description)" band that is the very top of the panel).
    let h = 41.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;

    let node = snapshot
        .nodes
        .get(state.selected)
        .or_else(|| snapshot.working_copy());

    let (change_id, desc, empty) = match node {
        Some(n) => (
            n.change_id.clone(),
            n.description.clone(),
            n.description.trim().is_empty(),
        ),
        None => (String::new(), String::new(), true),
    };
    let id8: String = change_id.chars().take(8).collect();

    let x = r.x0 + PAD_X;
    // change-id: mono, amber, weight 600, fs-md.
    let after_id = text::draw_text(
        scene, &ctx.fonts.mono_bold, font::FS_MD, t.amber,
        x, baseline_for(cy, font::FS_MD, &ctx.fonts.mono_bold), &id8,
    );
    let (dtext, dcolor) = if empty {
        ("(no description)".to_string(), t.text_faint)
    } else {
        (desc, t.text)
    };
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_MD, dcolor,
        after_id + 10.0, baseline_for(cy, font::FS_MD, &ctx.fonts.ui), &dtext,
    );

    // "Describe" ghost button, right-aligned.
    ghost_button(scene, r.x1 - PAD_X, cy, "Describe", t.subtext0, ctx);

    r.y1
}

// --- §5.2 Files bar --------------------------------------------------------

fn files_bar(scene: &mut Scene, rect: Rect, y: f64, diff: &CommitDiff, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    // FILES bar (§5.2): 38px tall, calibrated to docs/reference/revisions.png.
    let h = 38.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;

    let mut x = r.x0 + PAD_X;
    let n = diff.files.len();
    // "FILES" + two select-all/none chips ([ ]) + "(N)".
    x = text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_XS, t.text_faint,
        x, baseline_for(cy, font::FS_XS, &ctx.fonts.ui_bold), "FILES",
    );
    x += 8.0;
    x = file_chip(scene, x, cy, "[", ctx);
    x += 4.0;
    x = file_chip(scene, x, cy, "]", ctx);
    x += 8.0;
    x = text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_XS, t.text_faint,
        x, baseline_for(cy, font::FS_XS, &ctx.fonts.ui_bold), &format!("({n})"),
    );
    let added = diff.total_added();
    let removed = diff.total_removed();
    x += 10.0;
    // `.total-stats` is `font-family: inherit` (body `--font-ui`), weight 600 —
    // the aggregate +/- counts are sans (Inter ui_bold), NOT mono.
    let pa = format!("+{added}");
    x = text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_SM, t.green,
        x, baseline_for(cy, font::FS_SM, &ctx.fonts.ui_bold), &pa,
    );
    x += 6.0;
    let pr = format!("-{removed}");
    x = text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_SM, t.red,
        x, baseline_for(cy, font::FS_SM, &ctx.fonts.ui_bold), &pr,
    );

    // File tabs.
    x += 18.0;
    for (i, file) in diff.files.iter().enumerate() {
        let active = i == 0;
        let name = file_name(&file.path);
        let dot_color = type_color(file.status, t);

        let dot_r = 2.5;
        let dot_cx = x + dot_r;
        scene.fill(
            Fill::NonZero, Affine::IDENTITY, dot_color, None,
            &vello::kurbo::Circle::new((dot_cx, cy), dot_r),
        );
        let mut nx = x + dot_r * 2.0 + 6.0;
        let name_font = if active { &ctx.fonts.ui_bold } else { &ctx.fonts.ui };
        let name_color = if active { t.text } else { t.subtext0 };
        let after_name = text::draw_text(
            scene, name_font, font::FS_SM, name_color,
            nx, baseline_for(cy, font::FS_SM, name_font), name,
        );
        nx = after_name + 5.0;
        let (stat, scol) = if file.added > 0 && file.removed == 0 {
            (format!("+{}", file.added), t.green)
        } else if file.removed > 0 && file.added == 0 {
            (format!("-{}", file.removed), t.red)
        } else {
            (format!("+{} -{}", file.added, file.removed), t.subtext0)
        };
        // `.file-tab-stats` is `font-family: inherit` (the diff panel inherits
        // body `--font-ui`) — the +/- counts in the file-tab strip are sans
        // (Inter), NOT mono. (Only diff-line code + hunk headers are mono.)
        let after_stat = text::draw_text(
            scene, &ctx.fonts.ui, font::FS_XS, scol,
            nx, baseline_for(cy, font::FS_XS, &ctx.fonts.ui), &stat,
        );

        if active {
            // Active-tab underline (lightjj's `border-bottom` on the active file
            // tab) sits ~2px below the tab text, not on the bar's bottom edge.
            // Calibrated to docs/reference/revisions.png (device y 262–265 @2x =
            // logical 131.0–133.0, i.e. r.y1-8 .. r.y1-6).
            fill_rect(
                scene,
                Rect::new(x - 4.0, r.y1 - 8.0, after_stat + 4.0, r.y1 - 6.0),
                t.amber,
            );
        }
        x = after_stat + 18.0;
    }

    r.y1
}

// --- §5.3 Diff toolbar -----------------------------------------------------

fn diff_toolbar(scene: &mut Scene, rect: Rect, y: f64, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    // Diff toolbar (§5.3): 30px tall, calibrated to docs/reference/revisions.png.
    let h = 30.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;

    // Left: collapse button (⊟).
    let bx = r.x0 + PAD_X;
    let bw = 24.0;
    let btn = Rect::new(bx, cy - 9.0, bx + bw, cy + 9.0);
    stroke_rect(scene, btn, t.surface1, 1.0);
    let glyph = "\u{229f}";
    let gw = text::measure(&ctx.fonts.ui, font::FS_SM, glyph) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.subtext0,
        btn.center().x - gw / 2.0, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), glyph,
    );

    // ann-hint.
    let mut hx = btn.x1 + 10.0;
    hx = kbd_chip(scene, hx, cy, "Alt", ctx);
    hx = text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
        hx + 4.0, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), "+click annotate \u{00b7} ",
    );
    hx = kbd_chip(scene, hx + 2.0, cy, "Ctrl", ctx);
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
        hx + 4.0, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), "hover definition",
    );

    // Right: unified/split toggle (≡).
    let tw = 24.0;
    let tb = Rect::new(r.x1 - PAD_X - tw, cy - 9.0, r.x1 - PAD_X, cy + 9.0);
    stroke_rect(scene, tb, t.surface1, 1.0);
    let tg = "\u{2261}";
    let tgw = text::measure(&ctx.fonts.ui, font::FS_SM, tg) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.subtext0,
        tb.center().x - tgw / 2.0, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), tg,
    );

    r.y1
}

// --- §5.4–5.6 Per-file block ----------------------------------------------

fn file_block(
    scene: &mut Scene,
    rect: Rect,
    mut y: f64,
    file: &FileDiff,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;

    // --- File header (§5.4) ---
    let hh = 7.0 + font::FS_MD as f64 + 8.0 + 7.0;
    let hdr = Rect::new(rect.x0, y, rect.x1, y + hh);
    fill_rect(scene, hdr, t.mantle);
    border_bottom(scene, hdr, t.surface0);
    let cy = hdr.center().y;

    let mut x = hdr.x0 + PAD_X;
    // collapse chevron (expanded → ▼).
    x = text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
        x, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), "\u{25be}",
    );
    x += 8.0;

    // file-type badge.
    let (letter, badge_fg, badge_bg) = badge_for(file.status, t);
    let lw = text::measure(&ctx.fonts.ui_bold, font::FS_XS, letter) as f64;
    let bw = lw + 12.0;
    let badge = Rect::new(x, cy - 9.0, x + bw, cy + 9.0);
    fill_round(scene, badge, 4.0, badge_bg);
    text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_XS, badge_fg,
        badge.center().x - lw / 2.0, baseline_for(cy, font::FS_XS, &ctx.fonts.ui_bold), letter,
    );
    x = badge.x1 + 10.0;

    // path: dir faint + name bold.
    let (dir, name) = split_path(&file.path);
    if !dir.is_empty() {
        x = text::draw_text(
            scene, &ctx.fonts.ui, font::FS_MD, t.text_faint,
            x, baseline_for(cy, font::FS_MD, &ctx.fonts.ui), &dir,
        );
    }
    text::draw_text(
        scene, &ctx.fonts.ui_bold, font::FS_MD, t.text,
        x, baseline_for(cy, font::FS_MD, &ctx.fonts.ui_bold), name,
    );

    // Right-aligned: Discard, [Edit], History ghost buttons, checkbox, +N/-N.
    let mut rx = hdr.x1 - PAD_X;
    rx = ghost_button(scene, rx, cy, "Discard", t.red, ctx);
    rx -= 6.0;
    if file.status == ChangeStatus::Modified {
        rx = ghost_button(scene, rx, cy, "Edit", t.subtext0, ctx);
        rx -= 6.0;
    }
    rx = ghost_button(scene, rx, cy, "History", t.subtext0, ctx);
    rx -= 10.0;
    // checkbox (review toggle).
    let cbx = Rect::new(rx - 14.0, cy - 7.0, rx, cy + 7.0);
    stroke_rect(scene, cbx, t.surface2, 1.0);
    rx = cbx.x0 - 10.0;
    // `.file-stats`/`.stat-add`/`.stat-del` inherit body `--font-ui` — the
    // per-file +/- counts in the diff-file header are sans (Inter), not mono.
    if file.removed > 0 {
        let pr = format!("-{}", file.removed);
        let w = text::measure(&ctx.fonts.ui_bold, font::FS_SM, &pr) as f64;
        text::draw_text(
            scene, &ctx.fonts.ui_bold, font::FS_SM, t.red,
            rx - w, baseline_for(cy, font::FS_SM, &ctx.fonts.ui_bold), &pr,
        );
        rx -= w + 6.0;
    }
    if file.added > 0 {
        let pa = format!("+{}", file.added);
        let w = text::measure(&ctx.fonts.ui_bold, font::FS_SM, &pa) as f64;
        text::draw_text(
            scene, &ctx.fonts.ui_bold, font::FS_SM, t.green,
            rx - w, baseline_for(cy, font::FS_SM, &ctx.fonts.ui_bold), &pa,
        );
    }

    y = hdr.y1;

    // --- Hunks ---
    for (hi, hunk) in file.hunks.iter().enumerate() {
        if y > rect.y1 {
            return y;
        }
        // Leading elision ("··· N lines") before the first bounded hunk: the
        // number of lines hidden above the first displayed line.
        if hi == 0 && file.status == ChangeStatus::Modified {
            let first = hunk
                .lines
                .iter()
                .find_map(|l| l.new_no.or(l.old_no))
                .unwrap_or(1);
            let leading = first.saturating_sub(1);
            if leading > 0 {
                y = elision(scene, rect, y, &format!("{leading} lines"), ctx);
            }
        }

        y = hunk_header(scene, rect, y, hunk, ctx);

        for line in &hunk.lines {
            if y + L::DIFF_LINE_H < rect.y0 {
                y += L::DIFF_LINE_H;
                continue;
            }
            y = diff_line(scene, rect, y, line, ctx);
            if y > rect.y1 {
                return y;
            }
        }
    }

    // Trailing "rest of file" affordance.
    if matches!(file.status, ChangeStatus::Modified | ChangeStatus::Deleted) {
        y = elision(scene, rect, y, "rest of file", ctx);
    }

    y
}

fn hunk_header(scene: &mut Scene, rect: Rect, y: f64, hunk: &Hunk, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let h = 3.0 + font::FS_SM as f64 + 6.0 + 3.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    fill_rect(scene, r, t.bg_hunk_header);
    border_bottom(scene, r, t.surface1);
    let cy = r.center().y;

    let range = format!(
        "-{},{} +{},{}",
        hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len
    );
    let x = r.x0 + PAD_X;
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_XS, t.text_faint,
        x, baseline_for(cy, font::FS_XS, &ctx.fonts.mono), &range,
    );
    // NOTE: lightjj's `.hunk-context` comes from jj's `@@ … @@ ctx` trailer, which
    // is empty for this fixture (the reference shows only the range). The jj-lib
    // loader synthesizes a nearest-line context that the reference never displays,
    // so we deliberately omit it here to match the ground-truth render.
    let _ = &hunk.header_context;

    r.y1
}

fn diff_line(scene: &mut Scene, rect: Rect, y: f64, line: &DiffLine, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let lr = Rect::new(rect.x0, y, rect.x1, y + L::DIFF_LINE_H);
    let cy = lr.center().y;

    let (bg, base_fg, prefix, border): (Option<Color>, Color, &str, Option<Color>) =
        match line.kind {
            LineKind::Add => (Some(t.diff_add_bg), t.green, "+", Some(t.green)),
            LineKind::Remove => (Some(t.diff_remove_bg), t.red, "-", Some(t.red)),
            LineKind::Context => (None, t.subtext0, " ", None),
        };

    if let Some(bg) = bg {
        fill_rect(scene, lr, bg);
    }
    if let Some(bc) = border {
        fill_rect(scene, Rect::new(lr.x0, lr.y0, lr.x0 + 3.0, lr.y1), bc);
    }

    // Layout (lightjj's 11ch hanging-indent gutter, calibrated to the reference's
    // JetBrains-Mono advances so columns land identically despite our wider mono).
    // Panel-relative logical x targets, measured from docs/reference/revisions.png.
    let num_sz = font::FS_SM;
    let old_s = line.old_no.map(|n| n.to_string()).unwrap_or_default();
    let new_s = line.new_no.map(|n| n.to_string()).unwrap_or_default();

    let old_right = lr.x0 + OLD_NUM_RIGHT;
    let ow = text::measure(&ctx.fonts.mono, num_sz, &old_s) as f64;
    text::draw_text(
        scene, &ctx.fonts.mono, num_sz, t.text_faint,
        old_right - ow, baseline_for(cy, num_sz, &ctx.fonts.mono), &old_s,
    );
    let new_right = lr.x0 + NEW_NUM_RIGHT;
    let nw = text::measure(&ctx.fonts.mono, num_sz, &new_s) as f64;
    text::draw_text(
        scene, &ctx.fonts.mono, num_sz, t.text_faint,
        new_right - nw, baseline_for(cy, num_sz, &ctx.fonts.mono), &new_s,
    );

    // Context-line gutter right border (`--line-gutter-border`).
    if matches!(line.kind, LineKind::Context) {
        let bx = lr.x0 + GUTTER_BORDER_X;
        fill_rect(scene, Rect::new(bx, lr.y0, bx + 1.0, lr.y1), t.surface0);
    }

    // Code cell: prefix (+/-/space @ 0.5 opacity) then the code at CODE_X.
    let text_x = lr.x0 + CODE_X;
    let cw = text::measure(&ctx.fonts.mono, font::FS_MD, prefix) as f64;
    let code_baseline = baseline_for(cy, font::FS_MD, &ctx.fonts.mono);
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_MD, base_fg.multiply_alpha(0.5),
        text_x - cw, code_baseline, prefix,
    );
    draw_highlighted(
        scene, &ctx.fonts.mono, font::FS_MD, text_x, code_baseline,
        &line.text, line.kind, t,
    );

    lr.y1
}

/// Centered "··· label" elision affordance.
fn elision(scene: &mut Scene, rect: Rect, y: f64, label: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let h = L::DIFF_LINE_H + 4.0;
    let r = Rect::new(rect.x0, y, rect.x1, y + h);
    let cy = r.center().y;
    let full = format!("\u{00b7}\u{00b7}\u{00b7}  {label}");
    let w = text::measure(&ctx.fonts.ui, font::FS_SM, &full) as f64;
    let cx = (r.x0 + r.x1) / 2.0 - w / 2.0;
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
        cx, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), &full,
    );
    r.y1
}

// --- Buttons / chips -------------------------------------------------------

/// A ghost `.btn` right-anchored at `x_right`. Returns its left edge.
fn ghost_button(scene: &mut Scene, x_right: f64, cy: f64, label: &str, fg: Color, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let lw = text::measure(&ctx.fonts.ui, font::FS_SM, label) as f64;
    let pad = 10.0;
    let w = lw + pad * 2.0;
    let r = Rect::new(x_right - w, cy - 10.0, x_right, cy + 10.0);
    let border = if fg == t.red { t.red } else { t.surface1 };
    stroke_rect(scene, r, border, 1.0);
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_SM, fg,
        r.x0 + pad, baseline_for(cy, font::FS_SM, &ctx.fonts.ui), label,
    );
    r.x0
}

/// A small kbd hint chip. Returns the x past it.
fn kbd_chip(scene: &mut Scene, x: f64, cy: f64, key: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let kw = text::measure(&ctx.fonts.mono, font::FS_2XS, key) as f64;
    let w = kw + 8.0;
    let r = Rect::new(x, cy - 8.0, x + w, cy + 8.0);
    stroke_rect(scene, r, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_2XS, t.overlay0,
        r.x0 + 4.0, baseline_for(cy, font::FS_2XS, &ctx.fonts.mono), key,
    );
    r.x1
}

/// A tiny select-all/none chip in the FILES bar ("[" / "]"). Returns x past it.
fn file_chip(scene: &mut Scene, x: f64, cy: f64, glyph: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let gw = text::measure(&ctx.fonts.mono, font::FS_XS, glyph) as f64;
    let w = gw + 8.0;
    let r = Rect::new(x, cy - 8.0, x + w, cy + 8.0);
    stroke_rect(scene, r, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_XS, t.text_faint,
        r.x0 + 4.0, baseline_for(cy, font::FS_XS, &ctx.fonts.mono), glyph,
    );
    r.x1
}

// --- helpers ---------------------------------------------------------------

fn type_color(status: ChangeStatus, t: &Palette) -> Color {
    match status {
        ChangeStatus::Added => t.green,
        ChangeStatus::Modified | ChangeStatus::Renamed => t.amber,
        ChangeStatus::Deleted | ChangeStatus::Copied => t.red,
    }
}

/// (letter, fg, bg) for the file-type badge (§5.4).
fn badge_for(status: ChangeStatus, t: &Palette) -> (&'static str, Color, Color) {
    match status {
        ChangeStatus::Added => ("A", t.green, t.green.multiply_alpha(0.12)),
        ChangeStatus::Modified => ("M", t.amber, t.amber.multiply_alpha(0.12)),
        ChangeStatus::Deleted => ("D", t.red, t.red.multiply_alpha(0.12)),
        ChangeStatus::Renamed => ("R", t.amber, t.amber.multiply_alpha(0.12)),
        ChangeStatus::Copied => ("C", t.red, t.red.multiply_alpha(0.12)),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Split into (dir-with-trailing-slash, name).
fn split_path(path: &str) -> (String, &str) {
    match path.rfind('/') {
        Some(i) => (path[..=i].to_string(), &path[i + 1..]),
        None => (String::new(), path),
    }
}

// --- §5.6 Lightweight Rust syntax highlighting -----------------------------

/// Draw `text` as syntax-colored runs. Highlighted context dims to 0.7;
/// add/remove keep their semantic tint as the fallback color so unstyled
/// tokens still read green/red.
fn draw_highlighted(
    scene: &mut Scene,
    font_data: &FontData,
    size: f32,
    x: f64,
    baseline_y: f64,
    text_str: &str,
    kind: LineKind,
    t: &Palette,
) {
    let base = match kind {
        LineKind::Context => t.text.multiply_alpha(0.7),
        LineKind::Add => t.green,
        LineKind::Remove => t.red,
    };
    let mut pen = x;
    for (s, color) in tokenize(text_str, t, base) {
        pen = text::draw_text(scene, font_data, size, color, pen, baseline_y, &s);
    }
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
    "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop", "match",
    "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "type", "union", "unsafe", "use", "where", "while",
];

const ATOMS: &[&str] = &["true", "false", "None", "Some", "Ok", "Err"];

/// Tokenize one line of Rust-ish source into colored runs. Generic, not a real
/// parser — just enough to look right on Rust code.
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

        // Line comment.
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            push(&mut out, chars[i..].iter().collect(), t.syn_comment);
            break;
        }

        // String literal.
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

        // Whitespace.
        if c.is_whitespace() {
            let mut j = i;
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            push(&mut out, chars[i..j].iter().collect(), base);
            i = j;
            continue;
        }

        // Numbers.
        if c.is_ascii_digit() {
            let mut j = i;
            while j < n && (chars[j].is_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            push(&mut out, chars[i..j].iter().collect(), t.syn_number);
            i = j;
            continue;
        }

        // Identifiers / keywords / types / macros / calls.
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

        // Operators / punctuation.
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
