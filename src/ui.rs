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

// --- local chrome layout constants (no equivalent in theme::layout) ---------
//
// Toolbar metrics (ui-spec §3.1). lightjj uses `padding:0 10px`, `gap:8px`.
/// Horizontal inset of toolbar content from each edge (`.toolbar` padding).
const TOOLBAR_PAD_X: f64 = 10.0;
/// Gap between top-level toolbar clusters (`.toolbar` gap).
const TOOLBAR_GAP: f64 = 8.0;
/// `.toolbar-divider`: 1px × 14px, bg --surface1.
const TOOLBAR_DIVIDER_H: f64 = 14.0;
/// Segmented-control / nav-button vertical padding+content -> pill height.
/// `.seg-btn`/`.toolbar-nav-btn` use `padding:3px 10px`, line-height 1.4 over
/// --fs-sm(12) ≈ 17px text -> ~23px pill; we cap to fit the 34px bar.
const TOOLBAR_PILL_H: f64 = 23.0;
/// `.seg-btn` / `.btn` horizontal padding.
const BTN_PAD_X: f64 = 10.0;
/// `.toolbar-nav-btn` / `.toolbar-search` horizontal padding.
const NAV_BTN_PAD_X: f64 = 8.0;
/// Gap between a label and its trailing nav-hint kbd (CSS leaves a space glyph).
const KBD_GAP: f64 = 5.0;

// Revset filter bar (ui-spec §3.4).
/// `.revset-filter-bar` vertical padding (top & bottom).
const REVSET_PAD_Y: f64 = 4.0;
/// `.revset-filter-bar` horizontal padding.
const REVSET_PAD_X: f64 = 8.0;
/// `.revset-input` vertical padding -> input box height over --fs-md(13).
const REVSET_INPUT_PAD_Y: f64 = 3.0;
/// Preset-chips row total height. The chips themselves are `padding:2px 8px`
/// over --fs-sm(12) ≈ 18px tall; the row sits between the revset bar's 4px
/// bottom padding above and a 4px bottom inset below, matching the reference's
/// ~30px chip band beneath the revset input.
const PRESET_CHIPS_H: f64 = 31.0;
/// Chip-band top inset (slack above the chips inside the area).
const PRESET_CHIPS_PAD_TOP: f64 = 7.0;
/// `.preset-chips` bottom inset below the chips.
const PRESET_CHIPS_PAD_BOTTOM: f64 = 6.0;
/// `.preset-chip` horizontal padding.
const PRESET_CHIP_PAD_X: f64 = 8.0;
/// Gap between preset chips (`.preset-chips` gap:4).
const PRESET_CHIP_GAP: f64 = 4.0;

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
    /// Preset-chips row beneath the revset filter input (ui-spec §3.4).
    pub preset_chips: Rect,
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
        let preset_chips = Rect::new(0.0, revset_bar.y1, panel_w, revset_bar.y1 + PRESET_CHIPS_H);
        let graph_header =
            Rect::new(0.0, preset_chips.y1, panel_w, preset_chips.y1 + L::PANEL_HEADER_H);
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
            preset_chips,
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

    draw_toolbar(scene, &fl, ctx, state, snapshot);
    draw_tab_bar(scene, &fl, ctx, snapshot);

    // Revision panel chrome.
    draw_revset_bar(scene, &fl, ctx);
    draw_preset_chips(scene, &fl, ctx);
    draw_panel_header(
        scene,
        fl.graph_header,
        "REVISIONS",
        Some(snapshot.revision_count()),
        true,
        ctx,
    );
    graph::render(scene, fl.graph_content, snapshot, state, ctx);

    // Divider.
    fill_rect(scene, fl.divider, t.surface1);

    // Diff panel chrome + content.
    let diff_title = if state.active_view == View::Revisions { "CHANGES" } else { "DIFF" };
    draw_panel_header(scene, fl.diff_header, diff_title, None, false, ctx);
    diff::render(scene, fl.diff_content, snapshot, diff, state, ctx);

    draw_statusbar(scene, &fl, ctx, snapshot);
}

// --- chrome ---------------------------------------------------------------

