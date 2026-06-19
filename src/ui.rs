//! UI composition: turns a [`Snapshot`] (+ optional [`CommitDiff`]) into a Vello
//! [`Scene`]. This module owns the **chrome shell** (toolbar, tab bar, panel
//! headers, status bar, panel split) and the overall frame layout, then
//! delegates the two big content areas to [`graph`] and [`diff`].
//!
//! Contract for the content renderers: each exposes
//! `render(scene, rect, ..., ctx)` and paints ONLY within `rect`. The shell and
//! layout are stable; implement the panels against this interface.

pub mod branches;
pub mod diff;
pub mod graph;
pub mod oplog;
pub mod palette;

use vello::kurbo::{Affine, BezPath, Cap, Join, Line, Rect, Stroke};
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

/// Theme polarity. `build_scene` resolves this to a [`Palette`] each frame so
/// the whole UI re-themes at runtime (lightjj's `t` key toggles dark/light).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
}

impl Theme {
    /// The concrete palette for this polarity.
    pub fn palette(self) -> &'static Palette {
        match self {
            Theme::Dark => &theme::DARK,
            Theme::Light => &theme::LIGHT,
        }
    }
}

/// Mutable UI state the renderer reads (cursor, scroll, sizing).
#[derive(Clone, Debug)]
pub struct UiState {
    pub active_view: View,
    /// Active theme polarity; `build_scene` picks the palette from this.
    pub theme: Theme,
    /// Index into `snapshot.nodes` of the selected revision.
    pub selected: usize,
    /// Hovered revision index (JS-tracked hover in lightjj), if any.
    pub hovered: Option<usize>,
    pub panel_width: f64,
    /// Vertical scroll offset (px) of the graph list.
    pub graph_scroll: f64,
    /// Vertical scroll offset (px) of the diff content.
    pub diff_scroll: f64,
    /// Oplog bottom-drawer open (lightjj's `4` toggle).
    pub oplog_open: bool,
    /// Evolog bottom-drawer open (lightjj's `5` toggle).
    pub evolog_open: bool,
    /// Command-palette overlay open (Cmd+K / Ctrl+K).
    pub palette_open: bool,
    /// Live command-palette query (drives the input row + fuzzy filter).
    pub palette_query: String,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active_view: View::Revisions,
            theme: Theme::Dark,
            selected: 0,
            hovered: None,
            panel_width: L::REVISION_PANEL_DEFAULT_W,
            graph_scroll: 0.0,
            diff_scroll: 0.0,
            oplog_open: false,
            evolog_open: false,
            palette_open: false,
            palette_query: String::new(),
        }
    }
}

/// Per-frame inputs beyond the [`Snapshot`]/[`UiState`]: data the drawers need
/// that the callers (`shot`/`drive`) load from the backend. Kept as a small
/// struct so `build_scene`'s signature is stable as new drawers land.
#[derive(Default)]
pub struct Frame<'a> {
    /// Operation-log entries (newest-first), shown by the Oplog drawer. Only
    /// populated under the `jjlib` feature; empty otherwise (the mock build
    /// has no op-store, so the drawer renders its empty state).
    #[cfg(feature = "jjlib")]
    pub oplog: &'a [crate::model::jjlib::OpEntry],
    /// Lifetime anchor so the struct is generic over `'a` in every build.
    pub _marker: std::marker::PhantomData<&'a ()>,
}

/// Shared rendering context: fonts + active theme palette. STABLE shape —
/// `graph`/`diff`/`branches`/`oplog`/`palette` all read `ctx.theme: &Palette`.
/// `build_scene` constructs this each frame from the state-selected palette.
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
    /// Diff panel chrome header ("CHANGES"/"DIFF"). `None` in the Revisions view:
    /// lightjj shows the RevisionHeader (drawn inside `diff.rs`) as the very top
    /// of the diff panel, with no chrome title bar above it.
    pub diff_header: Option<Rect>,
    pub diff_content: Rect,
    /// Open bottom drawer (Oplog/Evolog), spanning the full width above the
    /// statusbar. `None` when no drawer is open. When present, the body areas
    /// above (graph/diff/branches) are shortened to `drawer.y0`.
    pub drawer: Option<Rect>,
    pub statusbar: Rect,
}

