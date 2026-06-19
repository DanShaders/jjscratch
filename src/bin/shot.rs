//! `shot` — the headless screenshot harness. Renders the jjscratch UI to a PNG.
//!
//! Usage: cargo run --bin shot -- [out.png] [width] [height]
//!
//! Data source: the mock fixture snapshot by default. When built with
//! `--features jjlib` and given `JJSCRATCH_REPO=<path>`, loads a real repo.

use anyhow::Result;
use jjscratch::model::mock;
use jjscratch::text::Fonts;
use jjscratch::theme;
use jjscratch::ui::{self, RenderCtx, UiState};
use jjscratch::Headless;
use vello::Scene;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "out.png".to_string());
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
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };
    let state = UiState::default();

    let mut scene = Scene::new();
    ui::build_scene(
        &mut scene,
        &snapshot,
        Some(&diff),
        &state,
        &ctx,
        width as f64,
        height as f64,
    );

    let img = hl.render(&scene, width, height, palette.base)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}
