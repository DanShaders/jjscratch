// INTEGRATION: move to src/ui/evolog.rs; show as a bottom drawer on key 5; replace MockEvologEntry with a jjlib evolog query (evolution of the selected change)
//
//! `preview_evolog` — renders a 1280x800 frame with the EVOLOG panel as a bottom
//! drawer (like Oplog) to a PNG, for isolated review of the evolog renderer.
//!
//! The full jjscratch shell (toolbar/tabbar/revision graph/diff/statusbar) is
//! laid out by `ui::build_scene` with the evolog drawer toggled open; that draws
//! the body chrome plus the drawer's "not yet wired" placeholder. We then
//! recompute the drawer rect the exact same way `ui.rs` does and overpaint it
//! with this bin's self-contained `evolog_view::render`, so the preview shows the
//! real evolog list as the bottom drawer above the statusbar.
//!
//! Usage: cargo run --bin preview_evolog -- [out.png] [width] [height]
//! Default output: evolog.png at 1280x800.

// The shared evolog renderer module (bin-local). Its own `main` shim satisfies
// the autobins requirement for `evolog_view.rs`; here we pull it in by path.
#[path = "evolog_view.rs"]
mod evolog_view;

use anyhow::Result;
use jjscratch::model::mock;
use jjscratch::text::Fonts;
use jjscratch::ui::{self, fill_rect, Frame, FrameLayout, RenderCtx, UiState, View};
use jjscratch::Headless;
use vello::kurbo::Rect;
use vello::Scene;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "evolog.png".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let snapshot = mock::snapshot();
    let diff = mock::working_copy_diff();
    let fonts = Fonts::bundled();

    // Drive the shell into the evolog-open state; select the working copy so the
    // body chrome behind the drawer reads like a normal frame.
    let mut state = UiState::default();
    state.evolog_open = true;
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);

    let palette = *state.theme.palette();
    let ctx = &RenderCtx { fonts: &fonts, theme: &palette };

    let frame = Frame::default();

    // 1) Full shell (toolbar/tabbar/graph/diff/statusbar) + drawer placeholder.
    let mut scene = Scene::new();
    ui::build_scene(
        &mut scene,
        &snapshot,
        Some(&diff),
        &state,
        &fonts,
        &frame,
        width as f64,
        height as f64,
    );

    // 2) Recompute the drawer rect EXACTLY as ui.rs does, then overpaint it with
    //    the real evolog renderer (over the placeholder build_scene drew).
    let fl = FrameLayout::compute(
        width as f64,
        height as f64,
        state.panel_width,
        View::Revisions,
        /* drawer_open = */ true,
    );
    if let Some(drawer) = fl.drawer {
        // 1px top border separating the drawer from the body above it (matches
        // ui.rs `draw_drawer`), then the evolog body below it.
        fill_rect(
            &mut scene,
            Rect::new(drawer.x0, drawer.y0, drawer.x1, drawer.y0 + 1.0),
            palette.surface1,
        );
        let body = Rect::new(drawer.x0, drawer.y0 + 1.0, drawer.x1, drawer.y1);

        let entries = evolog_view::mock_entries();
        evolog_view::render(&mut scene, body, &entries, evolog_view::mock_change_id(), ctx);
    }

    let clear = palette.base;
    let img = hl.render(&scene, width, height, clear)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}
