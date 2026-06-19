// INTEGRATION: move to src/ui/evolog.rs; show as a bottom drawer on key 5; replace MockEvologEntry with a jjlib evolog query (evolution of the selected change)
//
//! EVOLOG drawer renderer — lightjj's "5" toggle (`EvologPanel.svelte`).
//!
//! Evolog = the *evolution history* of ONE revision: a list of evolution
//! entries (each a past version of the same change) with inline per-entry diffs.
//! The diff shown for each entry is the inter-diff between that evolution and its
//! predecessor evolution — i.e. "what changed in this rewrite step". lightjj
//! renders this as a bottom drawer above the statusbar, same chrome as Oplog.
//!
//! Structurally it mirrors `EvologPanel.svelte`: a panel header
//! ("EVOLUTION LOG" + the change-id being inspected + an entry count), then a
//! list of evolution entries. lightjj uses a two-pane layout (entry list left,
//! diff of the selected entry right); here, to read well in a short bottom
//! drawer and to surface every step at once, each entry is a header row
//! (commit-id / operation / time) followed inline by its own small inter-diff
//! snippet — the "inline per-entry diffs" of the spec.
//!
//! This module is SELF-CONTAINED: it carries its own `MockEvologEntry` /
//! `MockDiffLine` stub types and its own minimal diff-line drawing, importing
//! only PUBLIC helpers from `jjscratch::ui`, `jjscratch::text`, and
//! `jjscratch::theme`. It paints ONLY within its `rect` (clipped), the same
//! contract the integrated `ui::{graph,diff,oplog}` renderers follow.
//
// This is a bin-local module (used by `preview_evolog.rs`). The autobins
// machinery treats every `src/bin/*.rs` as a binary needing a `main`, so this
// file carries a dead-code `main` shim; the real entry point lives in
// `preview_evolog.rs`, which `#[path]`-includes this module.

#![allow(dead_code)]

use jjscratch::text;
use jjscratch::theme::font;
use jjscratch::ui::{baseline_for, border_bottom, fill_rect, fill_round, stroke_round, RenderCtx};
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::Scene;

/// Autobins `main` shim — see the module comment. The real preview entry point
/// is `preview_evolog.rs`.
#[allow(dead_code)]
fn main() {}

// ---------------------------------------------------------------------------
// Stub data types (INTEGRATION: replace with a jjlib evolog query).
// ---------------------------------------------------------------------------

/// One change kind for a diff line in an inter-diff snippet.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MockLineKind {
    Add,
    Remove,
    Context,
}

/// One line of an evolution entry's inline inter-diff.
#[derive(Clone)]
pub struct MockDiffLine {
    pub kind: MockLineKind,
    /// Old-side line number (None for added lines).
    pub old_no: Option<u32>,
    /// New-side line number (None for removed lines).
    pub new_no: Option<u32>,
    pub text: String,
}

impl MockDiffLine {
    fn ctx(old_no: u32, new_no: u32, text: &str) -> Self {
        Self { kind: MockLineKind::Context, old_no: Some(old_no), new_no: Some(new_no), text: text.into() }
    }
    fn add(new_no: u32, text: &str) -> Self {
        Self { kind: MockLineKind::Add, old_no: None, new_no: Some(new_no), text: text.into() }
    }
    fn remove(old_no: u32, text: &str) -> Self {
        Self { kind: MockLineKind::Remove, old_no: Some(old_no), new_no: None, text: text.into() }
    }
}

/// One evolution entry: a past version of the inspected change. The `diff` is
/// the inter-diff against the predecessor evolution (empty for the origin entry
/// or a metadata-only rewrite). lightjj keys entries by `commit_id` and shows
/// `change_id`/`commit_id`/`time` + the operation that produced this version.
#[derive(Clone)]
pub struct MockEvologEntry {
    /// Short commit-id of this evolution (lightjj's `.entry-id`, mono, fixed col).
    pub commit_id: String,
    /// The operation that produced this version (lightjj's `.entry-op`).
    pub operation: String,
    /// Timestamp (lightjj slices `time.slice(0,19)`).
    pub time: String,
    /// File path touched by this step's inter-diff (one file for the stub).
    pub file_path: String,
    /// The inter-diff against the predecessor evolution. Empty ⇒ origin / no-op.
    pub diff: Vec<MockDiffLine>,
    /// True for the newest entry (lightjj's `.current`, amber id).
    pub current: bool,
    /// True for the oldest entry (lightjj's `.origin`, dimmed, no predecessor).
    pub origin: bool,
}