/// Height of an open bottom drawer (Oplog/Evolog). lightjj's `.oplog-panel`
/// floats above the statusbar; ~38% of an 800px frame reads like the reference.
const DRAWER_H: f64 = 300.0;

impl FrameLayout {
    pub fn compute(w: f64, h: f64, panel_w: f64, view: View, drawer_open: bool) -> Self {
        let panel_w = panel_w.clamp(L::REVISION_PANEL_MIN_W, L::REVISION_PANEL_MAX_W);
        let mut y = 0.0;
        let toolbar = Rect::new(0.0, y, w, y + L::TOOLBAR_H);
        y += L::TOOLBAR_H;
        let tab_bar = Rect::new(0.0, y, w, y + L::TAB_BAR_H);
        y += L::TAB_BAR_H;
        let body_top = y;
        let statusbar_top = h - L::STATUSBAR_H;
        // An open drawer steals a band off the bottom of the body, sitting just
        // above the statusbar; the body bottom rises to the drawer top.
        let drawer = if drawer_open {
            let dh = DRAWER_H.min((statusbar_top - body_top).max(0.0) * 0.6);
            Some(Rect::new(0.0, statusbar_top - dh, w, statusbar_top))
        } else {
            None
        };
        let body_bot = drawer.map(|d| d.y0).unwrap_or(statusbar_top);

        let revset_bar = Rect::new(0.0, body_top, panel_w, body_top + L::REVSET_BAR_H);
        let preset_chips = Rect::new(0.0, revset_bar.y1, panel_w, revset_bar.y1 + PRESET_CHIPS_H);
        let graph_header =
            Rect::new(0.0, preset_chips.y1, panel_w, preset_chips.y1 + L::PANEL_HEADER_H);
        let graph_content = Rect::new(0.0, graph_header.y1, panel_w, body_bot);

        let divider = Rect::new(panel_w, body_top, panel_w + L::PANEL_DIVIDER_W, body_bot);
        let diff_x = divider.x1;
        // Neither the Revisions NOR the Branches view has a chrome title bar:
        // lightjj's RevisionHeader (Revisions, drawn by diff.rs) and the
        // BookmarksPanel's own `.bp-header` (Branches) are the very top of the
        // right column — there is no "DIFF"/"BRANCHES" panel-header above them.
        // Only the Merge placeholder keeps a chrome header. When there's no
        // header, the content spans the full body height from `body_top`.
        let (diff_header, diff_content) = if view == View::Merge {
            let header = Rect::new(diff_x, body_top, w, body_top + L::PANEL_HEADER_H);
            let content = Rect::new(diff_x, header.y1, w, body_bot);
            (Some(header), content)
        } else {
            (None, Rect::new(diff_x, body_top, w, body_bot))
        };

        let statusbar = Rect::new(0.0, statusbar_top, w, h);

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
            drawer,
            statusbar,
        }
    }
}

