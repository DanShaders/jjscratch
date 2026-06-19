//! `preview_oplog` — headless preview of the OPLOG drawer (lightjj's "4").
//!
//! Loads the committed fixture via jj-lib, queries its operation log with
//! [`jjscratch::model::jjlib::oplog`], and renders the OPLOG drawer (the bottom
//! 40% of a 1280×800 frame, floating above where the statusbar would sit) to a
//! PNG using the bin-local [`oplog_view`] renderer.
//!
//! Usage: cargo run --features jjlib --bin preview_oplog -- [out.png] [width] [height]
//!
//! The fixture path defaults to `<manifest>/fixture/repo`; override with
//! `JJSCRATCH_REPO=<path>`.
//!
//! This bin needs the `jjlib` feature (the op-log query lives there). Without
//! it, `main` is a stub so a featureless `cargo build` stays green — Cargo.toml
//! is owned elsewhere, so `required-features` isn't available to us.

/// Stub entry point WITHOUT the feature.
#[cfg(not(feature = "jjlib"))]
fn main() {
    eprintln!("preview_oplog needs --features jjlib.");
}

#[cfg(feature = "jjlib")]
use anyhow::Result;
#[cfg(feature = "jjlib")]
use jjscratch::model::jjlib;
#[cfg(feature = "jjlib")]
use jjscratch::text::Fonts;
#[cfg(feature = "jjlib")]
use jjscratch::theme;
#[cfg(feature = "jjlib")]
use jjscratch::ui::{fill_rect, RenderCtx};
#[cfg(feature = "jjlib")]
use jjscratch::Headless;
#[cfg(feature = "jjlib")]
use vello::kurbo::Rect;
#[cfg(feature = "jjlib")]
use vello::Scene;

#[cfg(feature = "jjlib")]
#[path = "oplog_view.rs"]
mod oplog_view;

#[cfg(feature = "jjlib")]
fn fixture_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("JJSCRATCH_REPO") {
        std::path::PathBuf::from(p)
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture/repo")
    }
}

#[cfg(feature = "jjlib")]
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "oplog.png".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    // Load the fixture's operation log via jj-lib.
    let path = fixture_path();
    eprintln!("loading op log from {}", path.display());
    let loaded = jjlib::open(&path)?;
    let ops = jjlib::oplog(&loaded, 30)?;
    eprintln!("loaded {} operations", ops.len());
    for op in ops.iter().take(8) {
        eprintln!("  {:<14} {:>4}  {}", op.id, op.time, op.description);
    }

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let fonts = Fonts::bundled();
    let palette = theme::DARK;
    let ctx = RenderCtx { fonts: &fonts, theme: &palette };

    let (w, h) = (width as f64, height as f64);
    let mut scene = Scene::new();

    // Fill the whole frame with the base background so the drawer reads in
    // context (the upper 60% stands in for the revisions/diff area).
    fill_rect(&mut scene, Rect::new(0.0, 0.0, w, h), palette.base);

    // OPLOG drawer occupies the bottom 40% of the frame, above a 24px statusbar
    // strip — exactly where lightjj floats it.
    let statusbar_h = theme::layout::STATUSBAR_H;
    let drawer_top = h * 0.60;
    let drawer = Rect::new(0.0, drawer_top, w, h - statusbar_h);

    // A 1px top border to separate the drawer from the content above it.
    fill_rect(&mut scene, Rect::new(0.0, drawer_top, w, drawer_top + 1.0), palette.surface1);

    oplog_view::render(&mut scene, drawer, &ops, &ctx);

    // Faux statusbar strip beneath the drawer for context.
    fill_rect(&mut scene, Rect::new(0.0, h - statusbar_h, w, h), palette.crust);

    let img = hl.render(&scene, width, height, palette.base)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}