/// Stub evolution log: ONE change with three evolutions (newest first), each with
/// a small inter-diff, plus an origin entry with no predecessor. INTEGRATION:
/// this is the placeholder for a jjlib evolog query of the selected change.
pub fn mock_entries() -> Vec<MockEvologEntry> {
    vec![
        MockEvologEntry {
            commit_id: "a1b2c3d4e5f6".into(),
            operation: "describe commit".into(),
            time: "2026-06-19 14:02:11".into(),
            file_path: "src/flags.rs".into(),
            current: true,
            origin: false,
            diff: vec![
                MockDiffLine::ctx(11, 11, "pub struct Flags {"),
                MockDiffLine::ctx(12, 12, "    pub verbose: bool,"),
                MockDiffLine::remove(13, "    pub color: bool,"),
                MockDiffLine::add(13, "    pub color: ColorWhen,"),
                MockDiffLine::add(14, "    pub quiet: bool,"),
                MockDiffLine::ctx(14, 15, "}"),
            ],
        },
        MockEvologEntry {
            commit_id: "9f8e7d6c5b4a".into(),
            operation: "squash into parent".into(),
            time: "2026-06-19 13:47:55".into(),
            file_path: "src/flags.rs".into(),
            current: false,
            origin: false,
            diff: vec![
                MockDiffLine::ctx(4, 4, "use crate::color::ColorWhen;"),
                MockDiffLine::ctx(5, 5, ""),
                MockDiffLine::add(6, "/// Parsed command-line flags."),
                MockDiffLine::ctx(6, 7, "pub struct Flags {"),
                MockDiffLine::remove(7, "    pub v: bool,"),
                MockDiffLine::add(8, "    pub verbose: bool,"),
            ],
        },
        MockEvologEntry {
            commit_id: "1a2b3c4d5e6f".into(),
            operation: "rebase -d main".into(),
            time: "2026-06-19 13:30:02".into(),
            file_path: "src/flags.rs".into(),
            current: false,
            origin: false,
            // Metadata-only step (rebase moved the commit but the tree is
            // unchanged relative to its new predecessor) — empty inter-diff.
            diff: vec![],
        },
        MockEvologEntry {
            commit_id: "0099aabbccdd".into(),
            operation: "new commit".into(),
            time: "2026-06-19 13:28:40".into(),
            file_path: "src/flags.rs".into(),
            current: false,
            origin: true,
            diff: vec![],
        },
    ]
}

/// The change-id of the inspected revision (lightjj's header `.header-change-id`,
/// amber). Stub. INTEGRATION: the change-id of the selected revision.
pub fn mock_change_id() -> &'static str {
    "qpvuntsmwlrt"
}

// ---------------------------------------------------------------------------
// Layout constants (mirroring lightjj `.evolog-panel` / `.evolog-entry`).
// ---------------------------------------------------------------------------

/// Panel-header height (`.panel-header`) — same chrome the oplog/graph headers use.
const HEADER_H: f64 = jjscratch::theme::layout::PANEL_HEADER_H;
/// `.evolog-entry` / `.panel-header` horizontal padding.
const PAD_X: f64 = 12.0;
/// `.entry-id` fixed column width (lightjj `width: 96px`).
const ID_COL_W: f64 = 100.0;
/// Entry-header row height (commit-id / operation / time). `.evolog-entry` is
/// --fs-md over ~1.4 line-height plus 2×4px padding ≈ 26px.
const ENTRY_H: f64 = 26.0;
/// Inline inter-diff line height (lightjj's diff rows are 18px; we tighten the
/// drawer-embedded snippet to 16px to fit more steps).
const DIFF_LINE_H: f64 = 16.0;
/// Left indent of the inline diff snippet under an entry header (aligns the
/// snippet under the operation column, indented past the id column rail).
const DIFF_INDENT: f64 = PAD_X + 14.0;
/// Two line-number columns inside the inline snippet (snippet-relative).
const OLD_NUM_RIGHT: f64 = 22.0;
const NEW_NUM_RIGHT: f64 = 50.0;
/// Left edge of code text in the snippet (prefix sits 1ch to its left).
const CODE_X: f64 = 64.0;
/// Max inline diff lines drawn per entry before eliding (keeps the drawer tidy).
const MAX_DIFF_LINES: usize = 8;

