//! Revision graph renderer (the left panel).
//!
//! STATUS: PLACEHOLDER. This draws a simplified one-row-per-commit list so the
//! frame composes. The real implementation must reproduce lightjj's GraphSvg
//! pixel-for-pixel (docs/spec/ui-spec.md §4): flattened 18px rows (node line +
//! optional bookmark line + description/meta line), the lane gutter with
//! vertical/horizontal/elbow connectors computed from the DAG edges (use
//! [`crate::graph_layout`]), node glyphs (@ amber concentric ring, ○ filled
//! lane-colored, ◆ dimmed diamond for immutable, × conflict, ◌ hidden), change-id
//! prefix highlighting, bookmark badges, immutable description dimming, and the
//! selected (inset amber bar) / hovered / checked row states.

use vello::kurbo::{Affine, Circle, Rect};
use vello::peniko::Fill;
use vello::Scene;

use super::{baseline, fill_rect, RenderCtx, UiState};
use crate::model::Snapshot;
use crate::text;
use crate::theme::{self, layout as L};

pub fn render(scene: &mut Scene, rect: Rect, snapshot: &Snapshot, state: &UiState, ctx: &RenderCtx) {
    let t = ctx.theme;
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let mut y = rect.y0 - state.graph_scroll;
    for (i, n) in snapshot.nodes.iter().enumerate() {
        let row = Rect::new(rect.x0, y, rect.x1, y + L::ROW_H);

        if i == state.selected {
            fill_rect(scene, row, t.bg_selected);
            fill_rect(scene, Rect::new(row.x0, row.y0, row.x0 + 2.0, row.y1), t.amber);
        } else if state.hovered == Some(i) {
            fill_rect(scene, row, t.bg_hover);
        }

        // Node glyph in lane 0.
        let cx = rect.x0 + L::CHECK_GUTTER_W + L::CELL_W * 0.5 + 4.0;
        let cy = y + L::ROW_H * 0.5;
        let lane = theme::with_opacity(t.graph[0], theme::GRAPH_NODE_OPACITY);
        if n.is_working_copy {
            scene.stroke(
                &vello::kurbo::Stroke::new(1.8),
                Affine::IDENTITY, t.amber, None, &Circle::new((cx, cy), L::WC_R + 1.0),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, t.amber, None, &Circle::new((cx, cy), 2.5));
        } else if n.is_immutable {
            let dim = theme::with_opacity(t.graph[0], theme::GRAPH_IMMUTABLE_OPACITY);
            let d = Rect::new(cx - 3.5, cy - 3.5, cx + 3.5, cy + 3.5).to_rounded_rect(1.0);
            let center = vello::kurbo::Point::new(cx, cy);
            scene.fill(Fill::NonZero, Affine::rotate_about(0.785, center), dim, None, &d);
        } else {
            scene.fill(Fill::NonZero, Affine::IDENTITY, lane, None, &Circle::new((cx, cy), L::NODE_R));
        }

        // change-id (amber prefix + faint tail) + description.
        let text_x = cx + 16.0;
        let pfx = &n.change_id[..n.change_prefix_len.min(n.change_id.len())];
        let end = text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_SM, t.amber,
            text_x, baseline(cy, theme::font::FS_SM), pfx);
        let tail_end = n.change_id.len().min(12);
        let tail = &n.change_id[n.change_prefix_len.min(tail_end)..tail_end];
        let end = text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_SM, t.text_faint,
            end, baseline(cy, theme::font::FS_SM), tail);

        let (desc, color) = if n.description.is_empty() {
            ("(no description)".to_string(), t.overlay0)
        } else {
            let c = if n.is_immutable { t.overlay0 } else { t.text };
            (n.description.clone(), c)
        };
        let mut dx = text::draw_text(scene, &ctx.fonts.ui, theme::font::FS_MD, color,
            end + 10.0, baseline(cy, theme::font::FS_MD), &desc);

        // bookmark badges (very rough).
        for b in &n.bookmarks {
            dx += 8.0;
            let label = format!("\u{2942} {}", b.name);
            let w = text::measure(&ctx.fonts.ui, theme::font::FS_XS, &label) as f64 + 10.0;
            let badge = Rect::new(dx, cy - 8.0, dx + w, cy + 8.0).to_rounded_rect(3.0);
            scene.fill(Fill::NonZero, Affine::IDENTITY, t.surface0, None, &badge);
            text::draw_text(scene, &ctx.fonts.ui, theme::font::FS_XS, t.subtext0,
                dx + 5.0, baseline(cy, theme::font::FS_XS), &label);
            dx += w;
        }

        y += L::ROW_H;
        if y > rect.y1 {
            break;
        }
    }

    scene.pop_layer();
}
