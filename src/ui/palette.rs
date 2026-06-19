//! Command palette (Cmd+K) overlay renderer.
//!
//! Draws lightjj's command palette (frontend/src/lib/CommandPalette.svelte): a
//! dimmed backdrop over the whole viewport, a centered modal (the palette's own
//! `.palette` CSS — width 580, radius 14, 1px surface1 border, no visible
//! drop-shadow ring), a search input row showing the live `query`, and a
//! fuzzy-filtered list of representative commands with optional kbd shortcuts /
//! submenu chevrons, the active row highlighted by `--bg-selected` (amber tint).
//! There is NO key-footer in the filtered (non-cheatsheet) list state — the
//! reference shows only the input row + the result rows.
//!
//! It paints only with the helpers from `super` so it composes over a real
//! `build_scene` frame as an overlay. `ui::build_scene` dispatches here when
//! `palette_open` is set, passing the live `palette_query`.

use crate::theme::{font, Palette};
use super::{baseline_for, fill_rect, fill_round, stroke_round, RenderCtx};
use crate::text;
use vello::kurbo::Rect;
use vello::Scene;

/// One representative palette command. Mirrors the fields lightjj's
/// `PaletteCommand` actually renders (label + shortcut + submenu chevron).
struct Command {
    /// Visible command label, e.g. "New workspace…".
    label: &'static str,
    /// Right-aligned kbd shortcut hint (e.g. "⌘K", "G P"), or "" for none.
    shortcut: &'static str,
    /// Drills into a submenu (renders a `›` chevron after the label), like
    /// lightjj's `children` (theme picker, aliases).
    submenu: bool,
}

/// The hardcoded command set, matching the commands lightjj's palette exposes
/// (App.svelte's palette commands: theme toggle/picker, new workspace, edit
/// config, git push/fetch, oplog/evolog, refresh, …). Order is the cheatsheet
/// order; the fuzzy filter narrows it live.
const COMMANDS: &[Command] = &[
    Command { label: "Toggle theme", shortcut: "T", submenu: false },
    Command { label: "Pick theme…", shortcut: "", submenu: true },
    Command { label: "New workspace…", shortcut: "", submenu: false },
    Command { label: "Edit config", shortcut: "", submenu: false },
    Command { label: "Git push", shortcut: "G P", submenu: false },
    Command { label: "Git fetch", shortcut: "G F", submenu: false },
    Command { label: "Toggle oplog", shortcut: "4", submenu: false },
    Command { label: "Toggle evolog", shortcut: "5", submenu: false },
    Command { label: "Refresh", shortcut: "R", submenu: false },
    Command { label: "Describe revision…", shortcut: "D", submenu: false },
    Command { label: "Abandon revision", shortcut: "", submenu: false },
    Command { label: "New change", shortcut: "N", submenu: false },
];

/// Tiny subsequence fuzzy match (lightjj's `fuzzyMatch`): every char of
/// `query` must appear in `haystack` in order, case-insensitively. Empty query
/// matches everything. This is the same predicate `filteredCommands` uses.
fn fuzzy_match(query: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars().flat_map(char::to_lowercase);
    for qc in query.chars().flat_map(char::to_lowercase) {
        // Advance the haystack iterator until we find this query char (subsequence).
        if !hay.any(|hc| hc == qc) {
            return false;
        }
    }
    true
}

// Modal geometry constants (lightjj's `.palette` CSS, matched to the reference
// capture docs/reference/palette.png @2x: modal x [349..929] logical = w 580,
// centered; top border at y=160 (20%); input row 160..209 (~49); rows ~36 tall).
const MODAL_W: f64 = 580.0; // `.palette { width: 580px }`
const MODAL_TOP_FRAC: f64 = 0.20; // `top: 20%`
const MODAL_RADIUS: f64 = 14.0; // `.palette { border-radius: 14px }`
const PAD_X: f64 = 16.0; // `.palette-input-wrap` / `.palette-item` horizontal padding 16
const INPUT_H: f64 = 44.0; // `.palette-input-wrap`: padding 12*2 + fs-lg line (ref: top border→divider 88px@2x)
const LIST_PAD_Y: f64 = 4.0; // `.palette-results { padding: 4px 0 }` — gap above/below the rows
const ROW_H: f64 = 36.0; // `.palette-item`: padding 8*2 + fs line (ref: ~72px@2x for two rows)