// ---------------------------------------------------------------------------
// Renderer.
// ---------------------------------------------------------------------------

/// Render the EVOLOG drawer into `rect`, painting only within it.
///
/// `entries` is newest-first (the current version first), exactly as a jjlib
/// evolog query of the selected change would return it. `change_id` is the
/// inspected revision's change-id (shown amber in the header).
pub fn render(
    scene: &mut Scene,
    rect: Rect,
    entries: &[MockEvologEntry],
    change_id: &str,
    ctx: &RenderCtx,
) {
    let t = ctx.theme;

    // Drawer background. (The 1px top border separating the drawer from the body
    // above is painted by the shell; the preview paints it explicitly.)
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    // --- Panel header: "EVOLUTION LOG" + change-id + nav hints + count -----
    let header = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HEADER_H);
    fill_rect(scene, header, t.mantle);
    border_bottom(scene, header, t.surface0);
    let hcy = header.center().y;
    let hsz = font::FS_SM;
    // `.panel-title`: --fs-sm, SemiBold, UPPERCASE, color --subtext1.
    let title_end = text::draw_text(
        scene, &ctx.fonts.ui_bold, hsz, t.subtext1,
        header.x0 + PAD_X, baseline_for(hcy, hsz, &ctx.fonts.ui_bold), "EVOLUTION LOG",
    );
    // Nav hints (`j` `k`) like the lightjj panel title.
    let mut kx = title_end + 6.0;
    for key in ["j", "k"] {
        kx = nav_hint_kbd(scene, kx, hcy, key, ctx) + 2.0;
    }
    // The inspected change-id (`.header-change-id`, amber, mono), 12 chars.
    let id12: String = change_id.chars().take(12).collect();
    if !id12.is_empty() {
        kx = text::draw_text(
            scene, &ctx.fonts.mono_bold, font::FS_SM, t.amber,
            kx + 6.0, baseline_for(hcy, font::FS_SM, &ctx.fonts.mono_bold), &id12,
        );
        // `.entry-count` ("· N entries") in faint text.
        let label = if entries.len() == 1 { "entry" } else { "entries" };
        let count = format!(" \u{00b7} {} {label}", entries.len());
        text::draw_text(
            scene, &ctx.fonts.ui, font::FS_SM, t.text_faint,
            kx + 4.0, baseline_for(hcy, font::FS_SM, &ctx.fonts.ui), &count,
        );
    }
    // Right-aligned count badge (`.panel-badge`).
    if !entries.is_empty() {
        draw_count_badge(scene, header, entries.len(), ctx);
    }

    // --- Entries: each a header row + its inline inter-diff snippet ---------
    let mut y = header.y1;
    if entries.is_empty() {
        text::draw_text(
            scene, &ctx.fonts.ui, font::BASE, t.text_faint,
            rect.x0 + PAD_X + 12.0, y + 40.0, "Select a revision to view its evolution",
        );
        scene.pop_layer();
        return;
    }

    for (i, entry) in entries.iter().enumerate() {
        if y >= rect.y1 {
            break;
        }
        y = entry_block(scene, rect, y, entry, i == 0, ctx);
    }

    scene.pop_layer();
}