fn draw_toolbar(
    scene: &mut Scene,
    fl: &FrameLayout,
    ctx: &RenderCtx,
    state: &UiState,
    snapshot: &Snapshot,
) {
    let t = ctx.theme;
    let bar = fl.toolbar;
    fill_rect(scene, bar, t.crust);
    border_bottom(scene, bar, t.surface1);

    let cy = bar.center().y;
    let mut x = TOOLBAR_PAD_X;

    // 1. Logo: 16×16 svg. Stand-in: a small amber "jj" mark drawn as a dot.
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        t.amber,
        None,
        &vello::kurbo::Circle::new((x + 8.0, cy), 5.0),
    );
    x += 16.0 + TOOLBAR_GAP;

    // 2. divider.
    x = toolbar_divider(scene, x, bar, t) + TOOLBAR_GAP;

    // 3. Workspace selector pill (◇ default ▾).
    x = workspace_pill(scene, x, bar, &snapshot.workspace_name, ctx) + TOOLBAR_GAP;

    // 4. divider.
    x = toolbar_divider(scene, x, bar, t) + TOOLBAR_GAP;

    // 5. Nav tabs = a `.seg` segmented control with three `.seg-btn`.
    // Icon + word kept separate so the icon can be drawn from whichever bundled
    // font covers the glyph (Inter lacks ◉/⑂/⧉/⟲/◐; see `icon_font`).
    let tabs = [
        ("\u{25c9}", "Revisions", "1", View::Revisions),
        ("\u{2942}", "Branches", "2", View::Branches),
        ("\u{29c9}", "Merge", "3", View::Merge),
    ];
    x = seg_control(scene, x, bar, &tabs, state.active_view, ctx) + TOOLBAR_GAP;

    // 6. divider.
    x = toolbar_divider(scene, x, bar, t) + TOOLBAR_GAP;

    // 7. Drawer toggles (borderless `.toolbar-nav-btn`).
    for (icon, word, key) in [("\u{27f2}", "Oplog", "4"), ("\u{25d0}", "Evolog", "5")] {
        x = nav_btn(scene, x, bar, icon, word, key, ctx);
    }

    // 8. divider.
    x = toolbar_divider(scene, x + (TOOLBAR_GAP - 4.0), bar, t) + TOOLBAR_GAP;

    // 9. Search button (`Search…` + ⌘K/Ctrl+K kbd chip).
    search_button(scene, x, bar, ctx);

    // Right group (space-between): theme toggle glyph at the far right.
    let sun = "\u{2600}";
    let sun_sz = theme::font::BASE;
    let tw = text::measure(&ctx.fonts.ui, sun_sz, sun) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, sun_sz, t.subtext0,
        bar.x1 - TOOLBAR_PAD_X - 6.0 - tw, baseline_for(cy, sun_sz, &ctx.fonts.ui), sun,
    );
}

fn draw_tab_bar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx, snapshot: &Snapshot) {
    let t = ctx.theme;
    let bar = fl.tab_bar;
    fill_rect(scene, bar, t.base);
    border_bottom(scene, bar, t.surface1);
    let cy = bar.center().y;
    let sz = theme::font::FS_SM;

    // Single active `.tab` = the repo name. `padding-left:10px` on the bar,
    // `padding:0 10px` + `gap:6px` on the tab. Active: glyph + name in amber/text.
    let tab_pad = 10.0;
    let tab_x0 = 10.0; // bar padding-left
    let mut x = tab_x0 + tab_pad;
    // Active-tab glyph `▪` (amber, --fs-3xs).
    let gsz = theme::font::FS_3XS;
    x = text::draw_text(
        scene, &ctx.fonts.mono, gsz, t.amber,
        x, baseline_for(cy, gsz, &ctx.fonts.mono), "\u{25aa}",
    ) + 6.0;
    let end = text::draw_text(
        scene, &ctx.fonts.mono, sz, t.text,
        x, baseline_for(cy, sz, &ctx.fonts.mono), &snapshot.repo_name,
    );
    // Active 2px amber bottom-border spanning the tab box.
    let tab_x1 = end + tab_pad;
    fill_rect(scene, Rect::new(tab_x0, bar.y1 - 2.0, tab_x1, bar.y1), t.amber);

    // `.tab-new` `+` affordance: --fs-lg, --text-faint.
    text::draw_text(
        scene, &ctx.fonts.ui, theme::font::FS_LG, t.text_faint,
        tab_x1 + 13.0, baseline_for(cy, theme::font::FS_LG, &ctx.fonts.ui), "+",
    );
}

