//! OPLOG drawer renderer — lightjj's "4" toggle (`OplogPanel.svelte`).
//!
//! Draws the operation-log list that lightjj shows as a bottom drawer above the
//! statusbar: a panel header ("OPERATION LOG") over a list of rows, each row a
//! `op description`, a faint short op `id`, and a relative `time`. Mirrors
//! lightjj's `.oplog-panel` / `.oplog-entry` (docs/spec/ui-spec.md §3, lightjj
//! `frontend/src/lib/OplogPanel.svelte`).
//!
//! It paints ONLY within `rect` (clipped), the same contract the integrated
//! `ui::{graph,diff}` renderers follow. `ui::build_scene` dispatches here as a
//! bottom drawer when `oplog_open` (the `4` toggle) is set.
//
// The renderer consumes `OpEntry`, which only exists under the `jjlib` feature,
// so every item is gated on it. Without the feature the module is empty (the
// `4` toggle in build_scene passes a mock/empty slice and skips the drawer),
// keeping a featureless `cargo build` green.

#[cfg(feature = "jjlib")]
use crate::model::jjlib::OpEntry;
#[cfg(feature = "jjlib")]
use crate::text;
#[cfg(feature = "jjlib")]
use crate::theme::{font, layout as L};
#[cfg(feature = "jjlib")]
use super::{baseline_for, border_bottom, fill_rect, RenderCtx};
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
/// `.oplog-entry` / `.panel-header` horizontal padding. (`.oplog-entry`'s
/// `padding: 3px 12px` vertical 3px is folded into the fixed `ROW_H` below.)
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
        kx = nav_hint_kbd(scene, kx, hcy, key, ctx) + 1.0;
    }
    // Right-aligned action buttons (`.panel-actions`: ghost `.btn`s). lightjj's
    // OplogPanel header carries "Refresh" and "Close" buttons here, not a count
    // badge. They are static (jjscratch has no interactivity yet) but reproduce
    // the reference chrome. Drawn right-to-left so each anchors its right edge.
    let close_x0 = draw_btn(scene, header.x1 - PAD_X, hcy, "Close", ctx);
    draw_btn(scene, close_x0 - 8.0, hcy, "Refresh", ctx);

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
    crate::ui::stroke_round(scene, chip, 3.0, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, sz, t.overlay0,
        x + pad, baseline_for(cy, sz, &ctx.fonts.mono), key,
    );
    chip.x1
}

/// A right-anchored ghost button (`.btn`): transparent bg, 1px --surface1
/// border, radius 4, padding `3px 10px`, --subtext0 label at --fs-sm. `x1` is
/// the button's right edge; returns its left edge x so the caller can stack the
/// next button leftward with a gap.
fn draw_btn(scene: &mut Scene, x1: f64, cy: f64, label: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let sz = font::FS_SM;
    let tw = text::measure(&ctx.fonts.ui, sz, label) as f64;
    let pad_x = 10.0;
    let bw = tw + pad_x * 2.0;
    let x0 = x1 - bw;
    // `.btn` height: padding 3px×2 + fs-sm·line-height(1.4) ≈ 23px.
    let half_h = 11.5;
    let btn = Rect::new(x0, cy - half_h, x1, cy + half_h);
    crate::ui::stroke_round(scene, btn, 4.0, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.ui, sz, t.subtext0,
        x0 + pad_x, baseline_for(cy, sz, &ctx.fonts.ui), label,
    );
    x0
}
} // mod imp
