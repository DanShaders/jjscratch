//! Revision graph renderer (the left "REVISIONS" panel).
//!
//! Reproduces lightjj's GraphSvg + flattened-row model (docs/spec/ui-spec.md §4):
//! every revision expands into a stack of fixed-18px rows — a node line (node
//! glyph + role/alert badges + description), an optional bookmark line, and a
//! meta line (change-id with amber prefix + faint tail, faint commit-id, author
//! chip, relative timestamp). The lane gutter is drawn from
//! [`crate::graph_layout`] connector cells so the pipes stay continuous across a
//! revision's sub-rows. The `(elided revisions)` marker is rendered for elided
//! (Indirect) parent edges.

use vello::kurbo::{Affine, BezPath, Cap, Circle, Line, Point, Rect, Stroke};
use vello::peniko::{Color, Fill};
use vello::Scene;

use super::{baseline_for, fill_rect, fill_round, RenderCtx, UiState};
use crate::graph_layout::{self, Cell, LayoutRow};
use crate::model::{BookmarkKind, CommitNode, EdgeType, Snapshot};
use crate::text;
use crate::theme::{self, font, layout as L, Palette};

/// Fixed "now" for relative-time rendering: 2026-06-19 ≈ 1781913600000 ms.
const NOW_MS: i64 = 1_781_913_600_000;

/// Horizontal gap (px) between the gutter SVG and the row content column. lightjj's
/// `.graph-row` reserves the SVG box (gutterWidth columns) then a flex gap before the
/// node/meta/bookmark content; matched empirically to the reference's content x0.
const CONTENT_GAP: f64 = 20.0;

/// Inline-flex gap (px) between badges/segments on a content line (`gap:6` node line,
/// `gap:4` bookmark line, `gap:8` meta line — §4.2/§4.3/§4.4).
const NODE_LINE_GAP: f64 = 6.0;
const BOOKMARK_GAP: f64 = 4.0;
const META_GAP: f64 = 8.0;

/// Top padding (px) of the graph row list below the panel header — matches the
/// reference's first-row offset from the "REVISIONS" header.
const LIST_TOP_PAD: f64 = 2.0;

/// One flattened display row tied to a revision.
enum LineKind {
    Node,
    Bookmark,
    Meta,
    /// The `(elided revisions)` marker drawn after a revision with an elided edge.
    Elided,
}

