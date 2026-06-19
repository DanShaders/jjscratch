//! Diff panel renderer (the right column in Revisions view).
//!
//! STATUS: PLACEHOLDER. Draws file headers + raw diff lines so the frame
//! composes. The real implementation must reproduce lightjj's DiffPanel
//! pixel-for-pixel (docs/spec/ui-spec.md §5): per-file header with type badge +
//! path (dir faint / name bold) + +N/-N stats, sticky behavior, hunk headers
//! (bg, range, context), unified diff lines with the 11ch hanging-indent gutter,
//! TWO line-number columns (old/new), `+`/`-` prefix at 0.5 opacity, add/remove/
//! context backgrounds + left border colors, syntax token coloring (tok-* →
//! --syn-* per §1.3), word-diff highlights, and the unified/split toggle.

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::Scene;

use super::{baseline, border_bottom, fill_rect, RenderCtx, UiState};
use crate::model::{ChangeStatus, CommitDiff, LineKind, Snapshot};
use crate::text;
use crate::theme::{self, layout as L};

pub fn render(
    scene: &mut Scene,
    rect: Rect,
    _snapshot: &Snapshot,
    diff: Option<&CommitDiff>,
    state: &UiState,
    ctx: &RenderCtx,
) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let Some(diff) = diff else {
        text::draw_text(scene, &ctx.fonts.ui, theme::font::BASE, t.text_faint,
            rect.x0 + 24.0, rect.y0 + 48.0, "No changes");
        scene.pop_layer();
        return;
    };

    let mut y = rect.y0 - state.diff_scroll;
    for file in &diff.files {
        // File header.
        let hdr = Rect::new(rect.x0, y, rect.x1, y + 28.0);
        fill_rect(scene, hdr, t.mantle);
        border_bottom(scene, hdr, t.surface0);
        let hcy = hdr.center().y;
        let (badge, bcolor) = match file.status {
            ChangeStatus::Added => ("A", t.green),
            ChangeStatus::Modified => ("M", t.amber),
            ChangeStatus::Deleted => ("D", t.red),
            ChangeStatus::Renamed | ChangeStatus::Copied => ("R", t.amber),
        };
        let bx = rect.x0 + 12.0;
        text::draw_text(scene, &ctx.fonts.ui_bold, theme::font::FS_XS, bcolor, bx, baseline(hcy, theme::font::FS_XS), badge);
        let px = bx + 16.0;
        let pe = text::draw_text(scene, &ctx.fonts.ui_bold, theme::font::FS_MD, t.text, px, baseline(hcy, theme::font::FS_MD), &file.path);
        let stats = format!("+{}  -{}", file.added, file.removed);
        text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_SM, t.green, pe + 12.0, baseline(hcy, theme::font::FS_SM), &stats);
        y = hdr.y1;

        for hunk in &file.hunks {
            // Hunk header.
            let hh = Rect::new(rect.x0, y, rect.x1, y + L::DIFF_LINE_H);
            fill_rect(scene, hh, t.bg_hunk_header);
            let range = format!("@@ -{},{} +{},{} @@ {}", hunk.old_start, hunk.old_len, hunk.new_start, hunk.new_len, hunk.header_context);
            text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_SM, t.overlay0, rect.x0 + 12.0, baseline(hh.center().y, theme::font::FS_SM), &range);
            y = hh.y1;

            for line in &hunk.lines {
                let lr = Rect::new(rect.x0, y, rect.x1, y + L::DIFF_LINE_H);
                let (bg, fg, prefix): (Option<Color>, Color, &str) = match line.kind {
                    LineKind::Add => (Some(t.diff_add_bg), t.green, "+"),
                    LineKind::Remove => (Some(t.diff_remove_bg), t.red, "-"),
                    LineKind::Context => (None, t.subtext0, " "),
                };
                if let Some(bg) = bg {
                    fill_rect(scene, lr, bg);
                    fill_rect(scene, Rect::new(lr.x0, lr.y0, lr.x0 + 3.0, lr.y1), fg);
                }
                let lcy = lr.center().y;
                // line-number gutter (old/new), faint.
                let gut = format!("{:>3} {:>3}",
                    line.old_no.map(|n| n.to_string()).unwrap_or_default(),
                    line.new_no.map(|n| n.to_string()).unwrap_or_default());
                text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_SM, t.text_faint, rect.x0 + 8.0, baseline(lcy, theme::font::FS_SM), &gut);
                let code_x = rect.x0 + 8.0 + 11.0 * 7.0;
                let px = text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_MD, fg.multiply_alpha(0.6), code_x, baseline(lcy, theme::font::FS_MD), prefix);
                text::draw_text(scene, &ctx.fonts.mono, theme::font::FS_MD, fg, px + 4.0, baseline(lcy, theme::font::FS_MD), &line.text);
                y = lr.y1;
                if y > rect.y1 { break; }
            }
        }
        y += 4.0;
        if y > rect.y1 { break; }
    }

    scene.pop_layer();
}
