// INTEGRATION: move to src/ui/merge.rs; dispatch from build_scene when active_view==View::Merge (full width, hide graph); replace MockConflict with a jjlib conflict query
//
//! Merge view (nav "3") — a native Rust/Vello reimplementation of lightjj's
//! 3-pane conflict editor (`MergePanel.svelte`) plus its left-rail
//! `ConflictQueue.svelte`. Self-contained: renders the full-width merge surface
//! (the revision graph is hidden in this view) from a small in-file mock of the
//! conflict data the jjlib backend will eventually supply.
//!
//! Layout (full width, `render`'s `rect`):
//!
//!   ┌──────────────┬──────────────────────────────────────────────────┐
//!   │ ConflictQueue │ toolbar (⧉ path · counter · nav · take-all/save) │
//!   │  rail (240px) ├──────────────────────────────────────────────────┤
//!   │  files grouped│ pane headers (⬅ ours | ✎ Result | theirs ➡)      │
//!   │  by commit    ├─────────┬───────┬─────────┬───────┬───────┬───────┤
//!   │  ○/● dots     │  ours   │ →gut. │ Result  │ ←gut. │theirs │minimap│
//!   │  footer N/M   │ (read)  │       │ (edit)  │       │(read) │       │
//!   └──────────────┴─────────┴───────┴─────────┴───────┴───────┴───────┘
//!
//! Conflict-side colors follow ui-spec §5.8 / MergePanel's CSS: the "ours" side
//! rail is GREEN (`--green`), the "theirs" side rail is BLUE (`--blue`), result
//! blocks tint to whichever side they currently carry, and per-side A/B framing
//! (the conflict-region rails) uses those colors at the block boundaries. The
//! ConflictQueue mirrors §5.8's group headers (change-id + description), ○/●
//! resolved dots, amber selection rail, N-way badge, and the N/M footer.
//!
//! Built against the PUBLIC paint helpers in `jjscratch::ui` so it stays a pure
//! consumer of the stable chrome contract (no edits to ui.rs / theme.rs).

use jjscratch::text;
use jjscratch::theme::{font, layout as L, Palette};
use jjscratch::ui::{
    self, baseline_for, border_bottom, fill_rect, fill_round, stroke_round, RenderCtx,
};

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill, FontData};
use vello::Scene;

// Autobins require a `main`; the real integration drops this in favor of
// dispatch from `ui::build_scene`. Harmless no-op shim.
#[allow(dead_code)]
fn main() {}

// ───────────────────────────── mock data ──────────────────────────────────
//
// Stand-in for the jjlib conflict query. At integration this is replaced by a
// `model`-level conflict type (conflicted files grouped by commit, each with
// reconstructed ours/base/theirs sides + per-block ranges). Shapes mirror
// lightjj's `ConflictEntry` (ConflictQueue) and `MergeSides` (MergePanel).

/// One conflicted file in the queue (a `ConflictQueue` row).
pub struct MockFile {
    /// Repo-relative path shown in the rail.
    pub path: &'static str,
    /// jj conflict side-count (`2` = ordinary 2-sided; `>2` shows an "N-way" badge).
    pub sides: u8,
    /// Resolved in this session → filled `●` dot, dimmed path.
    pub resolved: bool,
}

/// A commit that owns one or more conflicted files (a `ConflictQueue` group).
pub struct MockCommitGroup {
    pub change_id: &'static str,
    pub description: &'static str,
    pub files: Vec<MockFile>,
}

/// Which side a result line / block currently carries — drives its tint
/// (green/blue/amber) exactly like MergePanel's `merge-from-*` classes.
#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Ours,
    Theirs,
    Both,
    Mixed,
    /// Context line (outside any conflict block) — no tint.
    Context,
}

/// One line in a pane. `kind` selects the tint; `text` is the content.
pub struct PaneLine {
    pub text: &'static str,
    pub kind: Side,
}

/// A conflict block (one merge unit / arrow). Holds the 0-based result-pane
/// line range it spans so the gutter arrow + minimap chip can be placed.
pub struct MockBlock {
    /// First result line (0-based) of the block.
    pub result_from: usize,
    /// One past the last result line.
    pub result_to: usize,
    /// Current source of the block's result content.
    pub source: Side,
}