pub fn render(scene: &mut Scene, rect: Rect, snapshot: &Snapshot, state: &UiState, ctx: &RenderCtx) {
    let t = ctx.theme;
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let g = graph_layout::layout(&snapshot.nodes);

    // Gutter geometry: the SVG starts after the 14px check-gutter; each lane is
    // 20px wide (col*10), node center x = col*10 + 5 within the SVG box.
    let gutter_x0 = rect.x0 + L::CHECK_GUTTER_W;
    let gutter_cols = g.width_cols.clamp(1, L::MAX_GUTTER_COLS as usize);
    let gutter_w = gutter_cols as f64 * L::CELL_W;
    // lightjj leaves a comfortable gap between the gutter SVG and the row content.
    let content_x = gutter_x0 + gutter_w + CONTENT_GAP;

    // Flatten: per revision, a node line, optional bookmark line, meta line; an
    // elided marker row follows a revision whose first parent edge is Indirect.
    let mut flat: Vec<(usize, LineKind)> = Vec::new();
    for (i, n) in snapshot.nodes.iter().enumerate() {
        flat.push((i, LineKind::Node));
        if !n.bookmarks.is_empty() {
            flat.push((i, LineKind::Bookmark));
        }
        flat.push((i, LineKind::Meta));
        let elided = n
            .parents
            .iter()
            .find(|p| p.edge_type != EdgeType::Missing)
            .map(|p| p.edge_type == EdgeType::Indirect)
            .unwrap_or(false);
        if elided {
            flat.push((i, LineKind::Elided));
        }
    }

    // Viewport windowing: every flattened row is a fixed L::ROW_H tall, so the
    // first on-screen row index is exact arithmetic. We jump straight to it
    // instead of iterating-and-skipping every off-screen row above the viewport,
    // and we still stop at the first row past the bottom. The visible rows are
    // drawn byte-identically — only the leading off-screen draw work (glyphs,
    // text measurement, gutter cells) is elided. This makes the draw loop
    // O(visible rows), not O(history).
    let top0 = rect.y0 + LIST_TOP_PAD - state.graph_scroll;
    let first_visible = first_visible_row(top0, rect.y0, flat.len());

    let mut y = top0 + first_visible as f64 * L::ROW_H;
    for (node_idx, line) in flat.iter().skip(first_visible) {
        let row = Rect::new(rect.x0, y, rect.x1, y + L::ROW_H);
        if row.y0 > rect.y1 {
            break;
        }

        let n = &snapshot.nodes[*node_idx];
        let lrow = &g.rows[*node_idx];
        let lane = lrow.lane;
        let lane_c = lane_color(&t.graph, lane);
        let selected = *node_idx == state.selected;
        let hovered = state.hovered == Some(*node_idx) && !selected;

        if selected {
            fill_rect(scene, row, t.bg_selected);
        } else if hovered {
            fill_rect(scene, row, t.bg_hover);
        }

        // Gutter pipes for this sub-row. lightjj draws each revision's connector
        // SVG once across all its sub-rows, with elbows/horizontals only where the
        // node sits; the bookmark/meta/elided continuation rows below it just carry
        // the through-lane verticals. We mirror that: the node sub-row gets the full
        // connector cells, continuation rows get only the lanes that keep going down.
        if matches!(line, LineKind::Node) {
            draw_cells(scene, gutter_x0, y, &lrow.cells, &t.graph);
        } else {
            draw_cells(scene, gutter_x0, y, &continuation_cells(&lrow.cells), &t.graph);
        }

        // Node glyph on the node line only.
        if matches!(line, LineKind::Node) {
            let cx = node_center_x(gutter_x0, lrow);
            let cy = y + L::ROW_H * 0.5;
            draw_node(scene, cx, cy, n, t, lane_c);
        }

        // 2px amber inset bar for the selected revision (over the bg).
        if selected {
            fill_rect(scene, Rect::new(row.x0, row.y0, row.x0 + 2.0, row.y1), t.amber);
        }

        let cy = y + L::ROW_H * 0.5;
        match line {
            LineKind::Node => draw_node_line(scene, content_x, cy, n, t, ctx),
            LineKind::Bookmark => draw_bookmark_line(scene, content_x, cy, n, t, ctx, lane_c),
            LineKind::Meta => draw_meta_line(scene, content_x, cy, n, t, ctx),
            LineKind::Elided => draw_elided(scene, content_x, rect.x1, y, t, ctx),
        }

        y += L::ROW_H;
    }

    scene.pop_layer();
}

/// Index of the first flattened row that can be (partly) on-screen, given the
/// y of row 0's top (`top0`), the viewport's top edge (`view_top`), and the
/// total row count. Fixed-height rows make this exact arithmetic, so the draw
/// loop can jump straight to it instead of walking every off-screen row above.
/// The result is clamped to `[0, row_count]`.
fn first_visible_row(top0: f64, view_top: f64, row_count: usize) -> usize {
    if top0 >= view_top {
        0
    } else {
        // Largest index whose row top is still ≤ view_top, i.e. the row that the
        // viewport's top edge falls within. `floor` keeps that row (its lower part
        // is visible); rows strictly above it are fully off-screen.
        (((view_top - top0) / L::ROW_H).floor() as usize).min(row_count)
    }
}

fn lane_color(graph: &[Color; 8], lane: usize) -> Color {
    graph[lane % 8]
}

fn node_center_x(gx0: f64, lrow: &LayoutRow) -> f64 {
    gx0 + lrow.node_col() as f64 * L::CELL_W + L::CELL_W * 0.5
}

// --- gutter ----------------------------------------------------------------

/// Connector cells for a revision's *continuation* sub-rows (bookmark/meta/elided),
/// below the node row. Only the lanes that exit the bottom of the node row keep
/// going: through verticals stay vertical, a fork's top-elbow becomes its newly
/// opened vertical, an elided lane stays dashed; converging elbows and horizontal
/// stubs (which terminate at the node row) are dropped.
fn continuation_cells(cells: &[Cell]) -> Vec<Cell> {
    cells
        .iter()
        .map(|c| match c {
            Cell::Vertical => Cell::Vertical,
            Cell::Elided => Cell::Elided,
            // Fork elbows open a new lane that descends from this row.
            Cell::ElbowTopLeft | Cell::ElbowTopRight => Cell::Vertical,
            // Converging elbows / horizontal stubs terminate at the node row.
            _ => Cell::Empty,
        })
        .collect()
}