/// Build the full-window scene. `width`/`height` are logical px.
///
/// The caller passes `fonts` + the full `state` (and per-frame `frame` inputs);
/// `build_scene` resolves the active [`Palette`] from `state.theme` and builds
/// the [`RenderCtx`] the content renderers consume, so the whole UI re-themes
/// at runtime when `state.theme` flips (the `t` key). The `RenderCtx` shape is
/// stable — graph/diff/branches/oplog all read `ctx.theme: &Palette` unchanged.
pub fn build_scene(
    scene: &mut Scene,
    snapshot: &Snapshot,
    diff: Option<&CommitDiff>,
    state: &UiState,
    fonts: &Fonts,
    frame: &Frame,
    width: f64,
    height: f64,
) {
    // Resolve the palette from state this frame; construct the (stable) ctx.
    let palette = *state.theme.palette();
    let ctx = &RenderCtx { fonts, theme: &palette };
    let t = ctx.theme;

    let drawer_open = state.oplog_open || state.evolog_open;
    let fl = FrameLayout::compute(width, height, state.panel_width, state.active_view, drawer_open);

    // Base background (everything paints on top).
    fill_rect(scene, Rect::new(0.0, 0.0, width, height), t.base);

    draw_toolbar(scene, &fl, ctx, state, snapshot);
    draw_tab_bar(scene, &fl, ctx, snapshot);

    // Revision panel chrome (the left column keeps the graph in every view —
    // lightjj only HIDES it in the Merge view; Branches keeps it as a sibling
    // and fills the RIGHT column with the bookmarks list).
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

    // Right column: dispatch on the active view.
    match state.active_view {
        // Branches view — the bookmarks list replaces the diff panel. lightjj's
        // BookmarksPanel has NO chrome title bar: its own `.bp-header` (filter
        // input + sort control + count) is the top of the column, so the panel
        // fills the whole right column from `body_top` with no header above it.
        View::Branches => {
            branches::render(scene, fl.diff_content, snapshot, ctx);
        }
        // Merge view — full merge surface is out of scope; show a placeholder
        // panel so the view is reachable without rendering a misleading diff.
        View::Merge => {
            if let Some(header) = fl.diff_header {
                draw_panel_header(scene, header, "MERGE", None, false, ctx);
            }
            draw_merge_placeholder(scene, fl.diff_content, ctx);
        }
        // Revisions view — the diff panel. No "CHANGES" title bar: the
        // RevisionHeader (drawn inside diff::render) is the panel top.
        View::Revisions => {
            if let Some(diff_header) = fl.diff_header {
                draw_panel_header(scene, diff_header, "DIFF", None, false, ctx);
            }
            diff::render(scene, fl.diff_content, snapshot, diff, state, ctx);
        }
    }

    // Bottom drawer (Oplog/Evolog), above the statusbar.
    if let Some(drawer) = fl.drawer {
        draw_drawer(scene, drawer, state, frame, ctx);
    }

    draw_statusbar(scene, &fl, ctx, snapshot);

    // Command palette overlay, on top of everything.
    if state.palette_open {
        let viewport = Rect::new(0.0, 0.0, width, height);
        palette::render(scene, viewport, &state.palette_query, ctx);
    }
}

/// Render the open bottom drawer. Oplog uses the real op-log renderer (fed the
/// ops loaded by the caller); Evolog is a placeholder until its data path lands.
fn draw_drawer(scene: &mut Scene, drawer: Rect, state: &UiState, frame: &Frame, ctx: &RenderCtx) {
    let t = ctx.theme;
    // 1px top border separating the drawer from the body above it.
    fill_rect(scene, Rect::new(drawer.x0, drawer.y0, drawer.x1, drawer.y0 + 1.0), t.surface1);
    let body = Rect::new(drawer.x0, drawer.y0 + 1.0, drawer.x1, drawer.y1);

    if state.oplog_open {
        #[cfg(feature = "jjlib")]
        {
            oplog::render(scene, body, frame.oplog, ctx);
        }
        #[cfg(not(feature = "jjlib"))]
        {
            // No op-store without the jjlib feature; show the drawer chrome
            // with an empty-state message so the toggle is still visible.
            let _ = frame;
            draw_drawer_placeholder(scene, body, "OPERATION LOG", "No operations (mock build)", ctx);
        }
    } else if state.evolog_open {
        draw_drawer_placeholder(scene, body, "EVOLUTION LOG", "Evolution log — not yet wired", ctx);
    }
}