/// The full mock conflict set: the open file's three sides + blocks, plus the
/// queue of all conflicted files grouped by commit.
pub struct MockConflict {
    /// Path of the file open in the 3-pane editor (the selected queue row).
    pub open_path: &'static str,
    /// `oursRef` short change-id shown in the ours header.
    pub ours_ref: &'static str,
    /// `theirsRef` short change-id shown in the theirs header.
    pub theirs_ref: &'static str,
    pub ours: Vec<PaneLine>,
    pub result: Vec<PaneLine>,
    pub theirs: Vec<PaneLine>,
    pub blocks: Vec<MockBlock>,
    /// Index into the flattened queue of the selected row.
    pub selected: usize,
    pub groups: Vec<MockCommitGroup>,
}

/// Representative stub: three conflicted files across two commits; the open file
/// (`src/api.rs`) has two conflict blocks — one resolved to "ours", one still
/// pending on "theirs".
pub fn mock() -> MockConflict {
    use Side::*;
    // ours pane — read-only side #1.
    let ours = vec![
        PaneLine { text: "pub fn connect(addr: &str) -> Result<Conn> {", kind: Context },
        PaneLine { text: "    let cfg = Config::from_env()?;", kind: Context },
        PaneLine { text: "    let timeout = Duration::from_secs(30);", kind: Ours },
        PaneLine { text: "    let conn = Conn::dial(addr, cfg)?;", kind: Context },
        PaneLine { text: "    conn.set_timeout(timeout);", kind: Context },
        PaneLine { text: "    retry_backoff(conn, 5)", kind: Ours },
        PaneLine { text: "}", kind: Context },
        PaneLine { text: "", kind: Context },
        PaneLine { text: "fn retry_backoff(c: Conn, n: u32) -> Result<Conn> {", kind: Context },
        PaneLine { text: "    let base = Duration::from_millis(50);", kind: Ours },
        PaneLine { text: "    backoff::run(c, n, base)", kind: Ours },
        PaneLine { text: "}", kind: Context },
    ];
    // result pane — the editable center, seeded from theirs, block 0 taken ours.
    let result = vec![
        PaneLine { text: "pub fn connect(addr: &str) -> Result<Conn> {", kind: Context },
        PaneLine { text: "    let cfg = Config::from_env()?;", kind: Context },
        PaneLine { text: "    let timeout = Duration::from_secs(30);", kind: Ours },
        PaneLine { text: "    let conn = Conn::dial(addr, cfg)?;", kind: Context },
        PaneLine { text: "    conn.set_timeout(timeout);", kind: Context },
        PaneLine { text: "    conn.set_keepalive(true);", kind: Theirs },
        PaneLine { text: "    retry_backoff(conn, 3)", kind: Theirs },
        PaneLine { text: "}", kind: Context },
        PaneLine { text: "", kind: Context },
        PaneLine { text: "fn retry_backoff(c: Conn, n: u32) -> Result<Conn> {", kind: Context },
        // Block 2 — user took BOTH sides (ours + theirs concatenated).
        PaneLine { text: "    let base = Duration::from_millis(50);", kind: Both },
        PaneLine { text: "    let jitter = rng().gen_range(0..25);", kind: Both },
        // Block 3 — user hand-edited (mixed source).
        PaneLine { text: "    backoff::run(c, n, base, jitter) // tuned", kind: Mixed },
        PaneLine { text: "}", kind: Context },
    ];
    // theirs pane — read-only side #2.
    let theirs = vec![
        PaneLine { text: "pub fn connect(addr: &str) -> Result<Conn> {", kind: Context },
        PaneLine { text: "    let cfg = Config::from_env()?;", kind: Context },
        PaneLine { text: "    let timeout = Duration::from_secs(60);", kind: Theirs },
        PaneLine { text: "    let conn = Conn::dial(addr, cfg)?;", kind: Context },
        PaneLine { text: "    conn.set_timeout(timeout);", kind: Context },
        PaneLine { text: "    conn.set_keepalive(true);", kind: Theirs },
        PaneLine { text: "    retry_backoff(conn, 3)", kind: Theirs },
        PaneLine { text: "}", kind: Context },
        PaneLine { text: "", kind: Context },
        PaneLine { text: "fn retry_backoff(c: Conn, n: u32) -> Result<Conn> {", kind: Context },
        PaneLine { text: "    let jitter = rng().gen_range(0..25);", kind: Theirs },
        PaneLine { text: "    backoff::run(c, n, jitter)", kind: Theirs },
        PaneLine { text: "}", kind: Context },
    ];
    let blocks = vec![
        // Block 0 resolved to ours (line 2 in result).
        MockBlock { result_from: 2, result_to: 3, source: Ours },
        // Block 1 still on theirs (lines 5–6 in result) — pending.
        MockBlock { result_from: 5, result_to: 7, source: Theirs },
        // Block 2 — took both (lines 10–11 in result).
        MockBlock { result_from: 10, result_to: 12, source: Both },
        // Block 3 — hand-edited / mixed (line 12 in result).
        MockBlock { result_from: 12, result_to: 13, source: Mixed },
    ];
    let groups = vec![
        MockCommitGroup {
            change_id: "qpvuntsm",
            description: "feat: connection retry + keepalive",
            files: vec![
                MockFile { path: "src/api.rs", sides: 2, resolved: false },
                MockFile { path: "src/config.rs", sides: 2, resolved: true },
            ],
        },
        MockCommitGroup {
            change_id: "zzfkpwmx",
            description: "refactor: extract transport layer",
            files: vec![MockFile { path: "src/transport/mod.rs", sides: 3, resolved: false }],
        },
    ];
    MockConflict {
        open_path: "src/api.rs",
        ours_ref: "qpvuntsm",
        theirs_ref: "mlkpwrqz",
        ours,
        result,
        theirs,
        blocks,
        selected: 0,
        groups,
    }
}