fn draw_cells(scene: &mut Scene, gx0: f64, y: f64, cells: &[Cell], graph: &[Color; 8]) {
    for (col, cell) in cells.iter().enumerate() {
        // Lane color is keyed by the lane that the column belongs to (col/2).
        let lane = col / 2;
        draw_cell(scene, gx0, y, col, *cell, lane_color(graph, lane));
    }
}

/// Draw one gutter cell. `col` is the character column, row top at `y`.
fn draw_cell(scene: &mut Scene, gx0: f64, y: f64, col: usize, cell: Cell, color: Color) {
    let cx = gx0 + col as f64 * L::CELL_W + L::CELL_W * 0.5;
    let cy = y + L::ROW_H * 0.5;
    let line_color = theme::with_opacity(color, theme::GRAPH_LINE_OPACITY);
    let w = L::GRAPH_LINE_W;

    match cell {
        Cell::Empty => {}
        Cell::Vertical => crisp_vline(scene, cx, y, y + L::ROW_H, line_color, w),
        Cell::Elided => {
            // Dashed vertical at lower opacity.
            let c = theme::with_opacity(color, 0.3);
            let stroke = Stroke::new(w).with_dashes(0.0, [2.0, 3.0]);
            scene.stroke(&stroke, Affine::IDENTITY, c, None, &Line::new((cx, y), (cx, y + L::ROW_H)));
        }
        Cell::Horizontal => {
            crisp_hline(scene, cx - L::CELL_W * 0.5, cx + L::CELL_W * 0.5, cy, line_color, w)
        }
        Cell::ElbowTopLeft
        | Cell::ElbowTopRight
        | Cell::ElbowBottomLeft
        | Cell::ElbowBottomRight => draw_elbow(scene, cx, y, cell, line_color, w),
    }
}

/// Quadratic-curve elbow through (cx, cy), round cap.
fn draw_elbow(scene: &mut Scene, cx: f64, y: f64, cell: Cell, color: Color, w: f64) {
    let cy = y + L::ROW_H * 0.5;
    let top = y;
    let bot = y + L::ROW_H;
    let left = cx - L::CELL_W * 0.5;
    let right = cx + L::CELL_W * 0.5;

    let (start, end) = match cell {
        Cell::ElbowTopLeft => (Point::new(cx, bot), Point::new(left, cy)), // ╮
        Cell::ElbowTopRight => (Point::new(cx, bot), Point::new(right, cy)), // ╭
        Cell::ElbowBottomLeft => (Point::new(cx, top), Point::new(left, cy)), // ╯
        Cell::ElbowBottomRight => (Point::new(cx, top), Point::new(right, cy)), // ╰
        _ => return,
    };
    let mut path = BezPath::new();
    path.move_to(start);
    path.quad_to(Point::new(cx, cy), end);
    scene.stroke(&Stroke::new(w).with_caps(Cap::Round), Affine::IDENTITY, color, None, &path);
}

/// Crisp vertical line (snap x to integer center, `crispEdges` semantics).
fn crisp_vline(scene: &mut Scene, x: f64, y0: f64, y1: f64, color: Color, w: f64) {
    let x = x.round();
    fill_rect(scene, Rect::new(x - w * 0.5, y0, x + w * 0.5, y1), color);
}

fn crisp_hline(scene: &mut Scene, x0: f64, x1: f64, y: f64, color: Color, w: f64) {
    let y = y.round();
    fill_rect(scene, Rect::new(x0, y - w * 0.5, x1, y + w * 0.5), color);
}

// --- node glyphs -----------------------------------------------------------

