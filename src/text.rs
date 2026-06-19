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
/// These are the real fonts from lightjj's CSS font stacks
/// (docs/spec/ui-spec.md §2.1):
///   --font-ui   = 'Inter', -apple-system, 'Segoe UI', sans-serif
///   --font-mono = 'JetBrains Mono', 'SF Mono', 'Fira Code', monospace
///
/// Bundled (static, hinted TTFs — NOT the variable fonts, so skrifa glyph
/// selection stays trivial):
///   - Inter            v4.1   (rsms/inter release v4.1, extras/ttf static instances)
///   - JetBrains Mono   v2.304 (JetBrains/JetBrainsMono release v2.304)
///
/// Weight mapping:
///   ui        -> Inter Regular          (400)
///   ui_bold   -> Inter SemiBold         (600)  -- lightjj's "bold" UI chrome is
///                overwhelmingly weight 600 (change-id badges, panel titles read
///                at 600; the few 700 spots look fine in 600). Inter Bold (700)
///                is also bundled (assets/fonts/Inter-Bold.ttf) if a future split
///                into a dedicated 700 field is wanted.
///   mono      -> JetBrains Mono Regular (400)
///   mono_bold -> JetBrains Mono Bold    (700)
///
/// Both families are SIL OFL 1.1 (see Inter-OFL.txt / JetBrainsMono-OFL.txt in
/// assets/fonts/). Swap the embedded bytes to tune fidelity without touching
/// call sites.
///
/// SYMBOL FALLBACK: Inter (UI) and JetBrains Mono (code) don't cover the Unicode
/// icon characters lightjj leans on (`⑂` U+2442 Branches, `⧉` U+29C9 Merge,
/// `⟲` U+27F2 Oplog, `◐` U+25D0 Evolog, plus geometric/technical shapes). In a
/// browser those resolve via system font fallback; here we have no system font
/// access, so we bundle our own fallback chain and resolve any char missing from
/// the requested font to the first fallback that maps it. See `fallbacks()`.
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
            ui: embed_font!("Inter-Regular.ttf"),
            ui_bold: embed_font!("Inter-SemiBold.ttf"),
            mono: embed_font!("JetBrainsMono-Regular.ttf"),
            mono_bold: embed_font!("JetBrainsMono-Bold.ttf"),
        }
    }
}

/// Symbol fallback chain, tried (in order) for any character the requested font
/// can't map. Both are official Noto fonts under SIL OFL 1.1 (see
/// assets/fonts/NotoSansSymbols-OFL.txt). Together they cover every icon glyph
/// the UI uses:
///
///   - Noto Sans Math      v3.000 (notofonts/math)    — 657440 bytes.
///       Covers ⧉ ⟲ ◐ ◉ ◇ ▾ ▪ ↗ ⊞ ⊟ ≡ ◫ ✓ ×  (14 of 16 listed glyphs).
///   - Noto Sans Symbols 2 v2.008 (notofonts/symbols) — 671568 bytes.
///       Covers the two Noto Sans Math lacks: ⑂ U+2442 and ⌘ U+2318.
///
/// Verified per-codepoint: every char missing from Inter/JetBrains Mono maps to
/// a non-zero glyph id somewhere in this chain. Loaded once and shared by both
/// `measure` and `draw_text`, so advances stay consistent.
///
/// (Statics rather than a `Fonts` field: callers hand us a single `&FontData`,
/// not the whole `Fonts`, so the chain has to be reachable from `draw_text`.)
fn fallbacks() -> &'static [FontData] {
    use std::sync::OnceLock;
    static FALLBACKS: OnceLock<Vec<FontData>> = OnceLock::new();
    FALLBACKS.get_or_init(|| {
        vec![
            embed_font!("NotoSansMath-Regular.ttf"),
            embed_font!("NotoSansSymbols2-Regular.ttf"),
        ]
    })
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

/// A font's shaping state for one text run: its charmap + advance-width metrics
/// at the requested size. Built once per font touched by the run.
struct FontShaper<'a> {
    charmap: Charmap<'a>,
    metrics: GlyphMetrics<'a>,
}

impl<'a> FontShaper<'a> {
    fn new(fr: &FontRef<'a>, size: f32) -> Self {
        Self {
            charmap: fr.charmap(),
            metrics: fr.glyph_metrics(Size::new(size), LocationRef::default()),
        }
    }
    /// Glyph id for `c` in this font, or `None` if unmapped (mapped to `.notdef`
    /// / glyph 0 counts as "not covered" so the fallback chain is consulted).
    fn glyph(&self, c: char) -> Option<skrifa::GlyphId> {
        match self.charmap.map(c) {
            Some(gid) if gid.to_u32() != 0 => Some(gid),
            _ => None,
        }
    }
    fn advance(&self, gid: skrifa::GlyphId) -> f32 {
        self.metrics.advance_width(gid).unwrap_or(0.0)
    }
}