// ───────────────────────────── geometry ───────────────────────────────────

/// ConflictQueue rail width (`.cq-root` min-width 220, ui-spec §3 left-rail).
const RAIL_W: f64 = 240.0;
/// Toolbar row height (`.merge-toolbar`, padding 5 + content ≈ 28).
const TOOLBAR_H: f64 = 34.0;
/// Pane-header row height (`.merge-header`, padding 4 + line).
const HEADERS_H: f64 = 26.0;
/// Inter-pane gutter width (`.merge-gutter`, MergePanel `GUTTER_W = 40`).
const GUTTER_W: f64 = 40.0;
/// Minimap strip width (`.merge-minimap`).
const MINIMAP_W: f64 = 12.0;
/// Row height for code/queue lines (fixed 18px, the load-bearing diff/graph row).
const ROW_H: f64 = L::DIFF_LINE_H;
/// Horizontal padding inside chrome rows.
const PAD_X: f64 = 10.0;
/// Code-line left padding inside a pane (after the side rail + line-num column).
const CODE_PAD_X: f64 = 14.0;
/// ConflictQueue footer height (`.cq-footer`).
const CQ_FOOTER_H: f64 = 26.0;

/// `color-mix(in srgb, color P%, transparent)` — straight-alpha tint, matching
/// lightjj's CSS mixes used throughout MergePanel/ConflictQueue.
fn mix_alpha(c: Color, pct: f32) -> Color {
    c.multiply_alpha(pct / 100.0)
}

/// `color-mix(in srgb, a P%, b)` — opaque blend of two colors (for the queue's
/// amber-tinted selection rail, take-all button borders, etc.).
fn mix(a: Color, b: Color, a_pct: f32) -> Color {
    let f = a_pct / 100.0;
    let [ar, ag, ab, _] = a.to_rgba8().to_u8_array();
    let [br, bg, bb, _] = b.to_rgba8().to_u8_array();
    let l = |x: u8, y: u8| ((x as f32) * f + (y as f32) * (1.0 - f)) as u8;
    Color::from_rgb8(l(ar, br), l(ag, bg), l(ab, bb))
}

// ─────────────────────────────── render ───────────────────────────────────

/// Draw the full merge surface into `rect` from the (mock) `conflicts`.
pub fn render(scene: &mut Scene, rect: Rect, conflicts: &MockConflict, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    // Left rail (ConflictQueue) | right column (toolbar + headers + panes).
    let rail = Rect::new(rect.x0, rect.y0, rect.x0 + RAIL_W, rect.y1);
    let main = Rect::new(rail.x1, rect.y0, rect.x1, rect.y1);

    conflict_queue(scene, rail, conflicts, ctx);

    let toolbar = Rect::new(main.x0, main.y0, main.x1, main.y0 + TOOLBAR_H);
    let headers = Rect::new(main.x0, toolbar.y1, main.x1, toolbar.y1 + HEADERS_H);
    let panes = Rect::new(main.x0, headers.y1, main.x1, main.y1);

    merge_toolbar(scene, toolbar, conflicts, ctx);
    pane_headers(scene, headers, conflicts, ctx);
    merge_panes(scene, panes, conflicts, ctx);

    scene.pop_layer();
}

// ──────────────────────────── ConflictQueue ───────────────────────────────

