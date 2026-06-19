//! `shot` — the headless screenshot harness. Renders the jjscratch UI to a PNG.
//!
//! Usage: cargo run --bin shot -- [out.png] [width] [height] [scale]
//!
//! `width`/`height` are LOGICAL pixels (default 1280x800). `scale` is the device
//! scale factor (default 1; pass 2 for @2x). The UI is always laid out at the
//! logical size; at `scale > 1` that child `Scene` is appended into the render
//! scene under `Affine::scale(scale)` and rasterised at `width*scale x
//! height*scale` — so an @2x shot is directly comparable to the lightjj
//! reference PNGs captured at deviceScaleFactor 2 (2560x1600).
//!
//! Data source: the mock fixture snapshot by default. When built with
//! `--features jjlib` and given `JJSCRATCH_REPO=<path>`, loads a real repo.

use anyhow::Result;
use jjscratch::model::{mock, CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::theme;
use jjscratch::ui::{self, RenderCtx, UiState};
use jjscratch::Headless;
use vello::kurbo::Affine;
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
    // Parse `--scale N` out of the args first, then take positionals. `scale`
    // may also be given as the 4th positional. Default scale is 1 (@1x), so the
    // historical `cargo run --bin shot -- out.png 1280 800` behavior is intact.
    let mut scale: u32 = 1;
    let mut positional: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scale" => {
                scale = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .filter(|&s| s >= 1)
                    .unwrap_or(scale);
            }
            other if other.starts_with("--scale=") => {
                scale = other[8..].parse().ok().filter(|&s| s >= 1).unwrap_or(scale);
            }
            _ => positional.push(arg),
        }
    }
    let mut positional = positional.into_iter();
    let out = positional.next().unwrap_or_else(|| "out.png".to_string());
    let width: u32 = positional.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = positional.next().and_then(|s| s.parse().ok()).unwrap_or(800);
    // 4th positional, if present and no `--scale` flag overrode it.
    if let Some(s) = positional.next().and_then(|s| s.parse().ok()) {
        if scale == 1 {
            scale = s;
        }
    }
    let scale = scale.max(1);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let (snapshot, diff) = load_data()?;
    let fonts = Fonts::bundled();
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };
    // Default the cursor to the working copy so the RevisionHeader/diff (which
    // currently shows the working-copy diff) and the selected row agree,
    // regardless of jj-lib's topological stream order.
    let mut state = UiState::default();
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);

    // Lay the UI out once at the logical size. At @1x it is rendered directly;
    // at @Nx it is appended into a fresh render scene under Affine::scale(N) so
    // every vector primitive (and glyph) is rasterised at device resolution.
    let mut ui_scene = Scene::new();
    ui::build_scene(
        &mut ui_scene,
        &snapshot,
        diff.as_ref(),
        &state,
        &ctx,
        width as f64,
        height as f64,
    );

    let (dev_w, dev_h) = (width * scale, height * scale);
    let img = if scale == 1 {
        hl.render(&ui_scene, dev_w, dev_h, palette.base)?
    } else {
        let mut scene = Scene::new();
        scene.append(&ui_scene, Some(Affine::scale(scale as f64)));
        hl.render(&scene, dev_w, dev_h, palette.base)?
    };
    img.save_png(&out)?;
    eprintln!("wrote {out} ({dev_w}x{dev_h}, logical {width}x{height} @{scale}x)");
    Ok(())
}