fn draw_revset_bar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx) {
    let t = ctx.theme;
    let r = fl.revset_bar;
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;
    let sz = theme::font::FS_MD;
    // "$" icon: --text-faint, --fs-md, weight 700.
    let icon_end = text::draw_text(
        scene, &ctx.fonts.mono, sz, t.text_faint,
        r.x0 + REVSET_PAD_X, baseline_for(cy, sz, &ctx.fonts.mono), "$",
    );
    // Input box: flex:1, bg --base, 1px --surface1, radius 3, padding 3px 6px.
    let input = Rect::new(
        icon_end + 6.0,
        r.y0 + REVSET_PAD_Y,
        r.x1 - REVSET_PAD_X,
        r.y1 - REVSET_PAD_Y,
    );
    fill_round(scene, input, 3.0, t.base);
    stroke_round(scene, input, 3.0, t.surface1, 1.0);
    // Placeholder (the default revset), color --surface1 per spec.
    text::draw_text(
        scene, &ctx.fonts.mono, sz, t.surface2,
        input.x0 + 6.0 + REVSET_INPUT_PAD_Y, baseline_for(input.center().y, sz, &ctx.fonts.mono),
        "present(@) | ancestors(immutable_heads().., 2) | trunk()",
    );
}

/// Preset-chips row (`.preset-chips`): a wrapping row of small chips
/// (--fs-sm, padding 2x8, radius 3, bg --surface0, color --subtext0); the
/// active chip gets an amber border + amber text (ui-spec §3.4). The first
/// chip ("WIP") is active here to mirror the reference's default selection.
fn draw_preset_chips(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx) {
    let t = ctx.theme;
    let r = fl.preset_chips;
    // Same surface as the revset bar; no own border (the chips float on --mantle).
    fill_rect(scene, r, t.mantle);
    let sz = theme::font::FS_SM;

    // Chip row sits between a top inset and a bottom inset within the area.
    let chip_h = r.height() - PRESET_CHIPS_PAD_TOP - PRESET_CHIPS_PAD_BOTTOM;
    let cy = r.y0 + PRESET_CHIPS_PAD_TOP + chip_h / 2.0;
    let presets = ["WIP", "My work", "Default", "All", "Conflicts", "Divergent"];

    let mut x = r.x0 + REVSET_PAD_X;
    for (i, label) in presets.iter().enumerate() {
        let lw = text::measure(&ctx.fonts.ui, sz, label) as f64;
        let w = PRESET_CHIP_PAD_X + lw + PRESET_CHIP_PAD_X;
        let chip = Rect::new(x, cy - chip_h / 2.0, x + w, cy + chip_h / 2.0);
        let active = i == 0;
        fill_round(scene, chip, 3.0, t.surface0);
        let color = if active {
            stroke_round(scene, chip, 3.0, t.amber, 1.0);
            t.amber
        } else {
            t.subtext0
        };
        text::draw_text(
            scene, &ctx.fonts.ui, sz, color,
            x + PRESET_CHIP_PAD_X, baseline_for(cy, sz, &ctx.fonts.ui), label,
        );
        x += w + PRESET_CHIP_GAP;
    }
}

fn draw_panel_header(
    scene: &mut Scene,
    r: Rect,
    title: &str,
    badge: Option<usize>,
    nav_hints: bool,
    ctx: &RenderCtx,
) {
    let t = ctx.theme;
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;
    let sz = theme::font::FS_SM;
    // `.panel-title`: --fs-sm, weight 700, UPPERCASE, color --subtext1.
    let title_end = text::draw_text(
        scene, &ctx.fonts.ui_bold, sz, t.subtext1,
        r.x0 + 12.0, baseline_for(cy, sz, &ctx.fonts.ui_bold), title,
    );
    // RevisionGraph header: `<kbd>j</kbd><kbd>k</kbd>` nav hints after the title.
    if nav_hints {
        let mut kx = title_end + 6.0;
        kx = nav_hint_kbd(scene, kx, cy, "j", t.overlay0, ctx) + 2.0;
        nav_hint_kbd(scene, kx, cy, "k", t.overlay0, ctx);
    }
    // `.panel-badge`: bg --surface0, color --subtext0, padding 0 6, radius 8,
    // --fs-xs, weight 600, right-aligned.
    if let Some(n) = badge {
        let s = n.to_string();
        let bsz = theme::font::FS_XS;
        let tw = text::measure(&ctx.fonts.ui_bold, bsz, &s) as f64;
        let bw = tw + 6.0 * 2.0;
        let pill = Rect::new(r.x1 - 12.0 - bw, cy - 8.0, r.x1 - 12.0, cy + 8.0);
        fill_round(scene, pill, 8.0, t.surface0);
        text::draw_text(
            scene, &ctx.fonts.ui_bold, bsz, t.subtext0,
            pill.x0 + 6.0, baseline_for(cy, bsz, &ctx.fonts.ui_bold), &s,
        );
    }
}