/// The left-rail conflict queue: files grouped by commit, ○/● resolved dots,
/// amber selection rail, N/M footer. Mirrors `ConflictQueue.svelte`.
fn conflict_queue(scene: &mut Scene, rect: Rect, c: &MockConflict, ctx: &RenderCtx) {
    let t = ctx.theme;
    // `.cq-root`: mantle bg, right border surface0.
    fill_rect(scene, rect, t.mantle);
    fill_rect(scene, Rect::new(rect.x1 - 1.0, rect.y0, rect.x1, rect.y1), t.surface0);

    let mut y = rect.y0;
    let mut flat = 0usize; // flat index across all files (for selection)
    let mut total = 0usize;
    let mut resolved = 0usize;

    for group in &c.groups {
        // `.cq-group` header: change-id (amber mono) + description (subtext0),
        // top border surface0 (none on the first group).
        let gh = Rect::new(rect.x0, y, rect.x1, y + ROW_H + 2.0);
        if flat > 0 {
            ui::border_top(scene, gh, t.surface0);
        }
        let gcy = gh.center().y;
        let idsz = font::FS_XS;
        let id = &group.change_id[..8.min(group.change_id.len())];
        let after = text::draw_text(
            scene, &ctx.fonts.mono, idsz, t.amber,
            rect.x0 + PAD_X, baseline_for(gcy, idsz, &ctx.fonts.mono), id,
        );
        let dsz = font::FS_SM;
        draw_clipped(
            scene, &ctx.fonts.ui, dsz, t.subtext0,
            after + 6.0, baseline_for(gcy, dsz, &ctx.fonts.ui), group.description,
            rect.x1 - PAD_X,
        );
        y = gh.y1;

        for f in &group.files {
            total += 1;
            if f.resolved {
                resolved += 1;
            }
            let selected = flat == c.selected;
            y = queue_row(scene, rect, y, f, selected, ctx);
            flat += 1;
        }
    }

    // `.cq-footer`: "N/M resolved", centered mono, top border surface0.
    let footer = Rect::new(rect.x0, rect.y1 - CQ_FOOTER_H, rect.x1, rect.y1);
    fill_rect(scene, footer, t.mantle);
    ui::border_top(scene, footer, t.surface0);
    let fsz = font::FS_XS;
    let ftxt = format!("{resolved}/{total} resolved");
    let fw = text::measure(&ctx.fonts.mono, fsz, &ftxt) as f64;
    text::draw_text(
        scene, &ctx.fonts.mono, fsz, t.subtext0,
        footer.center().x - fw / 2.0,
        baseline_for(footer.center().y, fsz, &ctx.fonts.mono), &ftxt,
    );
}

/// One `.cq-item`: ○/● dot + path + optional N-way badge; amber rail when
/// selected, green dot/dimmed path when resolved.
fn queue_row(scene: &mut Scene, rect: Rect, y: f64, f: &MockFile, selected: bool, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let r = Rect::new(rect.x0, y, rect.x1, y + ROW_H);
    let cy = r.center().y;

    if selected {
        // `.cq-selected`: amber 12% bg + 2px amber left rail.
        fill_rect(scene, r, mix_alpha(t.amber, 12.0));
        fill_rect(scene, Rect::new(r.x0, r.y0, r.x0 + 2.0, r.y1), t.amber);
    }

    // `.cq-dot`: ● green when resolved, ○ subtext1 otherwise.
    let dot = if f.resolved { "\u{25CF}" } else { "\u{25CB}" };
    let dot_color = if f.resolved { t.green } else { t.subtext1 };
    let dsz = font::FS_SM;
    let dot_x = r.x0 + 18.0;
    text::draw_text(
        scene, &ctx.fonts.ui, dsz, dot_color,
        dot_x, baseline_for(cy, dsz, &ctx.fonts.ui), dot,
    );

    // `.cq-path`: mono; dimmed (subtext0) when resolved.
    let psz = font::FS_SM;
    let path_color = if f.resolved { t.subtext0 } else { t.text };
    let path_x = dot_x + 16.0;

    // `.cq-nway` badge (right-anchored, red) when sides > 2.
    let mut clip_right = r.x1 - PAD_X;
    if f.sides > 2 {
        let nsz = font::FS_2XS;
        let label = format!("{}-way", f.sides);
        let nw = text::measure(&ctx.fonts.ui, nsz, &label) as f64;
        let badge = Rect::new(r.x1 - PAD_X - nw - 8.0, cy - 8.0, r.x1 - PAD_X, cy + 8.0);
        fill_round(scene, badge, 3.0, mix_alpha(t.red, 18.0));
        text::draw_text(
            scene, &ctx.fonts.ui, nsz, t.red,
            badge.x0 + 4.0, baseline_for(cy, nsz, &ctx.fonts.ui), &label,
        );
        clip_right = badge.x0 - 6.0;
    }

    draw_clipped(
        scene, &ctx.fonts.mono, psz, path_color,
        path_x, baseline_for(cy, psz, &ctx.fonts.mono), f.path, clip_right,
    );

    r.y1
}