fn draw_node(scene: &mut Scene, cx: f64, cy: f64, n: &CommitNode, t: &Palette, lane: Color) {
    // Two short lane segments above/below the glyph (NODE_GAP gap), at line opacity.
    let line_c = theme::with_opacity(lane, theme::GRAPH_LINE_OPACITY);
    let top = cy - L::ROW_H * 0.5;
    let bot = cy + L::ROW_H * 0.5;
    crisp_vline(scene, cx, top, cy - L::NODE_GAP, line_c, L::GRAPH_LINE_W);
    crisp_vline(scene, cx, cy + L::NODE_GAP, bot, line_c, L::GRAPH_LINE_W);

    let node_c = theme::with_opacity(lane, theme::GRAPH_NODE_OPACITY);

    // Divergent: extra dashed ring around any node.
    if n.is_divergent {
        let ring = theme::with_opacity(lane, 0.5);
        let stroke = Stroke::new(1.0).with_dashes(0.0, [2.0, 2.0]);
        scene.stroke(&stroke, Affine::IDENTITY, ring, None, &Circle::new((cx, cy), L::NODE_R + 3.0));
    }

    if n.has_conflict {
        scene.fill(Fill::NonZero, Affine::IDENTITY, t.red, None, &Circle::new((cx, cy), L::WC_R));
        let s = Stroke::new(1.5);
        let d = 2.4;
        scene.stroke(&s, Affine::IDENTITY, t.base, None, &Line::new((cx - d, cy - d), (cx + d, cy + d)));
        scene.stroke(&s, Affine::IDENTITY, t.base, None, &Line::new((cx - d, cy + d), (cx + d, cy - d)));
    } else if n.is_working_copy {
        // Amber concentric ring: outer stroke r=WC_R+1 width 1.8 + inner dot r=2.5.
        scene.stroke(&Stroke::new(1.8), Affine::IDENTITY, t.amber, None, &Circle::new((cx, cy), L::WC_R + 1.0));
        scene.fill(Fill::NonZero, Affine::IDENTITY, t.amber, None, &Circle::new((cx, cy), 2.5));
    } else if n.is_hidden {
        let c = theme::with_opacity(lane, 0.35);
        scene.stroke(&Stroke::new(1.2), Affine::IDENTITY, c, None, &Circle::new((cx, cy), L::NODE_R - 0.5));
    } else if n.is_immutable {
        // 7x7 rounded rect rotated 45° (diamond), dimmer (opacity 0.5).
        let dim = theme::with_opacity(lane, theme::GRAPH_IMMUTABLE_OPACITY);
        let d = Rect::new(cx - 3.5, cy - 3.5, cx + 3.5, cy + 3.5).to_rounded_rect(1.0);
        scene.fill(
            Fill::NonZero,
            Affine::rotate_about(std::f64::consts::FRAC_PI_4, Point::new(cx, cy)),
            dim,
            None,
            &d,
        );
    } else {
        scene.fill(Fill::NonZero, Affine::IDENTITY, node_c, None, &Circle::new((cx, cy), L::NODE_R));
    }
}

// --- content lines ---------------------------------------------------------

fn draw_node_line(scene: &mut Scene, x: f64, cy: f64, n: &CommitNode, t: &Palette, ctx: &RenderCtx) {
    let mut x = x;
    if n.is_divergent {
        x = alert_badge(scene, x, cy, "divergent", t, ctx);
    }
    if n.has_conflict {
        x = alert_badge(scene, x, cy, "conflict", t, ctx);
    }

    // jj descriptions carry a trailing newline (and may be multi-line); lightjj
    // shows only the first line. Take it and trim trailing whitespace so no
    // control char renders as a `.notdef` box.
    let first_line = n.description.lines().next().unwrap_or("").trim_end();
    let (desc, color) = if first_line.is_empty() {
        let label = if n.is_empty { "(empty)" } else { "(no description)" };
        (label, t.overlay0)
    } else if n.is_immutable {
        // Immutable description dimmed to overlay0.
        (first_line, t.overlay0)
    } else {
        (first_line, t.text)
    };
    let bl = baseline_for(cy, font::FS_MD, &ctx.fonts.ui);
    text::draw_text(scene, &ctx.fonts.ui, font::FS_MD, color, x, bl, desc);
}

fn alert_badge(scene: &mut Scene, x: f64, cy: f64, label: &str, t: &Palette, ctx: &RenderCtx) -> f64 {
    let tw = text::measure(&ctx.fonts.ui_bold, font::FS_XS, label) as f64;
    let w = tw + 8.0;
    let r = Rect::new(x, cy - 7.0, x + w, cy + 7.0);
    fill_round(scene, r, 3.0, t.bg_error);
    scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, t.red, None, &r.to_rounded_rect(3.0));
    let bl = baseline_for(cy, font::FS_XS, &ctx.fonts.ui_bold);
    text::draw_text(scene, &ctx.fonts.ui_bold, font::FS_XS, t.red, x + 4.0, bl, label);
    x + w + NODE_LINE_GAP
}