/// A minimal drawer placeholder: panel header + a centered faint message.
/// Used for the Evolog drawer (no renderer yet) and the non-jjlib Oplog.
fn draw_drawer_placeholder(scene: &mut Scene, rect: Rect, title: &str, msg: &str, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    let header = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + L::PANEL_HEADER_H);
    fill_rect(scene, header, t.mantle);
    border_bottom(scene, header, t.surface0);
    let hcy = header.center().y;
    let hsz = theme::font::FS_SM;
    text::draw_text(
        scene, &ctx.fonts.ui_bold, hsz, t.subtext1,
        header.x0 + 12.0, baseline_for(hcy, hsz, &ctx.fonts.ui_bold), title,
    );
    let sz = theme::font::BASE;
    let w = text::measure(&ctx.fonts.ui, sz, msg) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, sz, t.text_faint,
        rect.center().x - w / 2.0, header.y1 + 40.0, msg,
    );
}

/// Merge-view placeholder panel (full 3-pane merge surface is out of scope).
fn draw_merge_placeholder(scene: &mut Scene, rect: Rect, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    let msg = "Merge view — not yet implemented";
    let sz = theme::font::FS_LG;
    let w = text::measure(&ctx.fonts.ui, sz, msg) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, sz, t.text_faint,
        rect.center().x - w / 2.0, rect.center().y, msg,
    );
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

    // 1. Logo: the actual lightjj `/logo.svg` mark — a stroked amber lightning
    // bolt with node dots — drawn from its real BezPaths into the 16px box.
    draw_logo(scene, x, cy, 16.0, t.amber);
    x += 16.0 + TOOLBAR_GAP;

    // 2. divider.
    x = toolbar_divider(scene, x, bar, t) + TOOLBAR_GAP;

    // 3. Workspace selector pill (◇ default ▾). lightjj's markup places the
    // `.seg` nav control DIRECTLY after the workspace pill — there is NO
    // `.toolbar-divider` between them (the toolbar's only dividers are
    // logo↔pill, seg↔drawer-toggles, and drawer-toggles↔search). An extra
    // divider here was pushing the whole `◉ Revisions` control ~7px right of
    // the reference (compare2 REAL component #1, the gap before the ◉ glyph).
    x = workspace_pill(scene, x, bar, &snapshot.workspace_name, ctx) + TOOLBAR_GAP;

    // 4. Nav tabs = a `.seg` segmented control with three `.seg-btn`.
    // Icon + word kept separate so the icon gets its own metric-centered baseline
    // (it resolves in a symbol fallback font with different vertical metrics than
    // Inter; see `draw_icon_label`). Codepoints match lightjj's toolbar markup
    // (App.svelte): ◉ Revisions, ⑂ Branches (U+2442), ⧉ Merge.
    let tabs = [
        ("\u{25c9}", "Revisions", "1", View::Revisions),
        ("\u{2442}", "Branches", "2", View::Branches),
        ("\u{29c9}", "Merge", "3", View::Merge),
    ];
    x = seg_control(scene, x, bar, &tabs, state.active_view, ctx) + TOOLBAR_GAP;

    // 5. divider.
    x = toolbar_divider(scene, x, bar, t) + TOOLBAR_GAP;

    // 6. Drawer toggles (borderless `.toolbar-nav-btn`).
    for (icon, word, key) in [("\u{27f2}", "Oplog", "4"), ("\u{25d0}", "Evolog", "5")] {
        x = nav_btn(scene, x, bar, icon, word, key, ctx);
    }

    // 7. divider.
    x = toolbar_divider(scene, x + (TOOLBAR_GAP - 4.0), bar, t) + TOOLBAR_GAP;

    // 8. Search button (`Search…` + ⌘K/Ctrl+K kbd chip).
    search_button(scene, x, bar, ctx);

    // Right group (space-between): theme toggle glyph at the far right. lightjj
    // shows ☀ (U+2600) in dark mode / ● in light mode; we render the dark-mode ☀.
    let sun = "\u{2600}";
    let sun_sz = theme::font::BASE;
    let tw = text::measure(&ctx.fonts.ui, sun_sz, sun) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, sun_sz, t.subtext0,
        bar.x1 - TOOLBAR_PAD_X - 6.0 - tw,
        text::icon_baseline(&ctx.fonts.ui, sun_sz, '\u{2600}', cy), sun,
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

    // `.tab-new` `+` affordance: --fs-lg, --text-faint. `.tab-new` is `font:
    // inherit` under the `.tab-bar`'s `--font-mono`, so the `+` is mono.
    text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_LG, t.text_faint,
        tab_x1 + 13.0, baseline_for(cy, theme::font::FS_LG, &ctx.fonts.mono), "+",
    );
}