// ───────────────────────────── merge toolbar ──────────────────────────────

/// `.merge-toolbar`: ⧉ path · resolved counter · ‹ N of M › nav · spacer ·
/// take-all-ours/theirs · pane-toggle · save · cancel. Mantle bg, bottom border.
fn merge_toolbar(scene: &mut Scene, rect: Rect, c: &MockConflict, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.mantle);
    border_bottom(scene, rect, t.surface0);
    let cy = rect.center().y;

    // `.merge-title`: ⧉ + path (mono, text color).
    let sz = font::FS_SM;
    let mut x = rect.x0 + PAD_X;
    x = text::draw_text(
        scene, &ctx.fonts.ui, font::FS_MD, t.subtext0,
        x, text::icon_baseline(&ctx.fonts.ui, font::FS_MD, '\u{29C9}', cy), "\u{29C9}",
    );
    x = text::draw_text(
        scene, &ctx.fonts.mono, sz, t.text,
        x + 6.0, baseline_for(cy, sz, &ctx.fonts.mono), c.open_path,
    );

    // `.merge-counter`: resolved/total blocks, amber pill (green when done).
    let total = c.blocks.len();
    let pending = c.blocks.iter().filter(|b| b.source == Side::Theirs).count();
    let done_n = total - pending;
    let done = pending == 0;
    let ctxt = format!("{done_n}/{total}");
    let cw = text::measure(&ctx.fonts.mono, font::FS_XS, &ctxt) as f64;
    let pill = Rect::new(x + 12.0, cy - 9.0, x + 12.0 + cw + 12.0, cy + 9.0);
    let (pill_bg, pill_fg) = if done {
        (mix_alpha(t.green, 18.0), t.green)
    } else {
        (mix_alpha(t.amber, 18.0), t.amber)
    };
    fill_round(scene, pill, 8.0, pill_bg);
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_XS, pill_fg,
        pill.x0 + 6.0, baseline_for(cy, font::FS_XS, &ctx.fonts.mono), &ctxt,
    );
    x = pill.x1;

    // `.merge-nav`: ‹ current of total › (mono, subtext0).
    let nav = format!("\u{2039}  1 of {total}  \u{203A}");
    text::draw_text(
        scene, &ctx.fonts.mono, font::FS_XS, t.subtext0,
        x + 12.0, baseline_for(cy, font::FS_XS, &ctx.fonts.mono), &nav,
    );

    // Right-anchored buttons (drawn right→left): [Cancel] [Save] [◫◫◫]
    // [All theirs ←←] [→→ All ours].
    let mut bx = rect.x1 - PAD_X;
    bx = btn(scene, bx, cy, "Cancel", t.subtext0, t.surface1, false, ctx);
    bx = btn(scene, bx - 6.0, cy, "Save", t.green, mix(t.green, t.surface1, 40.0), true, ctx);
    bx = btn(scene, bx - 6.0, cy, "\u{25EB}\u{25EB}\u{25EB}", t.subtext0, t.surface1, false, ctx);
    bx = btn(
        scene, bx - 6.0, cy, "All theirs \u{2190}\u{2190}",
        mix(t.blue, t.text, 70.0), mix(t.blue, t.surface1, 30.0), false, ctx,
    );
    btn(
        scene, bx - 6.0, cy, "\u{2192}\u{2192} All ours",
        mix(t.green, t.text, 70.0), mix(t.green, t.surface1, 30.0), false, ctx,
    );
}

