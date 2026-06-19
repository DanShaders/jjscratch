//! `shot` — the headless screenshot harness. Renders the jjscratch UI to a PNG.
//!
//! Usage: cargo run --bin shot -- [out.png] [width] [height]
//!
//! Data source: the mock fixture snapshot by default. When built with
//! `--features jjlib` and given `JJSCRATCH_REPO=<path>`, loads a real repo.

use anyhow::Result;
use jjscratch::model::{mock, CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::theme;
use jjscratch::ui::{self, RenderCtx, UiState};
use jjscratch::Headless;
use vello::Scene;

/// Source the snapshot + working-copy diff from a real repo when
/// `JJSCRATCH_REPO` is set and the `jjlib` feature is on; otherwise mock.
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
            eprintln!(
                "loaded {} revisions from {} (workspace {})",
                snapshot.revision_count(),
                snapshot.repo_name,
                snapshot.workspace_name,
            );
            return Ok((snapshot, diff));
        }
    }
    Ok((mock::snapshot(), Some(mock::working_copy_diff())))
}

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

    let (snapshot, diff) = load_data()?;
    let fonts = Fonts::bundled();
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };
    let state = UiState::default();

    let mut scene = Scene::new();
    ui::build_scene(
        &mut scene,
        &snapshot,
        diff.as_ref(),
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