fn draw_statusbar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx, snapshot: &Snapshot) {
    let t = ctx.theme;
    let r = fl.statusbar;
    fill_rect(scene, r, t.crust);
    border_top(scene, r, t.surface1);
    let cy = r.center().y;
    let sz = theme::font::FS_SM;
    // lightjj statusText: `{N} revisions | @ {change-id 8}`.
    let wc = snapshot
        .working_copy()
        .map(|n| &n.change_id[..n.change_id.len().min(8)])
        .unwrap_or("--------");
    let s = format!("{} revisions | @ {}", snapshot.revision_count(), wc);
    text::draw_text(
        scene, &ctx.fonts.ui, sz, t.subtext0,
        r.x0 + TOOLBAR_PAD_X, baseline_for(cy, sz, &ctx.fonts.ui), &s,
    );
}

// --- toolbar pieces -------------------------------------------------------

/// `.toolbar-divider`: 1px wide × 14px tall, bg --surface1. Returns its right x.
fn toolbar_divider(scene: &mut Scene, x: f64, bar: Rect, t: &Palette) -> f64 {
    let cy = bar.center().y;
    let h = TOOLBAR_DIVIDER_H / 2.0;
    fill_rect(scene, Rect::new(x, cy - h, x + 1.0, cy + h), t.surface1);
    x + 1.0
}

/// A `.nav-hint` kbd: mono --fs-2xs, color, 1px --surface1 border, padding 0 3,
/// radius 3, drawn left-anchored at `x`, vertically centered on `cy`. Returns
/// the chip's right edge x.
fn nav_hint_kbd(scene: &mut Scene, x: f64, cy: f64, key: &str, color: Color, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let sz = theme::font::FS_2XS;
    let kw = text::measure(&ctx.fonts.mono, sz, key) as f64;
    // padding 0 3 -> 3px each side; chip a touch taller than the glyph box.
    let pad = 3.0;
    let chip = Rect::new(x, cy - 7.0, x + kw + pad * 2.0, cy + 7.0);
    // `.nav-hint`: no background, just a 1px --surface1 border.
    stroke_round(scene, chip, 3.0, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, sz, color,
        x + pad, baseline_for(cy, sz, &ctx.fonts.mono), key,
    );
    chip.x1
}

/// Pick the bundled font that covers `glyph` (so symbol icons aren't drawn as
/// Inter's tall tofu box). JetBrains Mono covers a few symbol glyphs Inter
/// lacks (e.g. `◉`, `◇`, `▾`, `▪`); fall back to the UI font otherwise. (The
/// remaining nav icons `⑂ ⧉ ⟲ ◐` exist in no bundled font and will tofu — that
/// needs a glyph added to assets/fonts, which is out of this module's scope.)
fn icon_font<'a>(glyph: &str, bold: bool, ctx: &'a RenderCtx) -> &'a vello::peniko::FontData {
    const MONO_COVERED: &[&str] = &["\u{25c9}", "\u{25c7}", "\u{25be}", "\u{25aa}"];
    if MONO_COVERED.contains(&glyph) {
        &ctx.fonts.mono
    } else if bold {
        &ctx.fonts.ui_bold
    } else {
        &ctx.fonts.ui
    }
}

/// Gap between a nav icon glyph and its following word.
const ICON_LABEL_GAP: f64 = 3.0;

/// Width of an `icon word` run (icon in its covering font, word in the UI font).
fn icon_label_w(icon: &str, word: &str, sz: f32, bold: bool, ctx: &RenderCtx) -> f64 {
    let ifont = icon_font(icon, bold, ctx);
    let wfont = if bold { &ctx.fonts.ui_bold } else { &ctx.fonts.ui };
    text::measure(ifont, sz, icon) as f64 + ICON_LABEL_GAP + text::measure(wfont, sz, word) as f64
}

