// INTEGRATION: move to src/ui/merge.rs; dispatch from build_scene when active_view==View::Merge (full width, hide graph); replace MockConflict with a jjlib conflict query
//
//! `preview_merge` — isolated preview of the Merge view (nav "3").
//!
//! Renders a 1280x800 full-width merge surface (ConflictQueue rail + 3-pane
//! ours|result|theirs editor) populated with representative stub conflicts to a
//! PNG, so the layout/colors can be reviewed against lightjj's MergePanel
//! without touching the live app. The merge view goes full width (graph hidden),
//! so this preview draws the merge surface across the whole frame below a thin
//! chrome strip standing in for the toolbar/tab-bar/statusbar bands.
//!
//! Usage: cargo run --bin preview_merge -- [out.png] [width] [height]

#[path = "merge_view.rs"]
mod merge_view;

use anyhow::Result;
use jjscratch::text::Fonts;
use jjscratch::theme::{self, layout as L};
use jjscratch::ui::{fill_rect, RenderCtx};
use jjscratch::Headless;
use vello::kurbo::Rect;
use vello::Scene;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "merge.png".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let fonts = Fonts::bundled();
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };

    let conflicts = merge_view::mock();

    let mut scene = Scene::new();
    let frame = Rect::new(0.0, 0.0, width as f64, height as f64);

    // Whole-frame base fill.
    fill_rect(&mut scene, frame, palette.base);

    // Thin chrome bands (toolbar + tab-bar above, statusbar below) so the
    // preview reads like the merge surface in situ. The merge surface itself
    // owns the full width between them — the revision graph is hidden in this
    // view (ui-spec §3: "Merge & doc views hide the revision panel entirely").
    let chrome_top = L::TOOLBAR_H + L::TAB_BAR_H;
    let chrome_bot = L::STATUSBAR_H;
    fill_rect(&mut scene, Rect::new(0.0, 0.0, frame.x1, L::TOOLBAR_H), palette.mantle);
    fill_rect(
        &mut scene,
        Rect::new(0.0, L::TOOLBAR_H, frame.x1, chrome_top),
        palette.crust,
    );
    fill_rect(
        &mut scene,
        Rect::new(0.0, frame.y1 - chrome_bot, frame.x1, frame.y1),
        palette.mantle,
    );
    // Active "⧉ Merge [3]" tab hint in the top band.
    jjscratch::text::draw_text(
        &mut scene,
        &fonts.ui,
        theme::font::FS_SM,
        palette.amber,
        12.0,
        jjscratch::ui::baseline_for(L::TOOLBAR_H / 2.0, theme::font::FS_SM, &fonts.ui),
        "\u{29C9} Merge  [3]",
    );

    let surface = Rect::new(0.0, chrome_top, frame.x1, frame.y1 - chrome_bot);
    merge_view::render(&mut scene, surface, &conflicts, &ctx);

    let img = hl.render(&scene, width, height, palette.base)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}
