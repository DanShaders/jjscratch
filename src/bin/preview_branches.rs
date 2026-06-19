//! `preview_branches` — isolated preview harness for the BRANCHES / bookmarks
//! view (`branches_view::render`). Loads the fixture snapshot (mock by default,
//! or a real repo via `--features jjlib` + `JSSCRATCH_REPO`) and renders a
//! 1280x800 PNG containing a minimal panel header + the branches body.
//!
//! The real toolbar/tab-bar are owned by `ui.rs` (off-limits to this isolated
//! pass); a later integration pass wires `branches_view::render` into
//! `ui::build_scene`. Here we draw only a minimal "BRANCHES" panel header above
//! the body so the view can be verified in isolation.
//!
//! Usage: cargo run --features jjlib --bin preview_branches -- [out.png] [w] [h]

#[path = "branches_view.rs"]
mod branches_view;

use anyhow::Result;
use jjscratch::model::{mock, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::theme::{self, font, layout as L};
use jjscratch::ui::{self, RenderCtx};
use jjscratch::Headless;
use vello::kurbo::Rect;
use vello::Scene;

/// Source the snapshot from a real repo when `JSSCRATCH_REPO` is set and the
/// `jjlib` feature is on; otherwise the fixture-matching mock. Mirrors
/// `src/bin/shot.rs`'s loader (we only need the snapshot, not the diff).
fn load_snapshot() -> Result<Snapshot> {
    #[cfg(feature = "jjlib")]
    {
        if let Some(path) = std::env::var_os("JSSCRATCH_REPO") {
            use jjscratch::model::jjlib;
            let path = std::path::PathBuf::from(path);
            eprintln!("loading real repo: {}", path.display());
            let loaded = jjlib::open(&path)?;
            let snapshot = jjlib::snapshot(&loaded)?;
            eprintln!(
                "loaded {} revisions from {} (workspace {})",
                snapshot.revision_count(),
                snapshot.repo_name,
                snapshot.workspace_name,
            );
            return Ok(snapshot);
        }
    }
    Ok(mock::snapshot())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "branches_out.png".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let snapshot = load_snapshot()?;
    let fonts = Fonts::bundled();
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };

    let (w, h) = (width as f64, height as f64);
    let mut scene = Scene::new();

    // Base background.
    ui::fill_rect(&mut scene, Rect::new(0.0, 0.0, w, h), palette.base);

    // Minimal panel header "BRANCHES" (the real toolbar/tab-bar live in ui.rs).
    let header = Rect::new(0.0, 0.0, w, L::PANEL_HEADER_H);
    ui::fill_rect(&mut scene, header, palette.mantle);
    ui::border_bottom(&mut scene, header, palette.surface0);
    let hcy = header.center().y;
    jjscratch::text::draw_text(
        &mut scene,
        &fonts.ui_bold,
        font::FS_SM,
        palette.subtext1,
        12.0,
        ui::baseline_for(hcy, font::FS_SM, &fonts.ui_bold),
        "BRANCHES",
    );
    // Count badge mirroring the chrome panel header.
    let count = snapshot
        .nodes
        .iter()
        .flat_map(|n| &n.bookmarks)
        .filter(|b| matches!(b.kind, jjscratch::model::BookmarkKind::Local))
        .count();
    let badge = count.to_string();
    let bsz = font::FS_XS;
    let bw = jjscratch::text::measure(&fonts.ui_bold, bsz, &badge) as f64 + 12.0;
    let pill = Rect::new(header.x1 - 12.0 - bw, hcy - 8.0, header.x1 - 12.0, hcy + 8.0);
    ui::fill_round(&mut scene, pill, 8.0, palette.surface0);
    jjscratch::text::draw_text(
        &mut scene,
        &fonts.ui_bold,
        bsz,
        palette.subtext0,
        pill.x0 + 6.0,
        ui::baseline_for(hcy, bsz, &fonts.ui_bold),
        &badge,
    );

    // Branches body fills the rest of the window.
    let body = Rect::new(0.0, header.y1, w, h);
    branches_view::render(&mut scene, body, &snapshot, &ctx);

    let img = hl.render(&scene, width, height, palette.base)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}
