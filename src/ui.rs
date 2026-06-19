//! UI composition: turns a [`Snapshot`] (+ optional [`CommitDiff`]) into a Vello
//! [`Scene`]. This module owns the **chrome shell** (toolbar, tab bar, panel
//! headers, status bar, panel split) and the overall frame layout, then
//! delegates the two big content areas to [`graph`] and [`diff`].
//!
//! Contract for the content renderers: each exposes
//! `render(scene, rect, ..., ctx)` and paints ONLY within `rect`. The shell and
//! layout are stable; implement the panels against this interface.

pub mod diff;
pub mod graph;

use vello::kurbo::{Affine, Line, Rect, Stroke};
use vello::peniko::{Color, Fill};
use vello::Scene;

use crate::model::{CommitDiff, Snapshot};
use crate::text::{self, Fonts};
use crate::theme::{self, layout as L, Palette};

/// Which top-level view fills the right column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Revisions,
    Branches,
    Merge,
}

/// Mutable UI state the renderer reads (cursor, scroll, sizing).
#[derive(Clone, Debug)]
pub struct UiState {
    pub active_view: View,
    /// Index into `snapshot.nodes` of the selected revision.
    pub selected: usize,
    /// Hovered revision index (JS-tracked hover in lightjj), if any.
    pub hovered: Option<usize>,
    pub panel_width: f64,
    /// Vertical scroll offset (px) of the graph list.
    pub graph_scroll: f64,
    /// Vertical scroll offset (px) of the diff content.
    pub diff_scroll: f64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active_view: View::Revisions,
            selected: 0,
            hovered: None,
            panel_width: L::REVISION_PANEL_DEFAULT_W,
            graph_scroll: 0.0,
            diff_scroll: 0.0,
        }
    }
}

/// Shared rendering context: fonts + active theme palette.
pub struct RenderCtx<'a> {
    pub fonts: &'a Fonts,
    pub theme: &'a Palette,
}

/// Computed frame rectangles.
pub struct FrameLayout {
    pub toolbar: Rect,
    pub tab_bar: Rect,
    pub revset_bar: Rect,
    pub graph_header: Rect,
    pub graph_content: Rect,
    pub divider: Rect,
    pub diff_header: Rect,
    pub diff_content: Rect,
    pub statusbar: Rect,
}

impl FrameLayout {
    pub fn compute(w: f64, h: f64, panel_w: f64) -> Self {
        let panel_w = panel_w.clamp(L::REVISION_PANEL_MIN_W, L::REVISION_PANEL_MAX_W);
        let mut y = 0.0;
        let toolbar = Rect::new(0.0, y, w, y + L::TOOLBAR_H);
        y += L::TOOLBAR_H;
        let tab_bar = Rect::new(0.0, y, w, y + L::TAB_BAR_H);
        y += L::TAB_BAR_H;
        let body_top = y;
        let body_bot = h - L::STATUSBAR_H;

        let revset_bar = Rect::new(0.0, body_top, panel_w, body_top + L::REVSET_BAR_H);
        let graph_header = Rect::new(0.0, revset_bar.y1, panel_w, revset_bar.y1 + L::PANEL_HEADER_H);
        let graph_content = Rect::new(0.0, graph_header.y1, panel_w, body_bot);

        let divider = Rect::new(panel_w, body_top, panel_w + L::PANEL_DIVIDER_W, body_bot);
        let diff_x = divider.x1;
        let diff_header = Rect::new(diff_x, body_top, w, body_top + L::PANEL_HEADER_H);
        let diff_content = Rect::new(diff_x, diff_header.y1, w, body_bot);

        let statusbar = Rect::new(0.0, body_bot, w, h);

        Self {
            toolbar,
            tab_bar,
            revset_bar,
            graph_header,
            graph_content,
            divider,
            diff_header,
            diff_content,
            statusbar,
        }
    }
}

