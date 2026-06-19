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
use jjscratch::input;
use jjscratch::model::{mock, CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::ui::{self, Frame, UiState};
use jjscratch::Headless;
use vello::kurbo::Affine;
use vello::Scene;

/// Everything the renderer needs from the data path: the snapshot, the
/// working-copy diff, and (under `jjlib`) the operation log for the Oplog drawer.
struct Data {
    snapshot: Snapshot,
    diff: Option<CommitDiff>,
    #[cfg(feature = "jjlib")]
    oplog: Vec<jjscratch::model::jjlib::OpEntry>,
}

/// Source data from a real repo when `JJSCRATCH_REPO` is set and the `jjlib`
/// feature is on; otherwise mock.
fn load_data() -> Result<Data> {
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
            // Load the op log so an open Oplog drawer shows real operations.
            let oplog = jjlib::oplog(&loaded, 30).unwrap_or_default();
            eprintln!(
                "loaded {} revisions from {} (workspace {}), {} ops",
                snapshot.revision_count(),
                snapshot.repo_name,
                snapshot.workspace_name,
                oplog.len(),
            );
            return Ok(Data { snapshot, diff, oplog });
        }
    }
    Ok(Data {
        snapshot: mock::snapshot(),
        diff: Some(mock::working_copy_diff()),
        #[cfg(feature = "jjlib")]
        oplog: Vec::new(),
    })
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

    let data = load_data()?;
    let snapshot = &data.snapshot;
    let fonts = Fonts::bundled();
    // Default the cursor to the working copy so the RevisionHeader/diff (which
    // currently shows the working-copy diff) and the selected row agree,
    // regardless of jj-lib's topological stream order.
    let mut state = UiState::default();
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);

    // Optional state driver: `JJSCRATCH_KEYS="2"` / `"4"` / `"t"` / `"cmd+k"`
    // (space-separated) applies keys through the real input router before the
    // shot, so a single `shot` invocation can capture any UI state for review.
    if let Ok(keys) = std::env::var("JJSCRATCH_KEYS") {
        for k in keys.split_whitespace() {
            input::handle_key(k, &mut state, snapshot);
        }
    }

    // Clear color follows the active theme so the @Nx letterbox matches.
    let clear = state.theme.palette().base;

    let frame = Frame {
        #[cfg(feature = "jjlib")]
        oplog: &data.oplog,
        ..Default::default()
    };

    // Lay the UI out once at the logical size. At @1x it is rendered directly;
    // at @Nx it is appended into a fresh render scene under Affine::scale(N) so
    // every vector primitive (and glyph) is rasterised at device resolution.
    let mut ui_scene = Scene::new();
    ui::build_scene(
        &mut ui_scene,
        snapshot,
        data.diff.as_ref(),
        &state,
        &fonts,
        &frame,
        width as f64,
        height as f64,
    );

    let (dev_w, dev_h) = (width * scale, height * scale);
    let img = if scale == 1 {
        hl.render(&ui_scene, dev_w, dev_h, clear)?
    } else {
        let mut scene = Scene::new();
        scene.append(&ui_scene, Some(Affine::scale(scale as f64)));
        hl.render(&scene, dev_w, dev_h, clear)?
    };
    img.save_png(&out)?;
    eprintln!("wrote {out} ({dev_w}x{dev_h}, logical {width}x{height} @{scale}x)");
    Ok(())
}