/// Render the command palette overlay over `viewport`. `query` is the live
/// search string (drives the input row + fuzzy filter); pass "" for the
/// empty-query state. The first matching command is drawn as the active row,
/// matching the palette's `cursor.index = 0` reset on every input change.
pub fn render(scene: &mut Scene, viewport: Rect, query: &str, ctx: &RenderCtx) {
    let t = ctx.theme;

    // (a) Dimmed backdrop over the whole viewport (`--backdrop` rgba(0,0,0,0.5)).
    fill_rect(scene, viewport, t.backdrop);

    // Filter the command set by the fuzzy query.
    let matches: Vec<&Command> = COMMANDS
        .iter()
        .filter(|c| fuzzy_match(query, c.label))
        .collect();

    // (b) Modal box geometry: centered horizontally, top at 20% of the viewport.
    let modal_x0 = (viewport.x0 + viewport.x1 - MODAL_W) / 2.0;
    let modal_x1 = modal_x0 + MODAL_W;
    let modal_y0 = viewport.y0 + (viewport.y1 - viewport.y0) * MODAL_TOP_FRAC;
    // Height = input row + `.palette-results` (4px top/bottom padding + one row
    // per match, or one "empty" row). The filtered list has NO footer bar.
    let n_rows = matches.len().max(1) as f64;
    let modal_h = INPUT_H + LIST_PAD_Y * 2.0 + ROW_H * n_rows;
    let modal = Rect::new(modal_x0, modal_y0, modal_x1, modal_y0 + modal_h);

    draw_modal_chrome(scene, modal, t);
    draw_input_row(scene, modal, query, ctx);

    // (d) Command list, inset by `.palette-results` 4px top padding.
    let list_top = modal.y0 + INPUT_H + LIST_PAD_Y;
    if matches.is_empty() {
        draw_empty_row(scene, modal, list_top, ctx);
    } else {
        for (i, cmd) in matches.iter().enumerate() {
            let row = Rect::new(
                modal.x0,
                list_top + ROW_H * i as f64,
                modal.x1,
                list_top + ROW_H * (i as f64 + 1.0),
            );
            // (e) active highlight on the first match (cursor.index == 0).
            draw_command_row(scene, row, cmd, i == 0, ctx);
        }
    }
}

/// (b) The modal box: base bg, 1px surface1 border, 14px radius. lightjj's
/// `--shadow-heavy` + `backdrop-filter: blur(4px)` is a soft, wide blur that is
/// invisible right at the border (the reference shows the dimmed backdrop
/// meeting the surface1 border with no crisp ring) — so we draw no shadow lip,
/// just the border, matching the captured edge exactly.
fn draw_modal_chrome(scene: &mut Scene, modal: Rect, t: &Palette) {
    // Modal background (`--base`).
    fill_round(scene, modal, MODAL_RADIUS, t.base);
    // 1px `--surface1` border.
    stroke_round(scene, modal, MODAL_RADIUS, t.surface1, 1.0);
}

/// (c) Input row: `.palette-input-wrap` — mantle bg, an amber `▸` arrow, the
/// `query` text (or a dimmed placeholder), and an `esc` kbd chip on the right.
/// A bottom border (`--surface0`) separates it from the list.
fn draw_input_row(scene: &mut Scene, modal: Rect, query: &str, ctx: &RenderCtx) {
    let t = ctx.theme;
    let row = Rect::new(modal.x0, modal.y0, modal.x1, modal.y0 + INPUT_H);
    let cy = row.center().y;

    // mantle bg + bottom divider.
    fill_rect(scene, Rect::new(row.x0, row.y0, row.x1, row.y1 - 1.0), t.mantle);
    fill_rect(scene, Rect::new(row.x0, row.y1 - 1.0, row.x1, row.y1), t.surface0);

    let mut x = row.x0 + PAD_X;

    // amber `▸` arrow (`.palette-arrow`).
    let arrow_base = baseline_for(cy, font::FS_LG, &ctx.fonts.ui);
    x = text::draw_text(scene, &ctx.fonts.ui, font::FS_LG, t.amber, x, arrow_base, "\u{25b8}");
    x += 8.0; // `gap: 8px`

    // Query text or placeholder. lightjj uses the larger `--fs-lg` for the input.
    let input_base = baseline_for(cy, font::FS_LG, &ctx.fonts.ui);
    if query.is_empty() {
        text::draw_text(
            scene,
            &ctx.fonts.ui,
            font::FS_LG,
            t.text_faint,
            x,
            input_base,
            "Type a command...",
        );
    } else {
        let end = text::draw_text(scene, &ctx.fonts.ui, font::FS_LG, t.text, x, input_base, query);
        // A thin amber caret after the typed query (the input always holds focus).
        let caret_h = font::FS_LG as f64;
        fill_rect(
            scene,
            Rect::new(end + 1.0, cy - caret_h / 2.0, end + 2.5, cy + caret_h / 2.0),
            t.amber,
        );
    }

    // `esc` kbd chip, right-aligned.
    draw_kbd_right(scene, row.x1 - PAD_X, cy, "esc", true, ctx);
}

