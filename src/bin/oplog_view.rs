//! OPLOG drawer renderer — lightjj's "4" toggle (`OplogPanel.svelte`).
//!
//! Draws the operation-log list that lightjj shows as a bottom drawer above the
//! statusbar: a panel header ("OPERATION LOG") over a list of rows, each row a
//! `op description`, a faint short op `id`, and a relative `time`. Mirrors
//! lightjj's `.oplog-panel` / `.oplog-entry` (docs/spec/ui-spec.md §3, lightjj
//! `frontend/src/lib/OplogPanel.svelte`).
//!
//! Self-contained for now: built and verified in isolation by `preview_oplog`.
//! It paints ONLY within `rect` (clipped), the same contract the integrated
//! `ui::{graph,diff}` renderers follow, so wiring it in later is mechanical.
//
// INTEGRATION: move to src/ui/oplog.rs; dispatch as a bottom drawer when the
// Oplog toggle is active.

// This file is compiled both as its own bin target and as a `#[path]` module of
// `preview_oplog`. `#[allow(dead_code)]` keeps the shared helpers from warning
// when compiled as the standalone bin (where `render` is the only public item).
//
// The renderer consumes `OpEntry`, which only exists under the `jjlib` feature,
// so every item is gated on it. Without the feature the bin still compiles —
// down to just the stub `main` below — keeping a featureless `cargo build`
// green (Cargo.toml is owned elsewhere, so we can't add `required-features`).
#![allow(dead_code)]

/// Standalone-bin entry point WITHOUT the feature: the renderer is unavailable.
#[cfg(not(feature = "jjlib"))]
fn main() {
    eprintln!("oplog_view needs --features jjlib (renders the op-log drawer).");
}

#[cfg(feature = "jjlib")]
use jjscratch::model::jjlib::OpEntry;
#[cfg(feature = "jjlib")]
use jjscratch::text;
#[cfg(feature = "jjlib")]
use jjscratch::theme::{font, layout as L};
#[cfg(feature = "jjlib")]
use jjscratch::ui::{baseline_for, border_bottom, fill_rect, RenderCtx};
#[cfg(feature = "jjlib")]
use vello::kurbo::{Affine, Rect};
#[cfg(feature = "jjlib")]
use vello::peniko::Fill;
#[cfg(feature = "jjlib")]
use vello::Scene;

// --- jjlib-gated renderer (everything below needs `OpEntry`) --------------
#[cfg(feature = "jjlib")]
pub use imp::render;

#[cfg(feature = "jjlib")]
mod imp {
use super::{baseline_for, border_bottom, fill_rect, font, text, Affine, Fill, OpEntry, Rect, RenderCtx, Scene, L};

/// Panel-header height (`.panel-header`, ui-spec §6 / §3) — same chrome the
/// graph/diff headers use.
const HEADER_H: f64 = L::PANEL_HEADER_H;
/// `.oplog-entry`: `padding: 3px 12px` — one row is content + 2×3px.
const ROW_PAD_Y: f64 = 3.0;
/// `.oplog-entry` / `.panel-header` horizontal padding.
const PAD_X: f64 = 12.0;
/// `.oplog-id` fixed column width (`width: 100px`).
const ID_COL_W: f64 = 100.0;
/// `.oplog-entry` `gap: 10px`.
const COL_GAP: f64 = 10.0;
/// Row height: `.oplog-content` is --fs-md (13px) over `line-height` ~1.4 plus
/// the 2×3px padding ≈ 24px. We use a fixed row so the list stays pixel-stable.
const ROW_H: f64 = 24.0;

/// Render the OPLOG drawer into `rect`, painting only within it.
///
/// `ops` is newest-first (head op first), exactly as
/// [`jjscratch::model::jjlib::oplog`] returns it.
pub fn render(scene: &mut Scene, rect: Rect, ops: &[OpEntry], ctx: &RenderCtx) {
    let t = ctx.theme;

    // Drawer background + top border (the drawer floats above the statusbar).
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    // --- Panel header: "OPERATION LOG" + nav hints ------------------------
    let header = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HEADER_H);
    fill_rect(scene, header, t.mantle);
    border_bottom(scene, header, t.surface0);
    let hcy = header.center().y;
    let hsz = font::FS_SM;
    // `.panel-title`: --fs-sm, weight ~700 (we use the SemiBold ui_bold),
    // UPPERCASE, color --subtext1.
    let title_end = text::draw_text(
        scene, &ctx.fonts.ui_bold, hsz, t.subtext1,
        header.x0 + PAD_X, baseline_for(hcy, hsz, &ctx.fonts.ui_bold), "OPERATION LOG",
    );
    // Nav hints (`j` `k` `Enter`) like the lightjj panel title.
    let mut kx = title_end + 6.0;
    for key in ["j", "k", "Enter"] {
        kx = nav_hint_kbd(scene, kx, hcy, key, ctx) + 2.0;
    }
    // Right-aligned count badge (`.panel-badge`).
    if !ops.is_empty() {
        draw_count_badge(scene, header, ops.len(), ctx);
    }