fn draw_bookmark_line(
    scene: &mut Scene,
    x: f64,
    cy: f64,
    n: &CommitNode,
    t: &Palette,
    ctx: &RenderCtx,
    lane: Color,
) {
    let mut x = x;
    for b in &n.bookmarks {
        match &b.kind {
            BookmarkKind::Local => {
                // Leading glyph is `⑂` (U+2442, branch fork) per §4.3; resolved via
                // the symbol fallback chain in `text`.
                let base_label = format!("\u{2442} {}", b.name);
                let mut full = base_label.clone();
                if b.conflicted {
                    full.push_str(" ??");
                }
                if b.unsynced {
                    full.push_str(" *");
                }
                let tw = text::measure(&ctx.fonts.ui, font::FS_XS, &full) as f64;
                // padding 0 5 → 5px each side.
                let w = tw + 10.0;
                // Badge box height calibrated to docs/reference/revisions.png: the
                // reference `main`/`experiment` chips are ~27px tall @2x (≈13.5
                // logical px), not 16 — so cy ± 6.5.
                let r = Rect::new(x, cy - 6.5, x + w, cy + 6.5);
                fill_round(scene, r, 3.0, t.surface0);
                // Lane-tinted (border = lane@50% over the fill, text = lane) unless
                // conflicted — matches the reference top/bottom border [107,80,37]
                // (= graph[0] mixed 50% with the surface0 fill), per §4.3.
                let (border, txt) = if b.conflicted {
                    (t.surface1, t.subtext0)
                } else {
                    (theme::with_opacity(lane, 0.5), lane)
                };
                scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, border, None, &r.to_rounded_rect(3.0));
                let bl = baseline_for(cy, font::FS_XS, &ctx.fonts.ui);
                let mut mx = text::draw_text(scene, &ctx.fonts.ui, font::FS_XS, txt, x + 5.0, bl, &base_label);
                if b.conflicted {
                    mx = text::draw_text(scene, &ctx.fonts.ui_bold, font::FS_XS, t.red, mx + 2.0, bl, "??");
                }
                if b.unsynced {
                    text::draw_text(scene, &ctx.fonts.ui_bold, font::FS_XS, t.amber, mx + 2.0, bl, "*");
                }
                x += w + BOOKMARK_GAP;
            }
            BookmarkKind::Remote { remote } => {
                let label = format!("{}/{}", remote, b.name);
                let tw = text::measure(&ctx.fonts.ui, font::FS_2XS, &label) as f64;
                let w = tw + 10.0;
                let r = Rect::new(x, cy - 7.0, x + w, cy + 7.0);
                scene.stroke(&Stroke::new(1.0), Affine::IDENTITY, t.surface0, None, &r.to_rounded_rect(3.0));
                let bl = baseline_for(cy, font::FS_2XS, &ctx.fonts.ui);
                text::draw_text(scene, &ctx.fonts.ui, font::FS_2XS, t.overlay0, x + 5.0, bl, &label);
                x += w + BOOKMARK_GAP;
            }
        }
    }
}

