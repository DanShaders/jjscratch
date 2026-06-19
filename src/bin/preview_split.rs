// INTEGRATION: move to src/ui/diff.rs (or a submodule); render this when the unified/split toggle is in split mode
//
//! `preview_split` — isolated harness for the SPLIT (side-by-side) diff view.
//!
//! Renders the working-copy diff in split layout (ui-spec §5.7) to a PNG so the
//! renderer can be eyeballed against lightjj's split view before it is wired
//! into the diff toolbar's unified/split toggle.
//!
//! Usage:
//!   cargo run --bin preview_split -- [out.png] [width] [height]
//!   JJSCRATCH_REPO=$PWD/fixture/repo \
//!     cargo run --features jjlib --bin preview_split -- out.png
//!
//! Data source mirrors `shot.rs`: a real repo under `--features jjlib` +
//! `JJSCRATCH_REPO`, otherwise the fixture-matching `mock::working_copy_diff()`.

use anyhow::Result;
use jjscratch::model::{mock, CommitDiff};
use jjscratch::text::Fonts;
use jjscratch::ui::{RenderCtx, Theme};
use jjscratch::Headless;
use vello::kurbo::Rect;
use vello::Scene;

// Pull in the split renderer as a module of this bin. (Its own `fn main` is a
// dead-code shim so it can still compile as its own autobin target.)
#[path = "split_diff_view.rs"]
mod split_diff_view;

/// Load the working-copy diff: real repo under `jjlib` + `JJSCRATCH_REPO`,
/// else the mock fixture diff (src/main.rs modified, src/parser.rs deleted).
fn load_diff() -> Result<CommitDiff> {
    #[cfg(feature = "jjlib")]
    {
        if let Some(path) = std::env::var_os("JJSCRATCH_REPO") {
            use jjscratch::model::jjlib;
            let path = std::path::PathBuf::from(path);
            eprintln!("loading real repo: {}", path.display());
            let loaded = jjlib::open(&path)?;
            if let Some(wc) = loaded.wc_commit_id_hex() {
                return Ok(jjlib::commit_diff(&loaded, &wc)?);
            }
            eprintln!("repo has no working-copy commit; falling back to mock");
        }
    }
    Ok(mock::working_copy_diff())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "split.png".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let diff = load_diff()?;
    eprintln!(
        "diff: {} files (+{} -{})",
        diff.files.len(),
        diff.total_added(),
        diff.total_removed()
    );

    let fonts = Fonts::bundled();
    let theme = Theme::Dark.palette();
    let ctx = RenderCtx { fonts: &fonts, theme };

    let mut scene = Scene::new();
    let rect = Rect::new(0.0, 0.0, width as f64, height as f64);
    split_diff_view::render(&mut scene, rect, &diff, &ctx);

    let img = hl.render(&scene, width, height, theme.base)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}
