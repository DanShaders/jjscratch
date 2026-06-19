//! `shot` — the headless screenshot harness.
//!
//! For now it renders a smoke-test scene (shapes + a rounded panel) to a PNG so
//! we can confirm the GPU pipeline works end-to-end on lavapipe. As the app grows
//! this binary will load a jj repo, build the real UI scene, and dump a PNG —
//! the primary way we "see" the app in this headless environment.
//!
//! Usage: cargo run --bin shot -- [out.png] [width] [height]

use anyhow::Result;
use jjscratch::Headless;
use vello::kurbo::{Affine, Circle, Rect, RoundedRect, Stroke};
use vello::peniko::{Color, Fill};
use vello::Scene;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "out.png".to_string());
    let width: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);
    let height: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(450);

    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    let fonts = jjscratch::text::Fonts::bundled();
    let mut scene = Scene::new();
    smoke_scene(&mut scene, width, height);

    // Text smoke test: UI + mono, exercising the shared text layer.
    let ink = Color::from_rgb8(0x1a, 0x1a, 0x1f);
    jjscratch::text::draw_text(&mut scene, &fonts.ui_bold, 15.0, ink, 110.0, 156.0, "jjscratch");
    jjscratch::text::draw_text(
        &mut scene, &fonts.mono, 13.0, Color::from_rgb8(0x40, 0x80, 0x40),
        110.0, 186.0, "+ added line of code");
    jjscratch::text::draw_text(
        &mut scene, &fonts.mono, 13.0, Color::from_rgb8(0xc0, 0x40, 0x40),
        110.0, 216.0, "- removed line of code");
    jjscratch::text::draw_text(
        &mut scene, &fonts.mono, 13.0, Color::from_rgb8(0xf5, 0x9e, 0x0b),
        110.0, 246.0, "@ wqnwktxr  working copy");

    // Background approximating lightjj's default light surface.
    let bg = Color::from_rgb8(0xf7, 0xf8, 0xfa);
    let img = hl.render(&scene, width, height, bg)?;
    img.save_png(&out)?;
    eprintln!("wrote {out} ({width}x{height})");
    Ok(())
}

fn smoke_scene(scene: &mut Scene, w: u32, h: u32) {
    let (w, h) = (w as f64, h as f64);

    // A rounded "panel" like the diff area.
    let panel = RoundedRect::new(40.0, 40.0, w - 40.0, h - 40.0, 8.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(0xff, 0xff, 0xff),
        None,
        &panel,
    );
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        Color::from_rgb8(0xd0, 0xd4, 0xda),
        None,
        &panel,
    );

    // A diff-add and diff-remove line swatch.
    let add = Rect::new(60.0, 70.0, w - 60.0, 92.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(0xe6, 0xff, 0xec),
        None,
        &add,
    );
    let rem = Rect::new(60.0, 96.0, w - 60.0, 118.0);
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        Color::from_rgb8(0xff, 0xeb, 0xe9),
        None,
        &rem,
    );

    // Graph nodes: mutable (palette), immutable (dim diamond stand-in), working-copy (amber ring).
    let palette = [
        Color::from_rgb8(0x4c, 0x6e, 0xf5),
        Color::from_rgb8(0x40, 0xc0, 0x57),
        Color::from_rgb8(0xf0, 0x8c, 0x00),
    ];
    for (i, c) in palette.iter().enumerate() {
        let cy = 150.0 + i as f64 * 30.0;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            *c,
            None,
            &Circle::new((80.0, cy), 5.0),
        );
    }
    // Working-copy amber concentric ring.
    let amber = Color::from_rgb8(0xf5, 0x9e, 0x0b);
    let cy = 150.0 + palette.len() as f64 * 30.0;
    scene.stroke(
        &Stroke::new(2.0),
        Affine::IDENTITY,
        amber,
        None,
        &Circle::new((80.0, cy), 6.0),
    );
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        amber,
        None,
        &Circle::new((80.0, cy), 2.5),
    );
}