fn draw_meta_line(scene: &mut Scene, x: f64, cy: f64, n: &CommitNode, t: &Palette, ctx: &RenderCtx) {
    let mut x = x;
    // Change id: amber prefix + faint tail (to 12 chars). Divergent appends /N red.
    let plen = n.change_prefix_len.min(n.change_id.len());
    let tail_end = n.change_id.len().min(12);
    let prefix = &n.change_id[..plen.min(tail_end)];
    let tail = if plen < tail_end { &n.change_id[plen..tail_end] } else { "" };
    let bl = baseline_for(cy, font::FS_SM, &ctx.fonts.mono);
    let end = text::draw_text(scene, &ctx.fonts.mono, font::FS_SM, t.amber, x, bl, prefix);
    let end = text::draw_text(scene, &ctx.fonts.mono, font::FS_SM, t.text_faint, end, bl, tail);
    let end = if n.is_divergent {
        text::draw_text(scene, &ctx.fonts.mono_bold, font::FS_SM, t.red, end, bl, "/1")
    } else {
        end
    };
    x = end + META_GAP;

    // Commit id: faint prefix-highlight + tail (to 12 chars), --fs-xs overlay0.
    let cplen = n.commit_prefix_len.min(n.commit_id.len());
    let ctail_end = n.commit_id.len().min(12);
    let cprefix = &n.commit_id[..cplen.min(ctail_end)];
    let ctail = if cplen < ctail_end { &n.commit_id[cplen..ctail_end] } else { "" };
    let cbl = baseline_for(cy, font::FS_XS, &ctx.fonts.mono);
    let end = text::draw_text(scene, &ctx.fonts.mono, font::FS_XS, t.overlay0, x, cbl, cprefix);
    let end = text::draw_text(scene, &ctx.fonts.mono, font::FS_XS, t.text_faint, end, cbl, ctail);
    x = end + META_GAP;

    // Relative timestamp chip.
    let ts = relative_time(n.timestamp_ms, NOW_MS);
    let tbl = baseline_for(cy, font::FS_XS, &ctx.fonts.ui);
    text::draw_text(scene, &ctx.fonts.ui, font::FS_XS, t.overlay0, x, tbl, &ts);
}

/// Width (px) the `(elided revisions)` marker spans from the content x. lightjj
/// does not stretch the dashed rules to the panel edge; matched to the reference.
const ELIDED_RULE_W: f64 = 205.0;

fn draw_elided(scene: &mut Scene, x: f64, x1: f64, y: f64, t: &Palette, ctx: &RenderCtx) {
    // §4.6: `(elided revisions)` label, --text-faint --fs-xs, *centered* between two
    // 1px dashed --surface1 rules. The rules span a fixed marker width (not the whole
    // panel), so the label sits left-of-panel-center as in the reference.
    let cy = y + L::ROW_H * 0.5;
    let bl = baseline_for(cy, font::FS_XS, &ctx.fonts.ui);
    const LABEL: &str = "(elided revisions)";
    const RULE_GAP: f64 = 8.0; // gap between a rule and the label.
    let tw = text::measure(&ctx.fonts.ui, font::FS_XS, LABEL) as f64;
    let right = (x + ELIDED_RULE_W).min(x1 - RULE_GAP);
    let avail = (right - x).max(tw);
    let text_x = x + (avail - tw) * 0.5;
    let end = text::draw_text(scene, &ctx.fonts.ui, font::FS_XS, t.text_faint, text_x, bl, LABEL);
    let stroke = Stroke::new(1.0).with_dashes(0.0, [2.0, 2.0]);
    if text_x - RULE_GAP > x {
        scene.stroke(&stroke, Affine::IDENTITY, t.surface1, None, &Line::new((x, cy), (text_x - RULE_GAP, cy)));
    }
    if right > end + RULE_GAP {
        scene.stroke(&stroke, Affine::IDENTITY, t.surface1, None, &Line::new((end + RULE_GAP, cy), (right, cy)));
    }
}

// --- relative time ---------------------------------------------------------