/// Resolve every char in `text` to `(font_index, glyph_id)` where `font_index`
/// is `0` for the requested font and `1..=N` for the matching fallback in
/// `fallbacks()`. A char missing from every font falls back to the requested
/// font's `.notdef` (index 0, glyph 0) so spacing/`.notdef` boxes stay sane.
///
/// `shapers[0]` is the requested font; `shapers[1..]` are the fallbacks (same
/// order as `fallbacks()`), each built lazily-but-eagerly here once per run.
fn resolve<'a>(
    shapers: &[FontShaper<'a>],
    text: &str,
) -> Vec<(usize, skrifa::GlyphId, f32)> {
    let mut out = Vec::with_capacity(text.len());
    for c in text.chars() {
        // ASCII / common Latin almost always lives in shapers[0]; this hits the
        // fast path first and only walks the fallback chain on a miss.
        let mut placed = None;
        for (i, s) in shapers.iter().enumerate() {
            if let Some(gid) = s.glyph(c) {
                placed = Some((i, gid, s.advance(gid)));
                break;
            }
        }
        let (i, gid, adv) = placed.unwrap_or_else(|| {
            // Nothing covers it: draw the primary font's .notdef at its advance.
            let gid = shapers[0].charmap.map(c).unwrap_or_default();
            (0, gid, shapers[0].advance(gid))
        });
        out.push((i, gid, adv));
    }
    out
}

/// Build the shaper list `[requested font, ...fallbacks]` for one run, keeping
/// each `FontRef` alive alongside its shaper. Returns `None` if the requested
/// font fails to parse.
fn shapers_for<'a>(
    primary: &'a FontData,
    fallback_fonts: &'a [FontData],
    size: f32,
) -> Option<(Vec<&'a FontData>, Vec<FontShaper<'a>>)> {
    let mut fonts: Vec<&FontData> = Vec::with_capacity(1 + fallback_fonts.len());
    let mut shapers: Vec<FontShaper> = Vec::with_capacity(1 + fallback_fonts.len());
    let pr = font_ref(primary)?;
    shapers.push(FontShaper::new(&pr, size));
    fonts.push(primary);
    for f in fallback_fonts {
        if let Some(fr) = font_ref(f) {
            shapers.push(FontShaper::new(&fr, size));
            fonts.push(f);
        }
    }
    Some((fonts, shapers))
}

/// Width in pixels of `text` rendered in `font` at `size` (advance sum).
///
/// Advances come from whichever font in `[font, ...fallbacks()]` actually draws
/// each glyph, so `measure` and `draw_text` agree character-for-character.
pub fn measure(font: &FontData, size: f32, text: &str) -> f32 {
    let fb = fallbacks();
    let Some((_, shapers)) = shapers_for(font, fb, size) else { return 0.0 };
    resolve(&shapers, text).iter().map(|(_, _, adv)| *adv).sum()
}

/// Draw `text` into `scene` with its baseline at (`x`, `baseline_y`) in scene
/// coordinates. Returns the advanced x position (x + run width) for chaining.
///
/// Characters missing from `font` are drawn from the bundled symbol fallback
/// chain. Vello's `draw_glyphs` is per-font, so we group consecutive glyphs that
/// share a font into segments and emit one `draw_glyphs` call per segment, with
/// each glyph positioned at the running pen advanced in its own font's metrics.
pub fn draw_text(
    scene: &mut Scene,
    font: &FontData,
    size: f32,
    color: Color,
    x: f64,
    baseline_y: f64,
    text: &str,
) -> f64 {
    let fb = fallbacks();
    let Some((fonts, shapers)) = shapers_for(font, fb, size) else { return x };
    let resolved = resolve(&shapers, text);

    let mut pen = 0.0f32;
    let mut i = 0;
    while i < resolved.len() {
        let seg_font = resolved[i].0;
        // Collect this run of same-font glyphs, positioning each at the running
        // pen and advancing pen by that glyph's own-font advance.
        let mut seg_glyphs: Vec<Glyph> = Vec::new();
        while i < resolved.len() && resolved[i].0 == seg_font {
            let (_, gid, adv) = resolved[i];
            seg_glyphs.push(Glyph { id: gid.to_u32(), x: pen, y: 0.0 });
            pen += adv;
            i += 1;
        }
        scene
            .draw_glyphs(fonts[seg_font])
            .font_size(size)
            .brush(color)
            .transform(Affine::translate((x, baseline_y)))
            .draw(Fill::NonZero, seg_glyphs.into_iter());
    }

    x + pen as f64
}