/// A right-anchored `.btn`; returns the new (left) x cursor. `success` tints the
/// bg faintly (matching `.btn-success`).
fn btn(
    scene: &mut Scene,
    right_x: f64,
    cy: f64,
    label: &str,
    fg: Color,
    border: Color,
    success: bool,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let sz = font::FS_SM;
    let w = text::measure(&ctx.fonts.ui, sz, label) as f64 + 10.0 * 2.0;
    let r = Rect::new(right_x - w, cy - 11.0, right_x, cy + 11.0);
    if success {
        fill_round(scene, r, 4.0, mix_alpha(t.green, 12.0));
    }
    stroke_round(scene, r, 4.0, border, 1.0);
    text::draw_text(
        scene, &ctx.fonts.ui, sz, fg,
        r.x0 + 10.0, baseline_for(cy, sz, &ctx.fonts.ui), label,
    );
    r.x0
}

// ───────────────────────────── pane headers ───────────────────────────────

/// `.merge-headers`: three flex headers aligned to the pane columns — ⬅ ours
/// (green left rail), ✎ Result (center, bold), theirs ➡ (blue right rail).
/// Crust bg, bottom border surface0.
fn pane_headers(scene: &mut Scene, rect: Rect, c: &MockConflict, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.crust);
    border_bottom(scene, rect, t.surface0);
    let cy = rect.center().y;
    let cols = pane_columns(rect);
    let sz = font::FS_XS;

    // ours header — green 3px left rail + faint green wash + change-id + label.
    let o = cols.ours;
    fill_rect(scene, o, mix_alpha(t.green, 4.0));
    fill_rect(scene, Rect::new(o.x0, o.y0, o.x0 + 3.0, o.y1), t.green);
    let mut x = o.x0 + PAD_X;
    x = text::draw_text(scene, &ctx.fonts.ui, sz, t.subtext0, x, baseline_for(cy, sz, &ctx.fonts.ui), "\u{2B05} ");
    x = text::draw_text(
        scene, &ctx.fonts.mono, font::FS_2XS, t.amber,
        x, baseline_for(cy, font::FS_2XS, &ctx.fonts.mono), c.ours_ref,
    );
    draw_clipped(
        scene, &ctx.fonts.ui, sz, t.subtext0,
        x + 6.0, baseline_for(cy, sz, &ctx.fonts.ui), " · Ours (side #1)", o.x1 - PAD_X,
    );

    // center header — ✎ Result, bold text, side borders.
    let ctr = cols.result;
    fill_rect(scene, Rect::new(ctr.x0, ctr.y0, ctr.x0 + 1.0, ctr.y1), t.surface0);
    fill_rect(scene, Rect::new(ctr.x1 - 1.0, ctr.y0, ctr.x1, ctr.y1), t.surface0);
    text::draw_text(
        scene, &ctx.fonts.ui_bold, sz, t.text,
        ctr.x0 + PAD_X, baseline_for(cy, sz, &ctx.fonts.ui_bold), "\u{270E} Result",
    );

    // theirs header — blue 3px right rail + faint blue wash, right-aligned label.
    let th = cols.theirs;
    fill_rect(scene, th, mix_alpha(t.blue, 4.0));
    fill_rect(scene, Rect::new(th.x1 - 3.0, th.y0, th.x1, th.y1), t.blue);
    let label = format!("{} · Theirs (side #2) \u{27A1}", c.theirs_ref);
    let lw = text::measure(&ctx.fonts.ui, sz, &label) as f64;
    draw_clipped(
        scene, &ctx.fonts.ui, sz, t.subtext0,
        (th.x1 - 3.0 - PAD_X - lw).max(th.x0 + PAD_X),
        baseline_for(cy, sz, &ctx.fonts.ui), &label, th.x1 - 3.0 - PAD_X + 2.0,
    );
}

// ───────────────────────────── merge panes ────────────────────────────────

/// Column rects for the three panes + two gutters + minimap, laid out across
/// `rect`. The two flank panes and the result share the remaining width equally.
struct Columns {
    ours: Rect,
    gutter_ours: Rect,
    result: Rect,
    gutter_theirs: Rect,
    theirs: Rect,
    minimap: Rect,
}

fn pane_columns(rect: Rect) -> Columns {
    let inner_w = rect.width() - 2.0 * GUTTER_W - MINIMAP_W;
    let pane_w = (inner_w / 3.0).max(0.0);
    let mut x = rect.x0;
    let ours = Rect::new(x, rect.y0, x + pane_w, rect.y1);
    x = ours.x1;
    let gutter_ours = Rect::new(x, rect.y0, x + GUTTER_W, rect.y1);
    x = gutter_ours.x1;
    let result = Rect::new(x, rect.y0, x + pane_w, rect.y1);
    x = result.x1;
    let gutter_theirs = Rect::new(x, rect.y0, x + GUTTER_W, rect.y1);
    x = gutter_theirs.x1;
    let theirs = Rect::new(x, rect.y0, x + pane_w, rect.y1);
    x = theirs.x1;
    let minimap = Rect::new(x, rect.y0, rect.x1, rect.y1);
    Columns { ours, gutter_ours, result, gutter_theirs, theirs, minimap }
}