/// Build the full-window scene. `width`/`height` are logical px.
pub fn build_scene(
    scene: &mut Scene,
    snapshot: &Snapshot,
    diff: Option<&CommitDiff>,
    state: &UiState,
    ctx: &RenderCtx,
    width: f64,
    height: f64,
) {
    let t = ctx.theme;
    let fl = FrameLayout::compute(width, height, state.panel_width);

    // Base background (everything paints on top).
    fill_rect(scene, Rect::new(0.0, 0.0, width, height), t.base);

    draw_toolbar(scene, &fl, ctx, state);
    draw_tab_bar(scene, &fl, ctx, snapshot);

    // Revision panel chrome.
    draw_revset_bar(scene, &fl, ctx);
    draw_panel_header(scene, fl.graph_header, "REVISIONS", Some(snapshot.revision_count()), ctx);
    graph::render(scene, fl.graph_content, snapshot, state, ctx);

    // Divider.
    fill_rect(scene, fl.divider, t.surface1);

    // Diff panel chrome + content.
    let diff_title = if state.active_view == View::Revisions { "CHANGES" } else { "DIFF" };
    draw_panel_header(scene, fl.diff_header, diff_title, None, ctx);
    diff::render(scene, fl.diff_content, snapshot, diff, state, ctx);

    draw_statusbar(scene, &fl, ctx, snapshot);
}

// --- chrome ---------------------------------------------------------------

fn draw_toolbar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx, state: &UiState) {
    let t = ctx.theme;
    fill_rect(scene, fl.toolbar, t.crust);
    border_bottom(scene, fl.toolbar, t.surface1);

    let cy = fl.toolbar.center().y;
    let mut x = 10.0;

    // Logo dot (amber) as a stand-in for the svg logo.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        t.amber,
        None,
        &vello::kurbo::Circle::new((x + 6.0, cy), 5.0),
    );
    x += 20.0;
    x = seg_divider(scene, x, fl.toolbar, t);

    // Nav tabs segmented control.
    let tabs = [
        ("\u{25c9} Revisions", "1", View::Revisions),
        ("\u{2942} Branches", "2", View::Branches),
        ("\u{29c9} Merge", "3", View::Merge),
    ];
    for (label, key, view) in tabs {
        let active = state.active_view == view;
        x = nav_tab(scene, x, fl.toolbar, label, key, active, ctx);
    }
    x = seg_divider(scene, x + 4.0, fl.toolbar, t);

    // Drawer toggles (borderless).
    for (label, key) in [("\u{27f2} Oplog", "4"), ("\u{25d0} Evolog", "5")] {
        x = nav_tab(scene, x, fl.toolbar, label, key, false, ctx);
    }

    // Right group: theme toggle glyph.
    let sun = "\u{2600}";
    let tw = text::measure(&ctx.fonts.ui, theme::font::BASE, sun) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, theme::font::BASE, t.subtext0,
        fl.toolbar.x1 - tw - 12.0, baseline(cy, theme::font::BASE), sun,
    );
}

fn draw_tab_bar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx, snapshot: &Snapshot) {
    let t = ctx.theme;
    fill_rect(scene, fl.tab_bar, t.base);
    border_bottom(scene, fl.tab_bar, t.surface1);
    let cy = fl.tab_bar.center().y;

    // Single active tab = the repo name.
    let label = format!("\u{25aa} {}", snapshot.repo_name);
    let x0 = 16.0;
    let end = text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_SM, t.text,
        x0, baseline(cy, theme::font::FS_SM), &label,
    );
    // Active underline (amber).
    let lw = end - x0 + 10.0;
    fill_rect(scene, Rect::new(x0 - 10.0, fl.tab_bar.y1 - 2.0, x0 + lw, fl.tab_bar.y1), t.amber);

    // "+" new-tab affordance.
    text::draw_text(
        scene, &ctx.fonts.ui, theme::font::FS_LG, t.text_faint,
        end + 18.0, baseline(cy, theme::font::FS_LG), "+",
    );
}

fn draw_revset_bar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx) {
    let t = ctx.theme;
    let r = fl.revset_bar;
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;
    // "$" icon.
    text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_MD, t.text_faint,
        8.0, baseline(cy, theme::font::FS_MD), "$",
    );
    // Input box.
    let input = Rect::new(22.0, r.y0 + 4.0, r.x1 - 8.0, r.y1 - 4.0);
    fill_rect(scene, input, t.base);
    stroke_rect(scene, input, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_MD, t.surface2,
        input.x0 + 6.0, baseline(input.center().y, theme::font::FS_MD), "revset filter\u{2026}",
    );
}