/// Compact relative age like "3h", "2d", "5mo", "6y" from `ts` vs `now` (ms).
fn relative_time(ts_ms: i64, now_ms: i64) -> String {
    let secs = (now_ms - ts_ms).max(0) / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    let months = days / 30;
    let years = days / 365;
    if secs < 60 {
        format!("{}s", secs.max(1))
    } else if mins < 60 {
        format!("{mins}m")
    } else if hours < 24 {
        format!("{hours}h")
    } else if days < 30 {
        format!("{days}d")
    } else if years < 1 {
        format!("{months}mo")
    } else {
        format!("{years}y")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The windowed draw must visit exactly the rows a full (un-windowed) walk
    /// would have *painted* — windowing only elides off-screen work, never
    /// changes which rows are on-screen. We model both walks over a fixed-height
    /// row list and assert the visited index sets are identical for a spread of
    /// scroll offsets, including the partially-scrolled boundary cases.
    #[test]
    fn windowing_matches_full_walk_cropped() {
        let row_h = L::ROW_H;
        let n_rows = 600usize;
        let view_top = 100.0;
        let view_bottom = 100.0 + 800.0; // an 800px content rect
        let top_pad = LIST_TOP_PAD;

        for &scroll in &[
            0.0, 5.0, 17.0, 18.0, 19.0, 100.0, 333.0, 1000.0, 5000.0, 10_790.0,
            // exactly one row, just-under, just-over the last row, way past end:
            row_h, row_h - 0.01, (n_rows as f64) * row_h, 1e6,
        ] {
            let top0 = view_top + top_pad - scroll;

            // Reference: walk ALL rows, keep those intersecting the viewport.
            let mut full = Vec::new();
            for i in 0..n_rows {
                let y = top0 + i as f64 * row_h;
                if y > view_bottom {
                    break;
                }
                if y + row_h <= view_top {
                    continue; // entirely above
                }
                full.push(i);
            }

            // Windowed: jump to first_visible, then walk until past the bottom.
            let first = first_visible_row(top0, view_top, n_rows);
            let mut win = Vec::new();
            let mut y = top0 + first as f64 * row_h;
            for i in first..n_rows {
                if y > view_bottom {
                    break;
                }
                // The renderer's break uses row.y0 > rect.y1; rows above are never
                // reached because we started at `first`. Mirror the same gate.
                if y + row_h > view_top {
                    win.push(i);
                }
                y += row_h;
            }

            assert_eq!(
                win, full,
                "scroll={scroll}: windowed visible rows must equal full-walk cropped"
            );
            // And the jump must never skip a row that should be visible: the first
            // windowed row is ≤ the first reference row.
            if let (Some(&wf), Some(&ff)) = (win.first(), full.first()) {
                assert!(wf <= ff, "scroll={scroll}: first windowed row {wf} > first visible {ff}");
            }
        }
    }

    #[test]
    fn first_visible_row_arithmetic() {
        // Unscrolled: row 0 starts at/after the viewport top → start at 0.
        assert_eq!(first_visible_row(100.0, 100.0, 50), 0);
        assert_eq!(first_visible_row(105.0, 100.0, 50), 0);
        // Scrolled so rows 0..k are fully above: the k-th row is first visible.
        // top0 = 100 - k*ROW_H means row k's top lands exactly at the viewport top.
        let k = 7usize;
        let top0 = 100.0 - k as f64 * L::ROW_H;
        assert_eq!(first_visible_row(top0, 100.0, 50), k);
        // A hair more scroll keeps row k partly visible (floor stays at k).
        assert_eq!(first_visible_row(top0 - 0.01, 100.0, 50), k);
        // Clamp to row_count when scrolled past the end.
        assert_eq!(first_visible_row(100.0 - 1e6, 100.0, 50), 50);
    }

    #[test]
    fn relative_time_buckets() {
        let now = NOW_MS;
        assert_eq!(relative_time(now - 30_000, now), "30s");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5m");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3h");
        assert_eq!(relative_time(now - 2 * 86_400_000, now), "2d");
        // 2026-01-15 -> 2026-06-19 is ~5 months.
        assert_eq!(relative_time(1_768_471_200_000, now), "5mo");
    }

    #[test]
    fn continuation_cells_keeps_through_lanes_drops_terminating_connectors() {
        // A merge row (`feat`): own lane vertical/elided + a converging child via an
        // elbow + horizontal stub. The continuation rows below it must keep only the
        // lanes that exit the bottom: the elided trunk stays dashed, the converging
        // elbow and its horizontal stub vanish (the child lane ended at the node row).
        let cells = vec![Cell::Elided, Cell::Horizontal, Cell::ElbowBottomLeft];
        assert_eq!(
            continuation_cells(&cells),
            vec![Cell::Elided, Cell::Empty, Cell::Empty]
        );

        // A fork row: node trunk vertical + a newly opened lane (top elbow). Both
        // lanes descend, so the fork elbow becomes a vertical on continuation rows.
        let cells = vec![Cell::Vertical, Cell::Horizontal, Cell::ElbowTopLeft];
        assert_eq!(
            continuation_cells(&cells),
            vec![Cell::Vertical, Cell::Empty, Cell::Vertical]
        );

        // A plain carried lane stays a vertical through the whole revision.
        assert_eq!(continuation_cells(&[Cell::Vertical]), vec![Cell::Vertical]);
    }
}