/// `.merge-panes`: the three CodeMirror panes + arrow gutters + minimap.
fn merge_panes(scene: &mut Scene, rect: Rect, c: &MockConflict, ctx: &RenderCtx) {
    let t = ctx.theme;
    let cols = pane_columns(rect);

    // Gutter backgrounds (faint directional wash drawn inside `arrow_gutter`) +
    // their dividers.
    fill_rect(scene, Rect::new(cols.gutter_ours.x1 - 1.0, rect.y0, cols.gutter_ours.x1, rect.y1), t.surface0);
    fill_rect(scene, Rect::new(cols.gutter_theirs.x0, rect.y0, cols.gutter_theirs.x0 + 1.0, rect.y1), t.surface0);
    // result pane side borders (`.merge-center` surface1 left/right).
    fill_rect(scene, Rect::new(cols.result.x0, rect.y0, cols.result.x0 + 1.0, rect.y1), t.surface1);
    fill_rect(scene, Rect::new(cols.result.x1 - 1.0, rect.y0, cols.result.x1, rect.y1), t.surface1);

    pane(scene, cols.ours, &c.ours, PaneRole::Ours, ctx);
    pane(scene, cols.result, &c.result, PaneRole::Result, ctx);
    pane(scene, cols.theirs, &c.theirs, PaneRole::Theirs, ctx);

    arrow_gutter(scene, cols.gutter_ours, c, Side::Ours, ctx);
    arrow_gutter(scene, cols.gutter_theirs, c, Side::Theirs, ctx);

    minimap(scene, cols.minimap, c, ctx);
}

#[derive(Clone, Copy, PartialEq)]
enum PaneRole {
    Ours,
    Result,
    Theirs,
}

/// Draw one code pane: line-number gutter + content lines with per-side tints
/// and rails (the §5.8 conflict framing). Read-only flank panes wash their
/// changed lines green/blue; the editable result tints by block source.
fn pane(scene: &mut Scene, rect: Rect, lines: &[PaneLine], role: PaneRole, ctx: &RenderCtx) {
    let t = ctx.theme;
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let num_w = 30.0; // line-number column
    let code_x = rect.x0 + num_w + CODE_PAD_X;
    let sz = font::FS_SM;

    for (i, line) in lines.iter().enumerate() {
        let y = rect.y0 + i as f64 * ROW_H;
        if y > rect.y1 {
            break;
        }
        let row = Rect::new(rect.x0, y, rect.x1, y + ROW_H);
        let cy = row.center().y;

        // Side tint + rail (ui-spec §5.8: ours green rail, theirs blue rail).
        // Flank panes color their own side's changed lines; the result colors
        // each line by its current source.
        let tint_side = match (role, line.kind) {
            (PaneRole::Ours, Side::Ours) => Some(Side::Ours),
            (PaneRole::Theirs, Side::Theirs) => Some(Side::Theirs),
            (PaneRole::Result, k) if k != Side::Context => Some(k),
            _ => None,
        };
        if let Some(s) = tint_side {
            let (bg, rail) = side_colors(t, s);
            fill_rect(scene, row, bg);
            // Rail side: ours/result-ours on the left, theirs on the right.
            let on_left = !matches!(s, Side::Theirs);
            if on_left {
                fill_rect(scene, Rect::new(row.x0, row.y0, row.x0 + 3.0, row.y1), rail);
            } else {
                fill_rect(scene, Rect::new(row.x1 - 3.0, row.y0, row.x1, row.y1), rail);
            }
        }

        // Line number (faint, right-aligned in its column).
        let num = format!("{}", i + 1);
        let nw = text::measure(&ctx.fonts.mono, font::FS_XS, &num) as f64;
        text::draw_text(
            scene, &ctx.fonts.mono, font::FS_XS, t.text_faint,
            rect.x0 + num_w - 6.0 - nw, baseline_for(cy, font::FS_XS, &ctx.fonts.mono), &num,
        );

        // Code text.
        draw_clipped(
            scene, &ctx.fonts.mono, sz, t.text,
            code_x, baseline_for(cy, sz, &ctx.fonts.mono), line.text, rect.x1 - 4.0,
        );
    }

    scene.pop_layer();
}