/// One evolution entry: a header row (commit-id / operation / time) followed by
/// its inline inter-diff snippet. Returns the y past the whole block.
fn entry_block(
    scene: &mut Scene,
    rect: Rect,
    y: f64,
    entry: &MockEvologEntry,
    _is_first: bool,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;

    // --- Entry header row -------------------------------------------------
    let row = Rect::new(rect.x0, y, rect.x1, (y + ENTRY_H).min(rect.y1));
    // The current (newest) entry gets the `.selected` tint (lightjj auto-selects
    // entry 0 on open). Origin entries dim (`.origin` opacity 0.6) — we lean on
    // faint text for that instead of layer alpha.
    if entry.current {
        fill_rect(scene, row, t.bg_checked);
    }
    border_bottom(scene, row, t.bg_hunk_header);
    let cy = row.center().y;

    // 1. commit-id — `.entry-id`: mono, fixed col, weight 600. Amber on the
    //    current entry (`.current .entry-id`), else faint (`--subtext0`).
    let id_sz = font::FS_SM;
    let id_x = row.x0 + PAD_X;
    let id_col = Rect::new(id_x, row.y0, id_x + ID_COL_W, row.y1);
    let id_color = if entry.current { t.amber } else { t.subtext0 };
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &id_col);
    text::draw_text(
        scene, &ctx.fonts.mono_bold, id_sz, id_color,
        id_x, baseline_for(cy, id_sz, &ctx.fonts.mono_bold), &entry.commit_id,
    );
    scene.pop_layer();

    // 3. time — `.entry-time`: --text-faint, --fs-xs, right-aligned.
    let time_sz = font::FS_XS;
    let time_w = text::measure(&ctx.fonts.ui, time_sz, &entry.time) as f64;
    let time_x = row.x1 - PAD_X - time_w;
    text::draw_text(
        scene, &ctx.fonts.ui, time_sz, t.text_faint,
        time_x, baseline_for(cy, time_sz, &ctx.fonts.ui), &entry.time,
    );

    // 2. operation — `.entry-op`: flex, --text. Origin entries read faint.
    let op_sz = font::FS_MD;
    let op_x0 = id_x + ID_COL_W + 10.0;
    let op_x1 = time_x - 10.0;
    if op_x1 > op_x0 {
        let op_col = Rect::new(op_x0, row.y0, op_x1, row.y1);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &op_col);
        let op_color = if entry.origin { t.text_faint } else { t.text };
        let after_op = text::draw_text(
            scene, &ctx.fonts.ui, op_sz, op_color,
            op_x0, baseline_for(cy, op_sz, &ctx.fonts.ui), &entry.operation,
        );
        // Origin entries get an "(origin)" tag like lightjj's dimmed origin row.
        if entry.origin {
            text::draw_text(
                scene, &ctx.fonts.ui, font::FS_XS, t.overlay0,
                after_op + 8.0, baseline_for(cy, font::FS_XS, &ctx.fonts.ui), "(origin)",
            );
        }
        scene.pop_layer();
    }

    let mut y = row.y1;

    // --- Inline inter-diff snippet ----------------------------------------
    // Mirrors EvologPanel's per-entry diff pane, inlined under the header.
    if y >= rect.y1 {
        return y;
    }
    if entry.diff.is_empty() {
        // lightjj's empty states: "Initial entry — no predecessor to diff
        // against" for the origin, "No changes (metadata-only operation)" else.
        let msg = if entry.origin {
            "Initial entry \u{2014} no predecessor to diff against"
        } else {
            "No changes (metadata-only operation)"
        };
        let r = Rect::new(rect.x0, y, rect.x1, (y + DIFF_LINE_H).min(rect.y1));
        text::draw_text(
            scene, &ctx.fonts.ui, font::FS_XS, t.text_faint,
            rect.x0 + DIFF_INDENT, baseline_for(r.center().y, font::FS_XS, &ctx.fonts.ui), msg,
        );
        return r.y1 + 4.0;
    }

    // File-path caption (which file this inter-diff touches) + +N/-N stats.
    let caption = Rect::new(rect.x0, y, rect.x1, (y + DIFF_LINE_H).min(rect.y1));
    let cap_cy = caption.center().y;
    let after_path = text::draw_text(
        scene, &ctx.fonts.mono, font::FS_XS, t.subtext0,
        rect.x0 + DIFF_INDENT, baseline_for(cap_cy, font::FS_XS, &ctx.fonts.mono), &entry.file_path,
    );
    let added = entry.diff.iter().filter(|l| l.kind == MockLineKind::Add).count();
    let removed = entry.diff.iter().filter(|l| l.kind == MockLineKind::Remove).count();
    let stat = format!("  +{added} -{removed}");
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_XS, t.text_faint,
        after_path, baseline_for(cap_cy, font::FS_XS, &ctx.fonts.mono), &stat,
    );
    y = caption.y1;

    let snippet_x0 = rect.x0 + DIFF_INDENT;
    let shown = entry.diff.len().min(MAX_DIFF_LINES);
    for line in entry.diff.iter().take(shown) {
        if y + DIFF_LINE_H > rect.y1 {
            return y;
        }
        y = diff_line(scene, snippet_x0, rect.x1, y, line, ctx);
    }
    if entry.diff.len() > shown {
        let hidden = entry.diff.len() - shown;
        let r = Rect::new(rect.x0, y, rect.x1, (y + DIFF_LINE_H).min(rect.y1));
        text::draw_text(
            scene, &ctx.fonts.ui, font::FS_XS, t.text_faint,
            snippet_x0, baseline_for(r.center().y, font::FS_XS, &ctx.fonts.ui),
            &format!("\u{00b7}\u{00b7}\u{00b7}  {hidden} more lines"),
        );
        y = r.y1;
    }

    // A little breathing room between entries (matches the panel's per-entry gap).
    y + 4.0
}