fn draw_panel_header(
    scene: &mut Scene,
    r: Rect,
    title: &str,
    badge: Option<usize>,
    ctx: &RenderCtx,
) {
    let t = ctx.theme;
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;
    text::draw_text(
        scene, &ctx.fonts.ui_bold, theme::font::FS_SM, t.subtext1,
        r.x0 + 12.0, baseline(cy, theme::font::FS_SM), title,
    );
    if let Some(n) = badge {
        let s = n.to_string();
        let tw = text::measure(&ctx.fonts.ui, theme::font::FS_XS, &s) as f64;
        let bw = tw + 12.0;
        let pill = Rect::new(r.x1 - bw - 12.0, cy - 8.0, r.x1 - 12.0, cy + 8.0);
        fill_round(scene, pill, 8.0, t.surface0);
        text::draw_text(
            scene, &ctx.fonts.ui, theme::font::FS_XS, t.subtext0,
            pill.x0 + 6.0, baseline(cy, theme::font::FS_XS), &s,
        );
    }
}

fn draw_statusbar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx, snapshot: &Snapshot) {
    let t = ctx.theme;
    fill_rect(scene, fl.statusbar, t.crust);
    border_top(scene, fl.statusbar, t.surface1);
    let cy = fl.statusbar.center().y;
    let wc = snapshot
        .working_copy()
        .map(|n| &n.commit_id[..n.commit_id.len().min(8)])
        .unwrap_or("--------");
    let s = format!("{} revisions  |  @ {}", snapshot.revision_count(), wc);
    text::draw_text(
        scene, &ctx.fonts.ui, theme::font::FS_SM, t.subtext0,
        10.0, baseline(cy, theme::font::FS_SM), &s,
    );
}

// --- toolbar pieces -------------------------------------------------------

fn nav_tab(
    scene: &mut Scene,
    x: f64,
    bar: Rect,
    label: &str,
    key: &str,
    active: bool,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let cy = bar.center().y;
    let lw = text::measure(&ctx.fonts.ui, theme::font::FS_SM, label) as f64;
    let kw = text::measure(&ctx.fonts.mono, theme::font::FS_2XS, key) as f64;
    let pad = 8.0;
    let w = lw + 6.0 + kw + 8.0 + pad * 2.0;
    let pill = Rect::new(x, bar.y0 + 6.0, x + w, bar.y1 - 6.0);
    if active {
        fill_round(scene, pill, 4.0, t.bg_active);
    }
    let color = if active { t.amber } else { t.subtext0 };
    let lx = x + pad;
    text::draw_text(scene, &ctx.fonts.ui, theme::font::FS_SM, color, lx, baseline(cy, theme::font::FS_SM), label);
    // kbd hint chip.
    let kx = lx + lw + 6.0;
    let chip = Rect::new(kx, cy - 7.0, kx + kw + 6.0, cy + 7.0);
    stroke_rect(scene, chip, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_2XS, t.overlay0,
        kx + 3.0, baseline(cy, theme::font::FS_2XS), key,
    );
    x + w + 4.0
}

fn seg_divider(scene: &mut Scene, x: f64, bar: Rect, t: &Palette) -> f64 {
    let cy = bar.center().y;
    fill_rect(scene, Rect::new(x, cy - 7.0, x + 1.0, cy + 7.0), t.surface1);
    x + 9.0
}

// --- paint helpers (shared by graph/diff renderers) -----------------------

pub fn fill_rect(scene: &mut Scene, r: Rect, color: Color) {
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &r);
}

pub fn fill_round(scene: &mut Scene, r: Rect, radius: f64, color: Color) {
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &r.to_rounded_rect(radius));
}

pub fn stroke_rect(scene: &mut Scene, r: Rect, color: Color, width: f64) {
    scene.stroke(&Stroke::new(width), Affine::IDENTITY, color, None, &r);
}

pub fn border_bottom(scene: &mut Scene, r: Rect, color: Color) {
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Rect::new(r.x0, r.y1 - 1.0, r.x1, r.y1));
}

pub fn border_top(scene: &mut Scene, r: Rect, color: Color) {
    scene.fill(Fill::NonZero, Affine::IDENTITY, color, None, &Rect::new(r.x0, r.y0, r.x1, r.y0 + 1.0));
}

pub fn hline(scene: &mut Scene, x0: f64, x1: f64, y: f64, color: Color, width: f64) {
    scene.stroke(&Stroke::new(width), Affine::IDENTITY, color, None, &Line::new((x0, y), (x1, y)));
}

/// Baseline y for vertically-centering text of `size` px around center `cy`.
/// Approximation: DejaVu cap/x-height puts a good optical center ~0.32*size
/// below the geometric middle.
pub fn baseline(cy: f64, size: f32) -> f64 {
    cy + size as f64 * 0.32
}
