//! `preview_palette` — render the command palette (Cmd+K) overlay to PNGs.
//!
//! Draws a representative jjscratch UI frame as the backdrop (the real
//! `ui::build_scene` over the mock fixture, or a real repo via `--features
//! jjlib` + `JJSCRATCH_REPO`), then composes `palette_view::render` on top —
//! once with a sample query ("the") and once empty — to two PNGs.
//!
//! Usage: cargo run --features jjlib --bin preview_palette -- [out_prefix] [w] [h]
//! Outputs `<prefix>-query.png` and `<prefix>-empty.png` (default prefix
//! `palette`).

mod palette_view;

use anyhow::Result;
use jjscratch::model::{mock, CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::theme;
use jjscratch::ui::{self, RenderCtx, UiState};
use jjscratch::Headless;
use vello::kurbo::Rect;
use vello::Scene;

fn load_data() -> Result<(Snapshot, Option<CommitDiff>)> {
    #[cfg(feature = "jjlib")]
    {
        if let Some(path) = std::env::var_os("JJSCRATCH_REPO") {
            use jjscratch::model::jjlib;
            let path = std::path::PathBuf::from(path);
            eprintln!("loading real repo: {}", path.display());
            let loaded = jjlib::open(&path)?;
            let snapshot = jjlib::snapshot(&loaded)?;
            let diff = match loaded.wc_commit_id_hex() {
                Some(wc) => Some(jjlib::commit_diff(&loaded, &wc)?),
                None => None,
            };
            return Ok((snapshot, diff));
        }
    }
    Ok((mock::snapshot(), Some(mock::working_copy_diff())))
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let prefix = args.next().unwrap_or_else(|| "palette".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let (snapshot, diff) = load_data()?;
    let fonts = Fonts::bundled();
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };

    let mut state = UiState::default();
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);

    let viewport = Rect::new(0.0, 0.0, width as f64, height as f64);

    // Render one PNG per query case: the backdrop frame + the palette overlay.
    for (suffix, query) in [("query", "the"), ("empty", "")] {
        let mut scene = Scene::new();
        // Backdrop: the real jjscratch UI frame (so the overlay sits over a
        // representative dark UI, exactly like the integrated case will).
        ui::build_scene(
            &mut scene,
            &snapshot,
            diff.as_ref(),
            &state,
            &ctx,
            width as f64,
            height as f64,
        );
        // Overlay: the command palette.
        palette_view::render(&mut scene, viewport, query, &ctx);

        let img = hl.render(&scene, width, height, palette.base)?;
        let out = format!("{prefix}-{suffix}.png");
        img.save_png(&out)?;
        eprintln!("wrote {out} ({width}x{height}, query={query:?})");
    }

    Ok(())
}