/// Draw one inline inter-diff line: add/remove background + left border, two
/// line-number columns, +/- prefix, then the code. Self-contained (no import of
/// the private diff renderer); plain-colored text (no syntax highlighting — the
/// drawer snippet reads as a quick inter-diff, not a full diff view).
fn diff_line(
    scene: &mut Scene,
    x0: f64,
    x1: f64,
    y: f64,
    line: &MockDiffLine,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let lr = Rect::new(x0, y, x1, y + DIFF_LINE_H);
    let cy = lr.center().y;

    let (bg, fg, prefix, border): (Option<Color>, Color, &str, Option<Color>) = match line.kind {
        MockLineKind::Add => (Some(t.diff_add_bg), t.green, "+", Some(t.green)),
        MockLineKind::Remove => (Some(t.diff_remove_bg), t.red, "-", Some(t.red)),
        MockLineKind::Context => (None, t.subtext0, " ", None),
    };

    if let Some(bg) = bg {
        fill_rect(scene, lr, bg);
    }
    if let Some(bc) = border {
        fill_rect(scene, Rect::new(lr.x0, lr.y0, lr.x0 + 2.0, lr.y1), bc);
    }

    let num_sz = font::FS_XS;
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

    // Code cell: prefix (dim) then the code text, clipped to the line rect.
    let code_x = lr.x0 + CODE_X;
    let code_sz = font::FS_SM;
    let pw = text::measure(&ctx.fonts.mono, code_sz, prefix) as f64;
    let baseline = baseline_for(cy, code_sz, &ctx.fonts.mono);
    text::draw_text(
        scene, &ctx.fonts.mono, code_sz, fg.multiply_alpha(0.5),
        code_x - pw, baseline, prefix,
    );
    let code_clip = Rect::new(code_x, lr.y0, lr.x1, lr.y1);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &code_clip);
    text::draw_text(scene, &ctx.fonts.mono, code_sz, fg, code_x, baseline, &line.text);
    scene.pop_layer();

    lr.y1
}

/// A `.nav-hint` kbd chip (mono --fs-2xs, --overlay0, 1px --surface1 border).
/// Returns its right edge x.
fn nav_hint_kbd(scene: &mut Scene, x: f64, cy: f64, key: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let sz = font::FS_2XS;
    let kw = text::measure(&ctx.fonts.mono, sz, key) as f64;
    let pad = 3.0;
    let chip = Rect::new(x, cy - 7.0, x + kw + pad * 2.0, cy + 7.0);
    stroke_round(scene, chip, 3.0, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, sz, t.overlay0,
        x + pad, baseline_for(cy, sz, &ctx.fonts.mono), key,
    );
    chip.x1
}

/// Right-aligned count pill (`.panel-badge`): bg --surface0, color --subtext0,
/// padding 0 6, radius 8, --fs-xs.
fn draw_count_badge(scene: &mut Scene, header: Rect, n: usize, ctx: &RenderCtx) {
    let t = ctx.theme;
    let cy = header.center().y;
    let s = n.to_string();
    let bsz = font::FS_XS;
    let tw = text::measure(&ctx.fonts.ui_bold, bsz, &s) as f64;
    let bw = tw + 6.0 * 2.0;
    let pill = Rect::new(header.x1 - PAD_X - bw, cy - 8.0, header.x1 - PAD_X, cy + 8.0);
    fill_round(scene, pill, 8.0, t.surface0);
    text::draw_text(
        scene, &ctx.fonts.ui_bold, bsz, t.subtext0,
        pill.x0 + 6.0, baseline_for(cy, bsz, &ctx.fonts.ui_bold), &s,
    );
}