/// Draw an `icon word` run at `x`, baseline-centered on `cy`. Returns end x.
fn draw_icon_label(
    scene: &mut Scene,
    x: f64,
    cy: f64,
    icon: &str,
    word: &str,
    sz: f32,
    color: Color,
    bold: bool,
    ctx: &RenderCtx,
) -> f64 {
    let ifont = icon_font(icon, bold, ctx);
    let wfont = if bold { &ctx.fonts.ui_bold } else { &ctx.fonts.ui };
    let ix = text::draw_text(scene, ifont, sz, color, x, baseline_for(cy, sz, ifont), icon)
        + ICON_LABEL_GAP;
    text::draw_text(scene, wfont, sz, color, ix, baseline_for(cy, sz, wfont), word)
}

/// Nav-tab segmented control (`.seg` wrapping three `.seg-btn`). One outer
/// rounded box (bg --surface0, 1px --surface1), each button `padding:3px 10px`,
/// active button bg --bg-active, amber text, weight 600. Returns the right x.
fn seg_control(
    scene: &mut Scene,
    x: f64,
    bar: Rect,
    tabs: &[(&str, &str, &str, View); 3],
    active_view: View,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let cy = bar.center().y;
    let sz = theme::font::FS_SM;

    // Pre-measure each button's content width.
    let btn_w = |icon: &str, word: &str, key: &str, bold: bool| -> f64 {
        let lw = icon_label_w(icon, word, sz, bold, ctx);
        let kw = text::measure(&ctx.fonts.mono, theme::font::FS_2XS, key) as f64;
        BTN_PAD_X + lw + KBD_GAP + (kw + 6.0) + BTN_PAD_X
    };
    let total: f64 = tabs
        .iter()
        .map(|(ic, w, k, v)| btn_w(ic, w, k, active_view == *v))
        .sum();

    let top = cy - TOOLBAR_PILL_H / 2.0;
    let bot = cy + TOOLBAR_PILL_H / 2.0;
    let outer = Rect::new(x, top, x + total, bot);
    fill_round(scene, outer, 4.0, t.surface0);
    stroke_round(scene, outer, 4.0, t.surface1, 1.0);

    let mut bx = x;
    for (icon, word, key, view) in tabs {
        let active = active_view == *view;
        let w = btn_w(icon, word, key, active);
        let btn = Rect::new(bx, top, bx + w, bot);
        let color = if active {
            fill_round(scene, btn, 4.0, t.bg_active);
            t.amber
        } else {
            t.subtext0
        };
        let lx = bx + BTN_PAD_X;
        let lend = draw_icon_label(scene, lx, cy, icon, word, sz, color, active, ctx);
        // trailing nav-hint kbd; active kbd gets amber text.
        let kcolor = if active { t.amber } else { t.overlay0 };
        nav_hint_kbd(scene, lend + KBD_GAP, cy, key, kcolor, ctx);
        bx += w;
    }
    outer.x1
}

/// `.toolbar-nav-btn` (borderless drawer toggle): `padding:3px 8px`, no border,
/// color --subtext0, --fs-sm + a trailing nav-hint kbd. Returns the right x
/// (incl. the toolbar gap to the next element).
fn nav_btn(scene: &mut Scene, x: f64, bar: Rect, icon: &str, word: &str, key: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let cy = bar.center().y;
    let sz = theme::font::FS_SM;
    let lx = x + NAV_BTN_PAD_X;
    let lend = draw_icon_label(scene, lx, cy, icon, word, sz, t.subtext0, false, ctx);
    let kend = nav_hint_kbd(scene, lend + KBD_GAP, cy, key, t.overlay0, ctx);
    kend + NAV_BTN_PAD_X + TOOLBAR_GAP
}