    // --- Rows -------------------------------------------------------------
    let mut y = header.y1;
    if ops.is_empty() {
        // Empty state, centered-ish text in faint color.
        text::draw_text(
            scene, &ctx.fonts.ui, font::BASE, t.text_faint,
            rect.x0 + PAD_X + 12.0, y + 40.0, "No operations",
        );
        scene.pop_layer();
        return;
    }

    for op in ops {
        if y >= rect.y1 {
            break;
        }
        let row = Rect::new(rect.x0, y, rect.x1, (y + ROW_H).min(rect.y1));
        draw_row(scene, row, op, ctx);
        y = row.y1;
    }

    scene.pop_layer();
}

/// Draw one `.oplog-entry` row: faint short id (fixed 100px column), description
/// (flex), then a faint right-aligned time. The head op gets a subtle "checked"
/// background tint (lightjj's `.oplog-current`).
fn draw_row(scene: &mut Scene, row: Rect, op: &OpEntry, ctx: &RenderCtx) {
    let t = ctx.theme;
    let is_current = op.tags.iter().any(|tag| tag == "current");

    // `.oplog-current` background tint (lightjj uses --bg-checked).
    if is_current {
        fill_rect(scene, row, t.bg_checked);
    }
    // `.oplog-entry` bottom hairline (--border-hunk-header ≈ --bg-hunk-header).
    border_bottom(scene, row, t.bg_hunk_header);

    let cy = row.center().y;

    // 1. op id — `.oplog-id`: mono, amber, weight 600, --fs-sm, fixed 100px col,
    //    clipped so a long id can't bleed into the description.
    let id_sz = font::FS_SM;
    let id_x = row.x0 + PAD_X;
    let id_col = Rect::new(id_x, row.y0, id_x + ID_COL_W, row.y1);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &id_col);
    text::draw_text(
        scene, &ctx.fonts.mono_bold, id_sz, t.amber,
        id_x, baseline_for(cy, id_sz, &ctx.fonts.mono_bold), &op.id,
    );
    scene.pop_layer();

    // 3. time — `.oplog-time`: --text-faint, --fs-sm, right-aligned, measured so
    //    we can right-anchor it and clip the description short of it.
    let time_sz = font::FS_SM;
    let time_w = text::measure(&ctx.fonts.ui, time_sz, &op.time) as f64;
    let time_x = row.x1 - PAD_X - time_w;
    text::draw_text(
        scene, &ctx.fonts.ui, time_sz, t.text_faint,
        time_x, baseline_for(cy, time_sz, &ctx.fonts.ui), &op.time,
    );

    // 2. description — `.oplog-desc`: flex, color --text, ellipsis (we hard-clip
    //    to the column between the id and the time).
    let desc_sz = font::FS_MD;
    let desc_x0 = id_x + ID_COL_W + COL_GAP;
    let desc_x1 = time_x - COL_GAP;
    if desc_x1 > desc_x0 {
        let desc_col = Rect::new(desc_x0, row.y0, desc_x1, row.y1);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &desc_col);
        let desc = if op.description.is_empty() {
            "(no description)"
        } else {
            &op.description
        };
        let color = if op.description.is_empty() {
            t.text_faint
        } else {
            t.text
        };
        text::draw_text(
            scene, &ctx.fonts.ui, desc_sz, color,
            desc_x0, baseline_for(cy, desc_sz, &ctx.fonts.ui), desc,
        );
        scene.pop_layer();
    }
}

/// A `.nav-hint` kbd chip (mono --fs-2xs, --overlay0, 1px --surface1 border).
/// Returns its right edge x.
fn nav_hint_kbd(scene: &mut Scene, x: f64, cy: f64, key: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let sz = font::FS_2XS;
    let kw = text::measure(&ctx.fonts.mono, sz, key) as f64;
    let pad = 3.0;
    let chip = Rect::new(x, cy - 7.0, x + kw + pad * 2.0, cy + 7.0);
    jjscratch::ui::stroke_round(scene, chip, 3.0, t.surface1, 1.0);
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
    jjscratch::ui::fill_round(scene, pill, 8.0, t.surface0);
    text::draw_text(
        scene, &ctx.fonts.ui_bold, bsz, t.subtext0,
        pill.x0 + 6.0, baseline_for(cy, bsz, &ctx.fonts.ui_bold), &s,
    );
}
} // mod imp

/// Standalone-bin entry point WITH the feature. The real preview lives in
/// `preview_oplog`; this only exists so `oplog_view` is a valid bin target.
#[cfg(feature = "jjlib")]
fn main() {
    eprintln!("oplog_view is a renderer module; run `preview_oplog` to see it.");
}