fn draw_revset_bar(scene: &mut Scene, fl: &FrameLayout, ctx: &RenderCtx) {
    let t = ctx.theme;
    let r = fl.revset_bar;
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;
    let sz = theme::font::FS_MD;
    // "$" icon: `.revset-icon` is a bare <span> — no font-family override, so it
    // inherits body's `--font-ui` (Inter), NOT mono. --text-faint, --fs-md,
    // weight 700 (use ui_bold for the 700 weight).
    let icon_end = text::draw_text(
        scene, &ctx.fonts.ui_bold, sz, t.text_faint,
        r.x0 + REVSET_PAD_X, baseline_for(cy, sz, &ctx.fonts.ui_bold), "$",
    );
    // Trailing `?` help button (circular, --text-faint, 1px --surface1 border),
    // right-anchored at the bar's padding edge.
    let help_d = r.height() - REVSET_PAD_Y * 2.0;
    let help = Rect::new(r.x1 - REVSET_PAD_X - help_d, cy - help_d / 2.0, r.x1 - REVSET_PAD_X, cy + help_d / 2.0);
    stroke_round(scene, help, help_d / 2.0, t.surface1, 1.0);
    let q = "?";
    let qw = text::measure(&ctx.fonts.ui, sz, q) as f64;
    text::draw_text(
        scene, &ctx.fonts.ui, sz, t.text_faint,
        help.center().x - qw / 2.0, baseline_for(cy, sz, &ctx.fonts.ui), q,
    );

    // Input box: flex:1, bg --base, 1px --surface1, radius 3, padding 3px 6px.
    // Ends short of the `?` button (8px gap).
    let input = Rect::new(
        icon_end + 6.0,
        r.y0 + REVSET_PAD_Y,
        help.x0 - 8.0,
        r.y1 - REVSET_PAD_Y,
    );
    fill_round(scene, input, 3.0, t.base);
    stroke_round(scene, input, 3.0, t.surface1, 1.0);
    // Placeholder (the default revset), color --surface1 per spec. `.revset-input`
    // is `font-family: inherit` → body `--font-ui` (Inter, sans) — NOT mono.
    // Clipped to the input box so the text can't bleed over the `?` button.
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &input);
    text::draw_text(
        scene, &ctx.fonts.ui, sz, t.surface2,
        input.x0 + 6.0 + REVSET_INPUT_PAD_Y, baseline_for(input.center().y, sz, &ctx.fonts.ui),
        "present(@) | ancestors(immutable_heads().., 2) | trunk()",
    );
    scene.pop_layer();
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