/// (d) One command row: active highlight, label, optional submenu chevron, and
/// a right-aligned shortcut kbd chip.
fn draw_command_row(scene: &mut Scene, row: Rect, cmd: &Command, active: bool, ctx: &RenderCtx) {
    let t = ctx.theme;
    let cy = row.center().y;

    if active {
        // `.palette-item-active { background: var(--bg-selected) }` — a flat
        // amber-tinted fill spanning the full row width. lightjj draws NO inset
        // amber bar here (the reference shows the bg meeting the surface1 border
        // directly), so neither do we.
        fill_rect(scene, row, t.bg_selected);
    }

    // Label (`.palette-label`), full-strength `--text`.
    let base = baseline_for(cy, font::BASE, &ctx.fonts.ui);
    let mut x = row.x0 + PAD_X;
    x = text::draw_text(scene, &ctx.fonts.ui, font::BASE, t.text, x, base, cmd.label);

    // Submenu chevron `›` in `--overlay0` (`.palette-submenu-arrow`).
    if cmd.submenu {
        text::draw_text(scene, &ctx.fonts.ui, font::BASE, t.overlay0, x + 8.0, base, "\u{203a}");
    }

    // Right-aligned shortcut kbd chip.
    if !cmd.shortcut.is_empty() {
        draw_kbd_right(scene, row.x1 - PAD_X, cy, cmd.shortcut, active, ctx);
    }
}

/// The "No matching commands" empty state (`.palette-empty`).
fn draw_empty_row(scene: &mut Scene, modal: Rect, top: f64, ctx: &RenderCtx) {
    let t = ctx.theme;
    let row = Rect::new(modal.x0, top, modal.x1, top + ROW_H);
    let cy = row.center().y;
    let msg = "No matching commands";
    let w = text::measure(&ctx.fonts.ui, font::BASE, msg) as f64;
    let base = baseline_for(cy, font::BASE, &ctx.fonts.ui);
    text::draw_text(
        scene,
        &ctx.fonts.ui,
        font::BASE,
        t.text_faint,
        row.center().x - w / 2.0,
        base,
        msg,
    );
}

/// Draw a kbd chip whose RIGHT edge sits at `right_x`, vertically centered on
/// `cy`. `(.palette-shortcut / .palette-esc)`: surface0 bg, surface1 border,
/// `--subtext0` text. `active` lifts the bg to `--surface1` (the active-row
/// shortcut treatment, `.palette-item-active .palette-shortcut`).
fn draw_kbd_right(scene: &mut Scene, right_x: f64, cy: f64, label: &str, active: bool, ctx: &RenderCtx) {
    let t = ctx.theme;
    let tw = text::measure(&ctx.fonts.mono, font::FS_SM, label) as f64;
    let pad = 6.0;
    let chip_w = tw + pad * 2.0;
    let chip_h = font::FS_SM as f64 + 6.0;
    let chip = Rect::new(right_x - chip_w, cy - chip_h / 2.0, right_x, cy + chip_h / 2.0);
    let bg = if active { t.surface1 } else { t.surface0 };
    fill_round(scene, chip, 3.0, bg);
    stroke_round(scene, chip, 3.0, t.surface1, 1.0);
    let base = baseline_for(cy, font::FS_SM, &ctx.fonts.mono);
    text::draw_text(scene, &ctx.fonts.mono, font::FS_SM, t.subtext0, chip.x0 + pad, base, label);
}
