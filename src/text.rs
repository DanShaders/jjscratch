//! Shared text layer: bundled fonts + glyph layout + drawing into a Vello scene.
//!
//! This is the ONE text helper the whole UI builds on — graph rows, diff lines,
//! toolbar labels, badges. Don't hand-roll glyph placement elsewhere; extend
//! this module instead.
//!
//! Layout here is deliberately simple (monospace-friendly horizontal advance,
//! no shaping/bidi/kerning). lightjj's UI is overwhelmingly Latin + monospace
//! code, so per-glyph advance accumulation matches it closely. If we later need
//! complex shaping for prose, swap in `parley` behind this same API.

use skrifa::charmap::Charmap;
use skrifa::instance::{LocationRef, Size};
use skrifa::metrics::{GlyphMetrics, Metrics};
use skrifa::{FontRef, MetadataProvider};
use vello::kurbo::Affine;
use vello::peniko::{Blob, Color, FontData, Fill};
use vello::{Glyph, Scene};

/// Bundled fonts. Self-contained (embedded in the binary) so worktrees and the
/// headless harness need no system font access.
///
/// DejaVu Sans / Sans Mono are stand-ins for lightjj's CSS font stacks
/// (system-ui / ui-monospace). Swap the embedded bytes to tune fidelity without
/// touching call sites.
pub struct Fonts {
    pub ui: FontData,
    pub ui_bold: FontData,
    pub mono: FontData,
    pub mono_bold: FontData,
}

macro_rules! embed_font {
    ($path:literal) => {
        FontData::new(Blob::new(std::sync::Arc::new(
            *include_bytes!(concat!("../assets/fonts/", $path)),
        )), 0)
    };
}

impl Fonts {
    pub fn bundled() -> Self {
        Self {
            ui: embed_font!("DejaVuSans.ttf"),
            ui_bold: embed_font!("DejaVuSans-Bold.ttf"),
            mono: embed_font!("DejaVuSansMono.ttf"),
            mono_bold: embed_font!("DejaVuSansMono-Bold.ttf"),
        }
    }
}

/// Vertical line metrics for a font at a given pixel size.
#[derive(Clone, Copy, Debug)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

impl LineMetrics {
    /// Total line height (ascent + |descent| + leading).
    pub fn line_height(&self) -> f32 {
        self.ascent + self.descent.abs() + self.leading
    }
}

fn font_ref(font: &FontData) -> Option<FontRef<'_>> {
    FontRef::from_index(font.data.as_ref(), font.index)
        .ok()
        .or_else(|| FontRef::new(font.data.as_ref()).ok())
}

/// Line metrics for `font` at `size` pixels.
pub fn line_metrics(font: &FontData, size: f32) -> LineMetrics {
    let Some(fr) = font_ref(font) else {
        return LineMetrics { ascent: size * 0.8, descent: size * 0.2, leading: 0.0 };
    };
    let m: Metrics = fr.metrics(Size::new(size), LocationRef::default());
    LineMetrics { ascent: m.ascent, descent: m.descent, leading: m.leading }
}

/// Width in pixels of `text` rendered in `font` at `size` (advance sum).
pub fn measure(font: &FontData, size: f32, text: &str) -> f32 {
    let Some(fr) = font_ref(font) else { return 0.0 };
    let charmap = fr.charmap();
    let gm: GlyphMetrics = fr.glyph_metrics(Size::new(size), LocationRef::default());
    let mut w = 0.0f32;
    for c in text.chars() {
        let gid = charmap.map(c).unwrap_or_default();
        w += gm.advance_width(gid).unwrap_or(0.0);
    }
    w
}

/// Draw `text` into `scene` with its baseline at (`x`, `baseline_y`) in scene
/// coordinates. Returns the advanced x position (x + run width) for chaining.
pub fn draw_text(
    scene: &mut Scene,
    font: &FontData,
    size: f32,
    color: Color,
    x: f64,
    baseline_y: f64,
    text: &str,
) -> f64 {
    let Some(fr) = font_ref(font) else { return x };
    let charmap: Charmap = fr.charmap();
    let gm: GlyphMetrics = fr.glyph_metrics(Size::new(size), LocationRef::default());

    let mut pen = 0.0f32;
    let glyphs: Vec<Glyph> = text
        .chars()
        .map(|c| {
            let gid = charmap.map(c).unwrap_or_default();
            let gx = pen;
            pen += gm.advance_width(gid).unwrap_or(0.0);
            Glyph { id: gid.to_u32(), x: gx, y: 0.0 }
        })
        .collect();

    scene
        .draw_glyphs(font)
        .font_size(size)
        .brush(color)
        .transform(Affine::translate((x, baseline_y)))
        .draw(Fill::NonZero, glyphs.into_iter());

    x + pen as f64
}