/// Workspace selector pill (`.toolbar-ws-btn`): `padding:3px 8px`, 1px --surface1
/// border, radius 4, mono --fs-sm. Glyph ◇ (--subtext0) + name (--text). The
/// chevron `▾` only appears with >1 workspace (single workspace -> omitted, as
/// in the reference). Returns the right x.
fn workspace_pill(scene: &mut Scene, x: f64, bar: Rect, name: &str, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let cy = bar.center().y;
    let sz = theme::font::FS_SM;
    let glyph = "\u{25c7}"; // ◇
    let gw = text::measure(&ctx.fonts.mono, theme::font::FS_XS, glyph) as f64;
    let nw = text::measure(&ctx.fonts.mono, sz, name) as f64;
    let inner = gw + 6.0 + nw;
    let w = NAV_BTN_PAD_X + inner + NAV_BTN_PAD_X;
    let top = cy - TOOLBAR_PILL_H / 2.0;
    let pill = Rect::new(x, top, x + w, cy + TOOLBAR_PILL_H / 2.0);
    stroke_round(scene, pill, 4.0, t.surface1, 1.0);

    let mut tx = x + NAV_BTN_PAD_X;
    tx = text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_XS, t.subtext0,
        tx, baseline_for(cy, theme::font::FS_XS, &ctx.fonts.mono), glyph,
    ) + 6.0;
    text::draw_text(
        scene, &ctx.fonts.mono, sz, t.text,
        tx, baseline_for(cy, sz, &ctx.fonts.mono), name,
    );
    pill.x1
}

/// Search button (`.toolbar-search`): `padding:3px 8px`, bg --surface0, radius 4,
/// color --text-faint, --fs-sm. "Search…" text + a `Ctrl+K` kbd chip. Returns
/// the right x.
fn search_button(scene: &mut Scene, x: f64, bar: Rect, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let cy = bar.center().y;
    let sz = theme::font::FS_SM;
    let label = "Search\u{2026}";
    let key = "Ctrl+K";
    let lw = text::measure(&ctx.fonts.ui, sz, label) as f64;
    let kw = text::measure(&ctx.fonts.mono, theme::font::FS_2XS, key) as f64;
    let kbd_w = kw + 4.0 * 2.0; // kbd padding 0 4
    let inner = lw + 8.0 + kbd_w;
    let w = NAV_BTN_PAD_X + inner + NAV_BTN_PAD_X;
    let top = cy - TOOLBAR_PILL_H / 2.0;
    let pill = Rect::new(x, top, x + w, cy + TOOLBAR_PILL_H / 2.0);
    fill_round(scene, pill, 4.0, t.surface0);

    let lx = x + NAV_BTN_PAD_X;
    let lend = text::draw_text(
        scene, &ctx.fonts.ui, sz, t.text_faint, lx, baseline_for(cy, sz, &ctx.fonts.ui), label,
    );
    // search kbd chip (mono --fs-2xs, --text-faint, 1px --surface1, pad 0 4, r3).
    let kx = lend + 8.0;
    let chip = Rect::new(kx, cy - 7.0, kx + kbd_w, cy + 7.0);
    stroke_round(scene, chip, 3.0, t.surface1, 1.0);
    text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_2XS, t.text_faint,
        kx + 4.0, baseline_for(cy, theme::font::FS_2XS, &ctx.fonts.mono), key,
    );
    pill.x1
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

pub fn stroke_round(scene: &mut Scene, r: Rect, radius: f64, color: Color, width: f64) {
    scene.stroke(&Stroke::new(width), Affine::IDENTITY, color, None, &r.to_rounded_rect(radius));
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

/// Baseline y for vertically-centering text of `size` px around center `cy`,
/// font-agnostic. Used by graph/diff renderers that don't pass a font.
///
/// Approximation: the optical center of Latin text sits ~half the cap-height
/// above the baseline. For the bundled UI/mono fonts cap-height ≈ 0.73·size,
/// so the baseline lands ~0.365·size below the geometric middle. (Prefer
/// [`baseline_for`] when a font is available — it uses real metrics.)
pub fn baseline(cy: f64, size: f32) -> f64 {
    cy + size as f64 * 0.365
}

/// Metric-based vertical centering: places the baseline so the font's optical
/// box (ascent above / descent below the baseline) is centered on `cy`.
///
/// With `ascent`/`descent` from [`text::line_metrics`], the text box spans
/// `[baseline - ascent, baseline + |descent|]`; centering that box on `cy`
/// gives `baseline = cy + (ascent - |descent|)/2`. This matches how lightjj's
/// flexbox `align-items:center` centers the line box of chrome text.
pub fn baseline_for(cy: f64, size: f32, font: &vello::peniko::FontData) -> f64 {
    let m = text::line_metrics(font, size);
    cy + (m.ascent - m.descent.abs()) as f64 / 2.0
}