/// lightjj's `/logo.svg` mark: an amber lightning bolt traced as a stroked
/// polyline with round caps/joins, a fainter spark tail, and node dots — drawn
/// directly from the SVG's coordinates. The SVG authoring space is the 80×80
/// `viewBox`; we map it into a `box_sz`-px box whose left edge is at `x` and
/// vertically centered on `cy` (so it matches lightjj's 16px toolbar logo).
///
/// Reference (frontend/public/logo.svg):
/// ```text
/// <path d="M46,8 L30,34 L46,36 L32,56" stroke-width=3.5 round caps/joins/>
/// <path d="M46,36 L58,58" stroke-width=2.5 opacity=0.65/>
/// <circle 46,8 r4/> <circle 38,35 r2/> <circle 32,56 r3.5/>
/// <circle 58,58 r2.5 opacity=0.65/>
/// ```
fn draw_logo(scene: &mut Scene, x: f64, cy: f64, box_sz: f64, color: Color) {
    let s = box_sz / 80.0; // viewBox (80) → box scale
    // Map an SVG-space point into device space (left-aligned at `x`, centered cy).
    let p = |px: f64, py: f64| (x + px * s, cy + (py - 40.0) * s);
    let faint = color.multiply_alpha(0.65);

    let round = |w: f64| {
        Stroke::new(w * s)
            .with_caps(Cap::Round)
            .with_join(Join::Round)
    };

    // Main bolt: M46,8 L30,34 L46,36 L32,56
    let mut bolt = BezPath::new();
    bolt.move_to(p(46.0, 8.0));
    bolt.line_to(p(30.0, 34.0));
    bolt.line_to(p(46.0, 36.0));
    bolt.line_to(p(32.0, 56.0));
    scene.stroke(&round(3.5), Affine::IDENTITY, color, None, &bolt);

    // Spark tail: M46,36 L58,58 (thinner, fainter).
    let mut tail = BezPath::new();
    tail.move_to(p(46.0, 36.0));
    tail.line_to(p(58.0, 58.0));
    scene.stroke(&round(2.5), Affine::IDENTITY, faint, None, &tail);

    // Node dots.
    let dot = |scene: &mut Scene, cxp: f64, cyp: f64, r: f64, col: Color| {
        let (dx, dy) = p(cxp, cyp);
        let rr = r * s;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            col,
            None,
            &vello::kurbo::Circle::new((dx, dy), rr),
        );
    };
    dot(scene, 46.0, 8.0, 4.0, color);
    dot(scene, 38.0, 35.0, 2.0, color);
    dot(scene, 32.0, 56.0, 3.5, color);
    dot(scene, 58.0, 58.0, 2.5, faint);
}

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

/// The font a nav icon is drawn *with*. All toolbar icons (`◉ ⑂ ⧉ ⟲ ◐ ◇ …`) are
/// missing from Inter and JetBrains Mono, so `text::draw_text` resolves them
/// through the symbol fallback chain (Noto Sans Math / Symbols 2) regardless of
/// which font we name here — the named font is only the *primary* of that chain.
/// We pass the UI font (matching the label's weight) so a single `draw_text` of
/// `"icon word"` would resolve consistently; the icon and word are drawn
/// separately only so the icon can take its own metric-centered baseline.
fn icon_font<'a>(bold: bool, ctx: &'a RenderCtx) -> &'a vello::peniko::FontData {
    if bold { &ctx.fonts.ui_bold } else { &ctx.fonts.ui }
}

/// Gap between a nav icon glyph and its following word.
const ICON_LABEL_GAP: f64 = 3.0;

/// Width of an `icon word` run (icon resolved via the fallback chain, word in
/// the UI font).
fn icon_label_w(icon: &str, word: &str, sz: f32, bold: bool, ctx: &RenderCtx) -> f64 {
    let font = icon_font(bold, ctx);
    text::measure(font, sz, icon) as f64 + ICON_LABEL_GAP + text::measure(font, sz, word) as f64
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
    let font = icon_font(bold, ctx);
    // Icon: center its real ink box (resolved in whatever fallback font draws it)
    // on `cy` — symbol fonts' line metrics don't match Inter's, so the plain
    // `baseline_for` would leave the glyph high or low. The word keeps Inter's
    // line-box baseline so the Latin label still aligns with the rest of the bar.
    let iglyph = icon.chars().next().unwrap_or(' ');
    let ix = text::draw_text(scene, font, sz, color, x, text::icon_baseline(font, sz, iglyph, cy), icon)
        + ICON_LABEL_GAP;
    text::draw_text(scene, font, sz, color, ix, baseline_for(cy, sz, font), word)
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
    // ◇ ink-box centered (its symbol-font metrics differ from the mono line box).
    let gchar = glyph.chars().next().unwrap_or(' ');
    tx = text::draw_text(
        scene, &ctx.fonts.mono, theme::font::FS_XS, t.subtext0,
        tx, text::icon_baseline(&ctx.fonts.mono, theme::font::FS_XS, gchar, cy), glyph,
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