/// Per-side (bg tint, rail color) for §5.8 framing: ours = green, theirs = blue,
/// both = green-ish, mixed = amber.
fn side_colors(t: &Palette, s: Side) -> (Color, Color) {
    match s {
        Side::Ours => (mix_alpha(t.green, 12.0), mix_alpha(t.green, 45.0)),
        Side::Theirs => (mix_alpha(t.blue, 12.0), mix_alpha(t.blue, 45.0)),
        Side::Both => (mix_alpha(t.green, 10.0), mix_alpha(t.green, 40.0)),
        Side::Mixed => (mix_alpha(t.amber, 10.0), mix_alpha(t.amber, 40.0)),
        Side::Context => (t.base, t.base),
    }
}

/// Inter-pane arrow gutter: one arrow chip per conflict block, vertically
/// aligned to the block's result rows. Ours gutter draws `→` (green), theirs
/// gutter draws `←` (blue); an "applied" block dims its arrow.
fn arrow_gutter(scene: &mut Scene, rect: Rect, c: &MockConflict, gutter_side: Side, ctx: &RenderCtx) {
    let t = ctx.theme;
    // Faint directional wash over the gutter (`.merge-gutter-{ours,theirs}`).
    let wash = match gutter_side {
        Side::Ours => mix_alpha(t.green, 5.0),
        _ => mix_alpha(t.blue, 5.0),
    };
    fill_rect(scene, rect, wash);

    // Result-pane rows start at `rect.y0` (panes share the same top).
    for b in &c.blocks {
        let block_cy = rect.y0 + (b.result_from as f64 + b.result_to as f64) / 2.0 * ROW_H;
        if block_cy < rect.y0 || block_cy > rect.y1 {
            continue;
        }
        let applied = b.source == gutter_side;
        let (glyph, base_color) = match gutter_side {
            Side::Ours => ("\u{2192}", t.green),
            _ => ("\u{2190}", t.blue),
        };
        // Chip: 18px rounded square, centered in the 40px gutter.
        let chip = Rect::new(rect.x0 + 11.0, block_cy - 9.0, rect.x0 + 11.0 + 18.0, block_cy + 9.0);
        let (chip_bg, chip_fg) = if applied {
            // `.merge-arrow-applied`: muted surface0 / subtext0.
            (t.surface0, t.subtext0)
        } else {
            (mix(base_color, t.surface0, 25.0), base_color)
        };
        fill_round(scene, chip, 3.0, chip_bg);
        let asz = font::FS_MD;
        let aw = text::measure(&ctx.fonts.ui_bold, asz, glyph) as f64;
        text::draw_text(
            scene, &ctx.fonts.ui_bold, asz, chip_fg,
            chip.center().x - aw / 2.0,
            baseline_for(chip.center().y, asz, &ctx.fonts.ui_bold), glyph,
        );
    }
}

/// `.merge-minimap`: proportional chips showing where each block sits, colored
/// by current source (green/blue/amber), crust bg + left border.
fn minimap(scene: &mut Scene, rect: Rect, c: &MockConflict, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.crust);
    fill_rect(scene, Rect::new(rect.x0, rect.y0, rect.x0 + 1.0, rect.y1), t.surface0);

    let total = c.result.len().max(1) as f64;
    for b in &c.blocks {
        let top = rect.y0 + (b.result_from as f64 / total) * rect.height();
        let h = ((b.result_to - b.result_from) as f64 / total * rect.height()).max(3.0);
        let chip = Rect::new(rect.x0 + 2.0, top, rect.x1 - 2.0, top + h);
        let color = match b.source {
            Side::Ours => t.green,
            Side::Theirs => t.blue,
            Side::Mixed => t.amber,
            _ => t.green,
        };
        fill_round(scene, chip, 2.0, mix_alpha(color, 50.0));
    }
}

// ───────────────────────────── helpers ────────────────────────────────────

/// Draw text clipped to `[ ., right]` so labels never bleed past their column.
fn draw_clipped(
    scene: &mut Scene,
    font: &FontData,
    size: f32,
    color: Color,
    x: f64,
    baseline: f64,
    s: &str,
    right: f64,
) {
    if right <= x {
        return;
    }
    let clip = Rect::new(x - 1.0, baseline - size as f64 * 1.3, right, baseline + size as f64 * 0.6);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &clip);
    text::draw_text(scene, font, size, color, x, baseline, s);
    scene.pop_layer();
}
