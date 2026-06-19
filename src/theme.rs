//! Theme = resolved colors + layout metrics + type scale, ported from lightjj's
//! `theme.css` / `themes.ts` (see docs/spec/ui-spec.md). The DEFAULT is DARK.
//!
//! Colors are stored as concrete `peniko::Color`. Translucent lightjj vars
//! (`color-mix(... p%, transparent)`) are pre-resolved over the relevant
//! background to an opaque color where that reads identically (row/selection
//! fills), and kept as true alpha colors (`*_a`) where they overlay arbitrary
//! content (diff word highlights, search). The spec notes which is which.

use vello::peniko::Color;

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}
/// Straight-alpha color (for fills that overlay arbitrary content).
const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color::from_rgba8(r, g, b, a)
}

/// Full color palette for one theme polarity.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    // Surfaces
    pub base: Color,
    pub mantle: Color,
    pub crust: Color,
    pub surface0: Color, // hover bg (resolved over base)
    pub surface1: Color, // default borders (resolved over base)
    pub surface2: Color, // strong borders/dividers ONLY (never text)
    // Text
    pub text: Color,
    pub subtext0: Color,
    pub subtext1: Color,
    pub overlay0: Color,
    pub overlay1: Color,
    pub text_faint: Color, // the only correct dim-text color
    // Accents / semantic
    pub amber: Color,
    pub green: Color,
    pub red: Color,
    pub blue: Color,
    pub mauve: Color,
    pub lavender: Color,
    // Graph lanes (drawn at node 0.8 / line 0.45 opacity by the renderer)
    pub graph: [Color; 8],
    // Syntax tokens
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_number: Color,
    pub syn_comment: Color,
    pub syn_type: Color,
    pub syn_property: Color,
    pub syn_operator: Color,
    pub syn_punct: Color,
    pub syn_atom: Color,
    // Row/selection backgrounds (opaque, resolved over base)
    pub bg_hover: Color,
    pub bg_selected: Color,
    pub bg_checked: Color,
    pub bg_checked_selected: Color,
    pub bg_active: Color, // active seg-btn (amber 14%)
    pub bg_error: Color,
    pub bg_warning: Color,
    // Diff fills (true alpha — overlay code text)
    pub diff_add_bg: Color,
    pub diff_remove_bg: Color,
    pub diff_add_word: Color,
    pub diff_remove_word: Color,
    pub bg_hunk_header: Color,
    // Selection inset bar / primary button hover
    pub btn_primary_hover: Color,
    /// Modal backdrop (over everything).
    pub backdrop: Color,
}

/// Default DARK palette (docs/spec/ui-spec.md §1).
pub const DARK: Palette = Palette {
    base: rgb(0x0f, 0x0f, 0x13),
    mantle: rgb(0x0f, 0x0f, 0x13),
    crust: rgb(0x0a, 0x0a, 0x0e),
    surface0: rgb(0x17, 0x17, 0x19),
    surface1: rgb(0x1e, 0x1e, 0x21),
    surface2: rgb(0x4e, 0x4e, 0x58),
    text: rgb(0xe2, 0xe2, 0xe6),
    subtext0: rgb(0x8a, 0x8a, 0x94),
    subtext1: rgb(0xe2, 0xe2, 0xe6),
    overlay0: rgb(0x8a, 0x8a, 0x94),
    overlay1: rgb(0x9a, 0x9a, 0xa4),
    text_faint: rgb(0x71, 0x71, 0x7a),
    amber: rgb(0xff, 0xa7, 0x26),
    green: rgb(0x66, 0xbb, 0x6a),
    red: rgb(0xef, 0x53, 0x50),
    blue: rgb(0x68, 0x80, 0xb8),
    mauve: rgb(0xc7, 0x92, 0xea),
    lavender: rgb(0xb4, 0xbe, 0xfe),
    graph: [
        rgb(0xBF, 0x8A, 0x30),
        rgb(0xB8, 0x68, 0x48),
        rgb(0xA8, 0x50, 0x6A),
        rgb(0x88, 0x68, 0xA8),
        rgb(0x68, 0x80, 0xB8),
        rgb(0x50, 0x98, 0xA0),
        rgb(0x5A, 0xA0, 0x58),
        rgb(0xA8, 0x98, 0x38),
    ],
    syn_keyword: rgb(0xc7, 0x92, 0xea),
    syn_string: rgb(0xa3, 0xbe, 0x8c),
    syn_number: rgb(0xf7, 0x8c, 0x6c),
    syn_comment: rgb(0x6c, 0x70, 0x86),
    syn_type: rgb(0xf9, 0xe2, 0xaf),
    syn_property: rgb(0x89, 0xb4, 0xfa),
    syn_operator: rgb(0x93, 0x99, 0xb2),
    syn_punct: rgb(0x6c, 0x70, 0x86),
    syn_atom: rgb(0xf7, 0x8c, 0x6c),
    bg_hover: rgb(0x17, 0x17, 0x19),
    bg_selected: rgb(0x24, 0x1c, 0x19),
    bg_checked: rgb(0x16, 0x22, 0x1a),
    bg_checked_selected: rgb(0x1a, 0x2a, 0x1d),
    bg_active: rgb(0x2c, 0x21, 0x1a),
    bg_error: rgb(0x26, 0x18, 0x1a),
    bg_warning: rgb(0x28, 0x20, 0x1a),
    diff_add_bg: rgba(0x66, 0xbb, 0x6a, 26),   // green @ ~10%
    diff_remove_bg: rgba(0xef, 0x53, 0x50, 26), // red @ ~10%
    diff_add_word: rgba(0x66, 0xbb, 0x6a, 56),  // green @ ~22%
    diff_remove_word: rgba(0xef, 0x53, 0x50, 56),
    bg_hunk_header: rgb(0x15, 0x15, 0x1a),
    btn_primary_hover: rgb(0xf0, 0xad, 0x44),
    backdrop: rgba(0, 0, 0, 128),
};

/// Default LIGHT palette ("Default Light", docs/spec/ui-spec.md §1).
///
/// Mirrors DARK field-for-field. Primaries come from lightjj's
/// `:root[data-theme="light"]` block (theme.css) / themes.ts "Default Light".
/// Derived row/selection/hunk backgrounds use the same `color-mix(... p%,
/// transparent/text)` formulas as DARK, re-resolved over the LIGHT base
/// (#f8f8f6) so they read as opaque tints. Diff word/add/remove stay true-alpha
/// (same alpha bytes as DARK) since they overlay arbitrary code.
pub const LIGHT: Palette = Palette {
    base: rgb(0xf8, 0xf8, 0xf6),
    mantle: rgb(0xf8, 0xf8, 0xf6),
    crust: rgb(0xee, 0xee, 0xec),
    surface0: rgb(0xef, 0xef, 0xed), // text 4% over base
    surface1: rgb(0xe8, 0xe8, 0xe7), // text 7% over base
    surface2: rgb(0xa1, 0xa1, 0xaa), // strong borders/dividers ONLY (never text)
    text: rgb(0x1a, 0x1a, 0x1e),
    subtext0: rgb(0x71, 0x71, 0x7a),
    subtext1: rgb(0x1a, 0x1a, 0x1e),
    overlay0: rgb(0x71, 0x71, 0x7a),
    overlay1: rgb(0x62, 0x62, 0x6a),
    text_faint: rgb(0x94, 0x94, 0x95), // text 45% over base (the only correct dim-text color)
    amber: rgb(0xe6, 0x8a, 0x00),
    green: rgb(0x2e, 0x7d, 0x32),
    red: rgb(0xc6, 0x28, 0x28),
    blue: rgb(0x48, 0x60, 0xa0),
    mauve: rgb(0x88, 0x39, 0xef),
    lavender: rgb(0x72, 0x87, 0xfd),
    graph: [
        rgb(0x9A, 0x6E, 0x18),
        rgb(0x98, 0x48, 0x30),
        rgb(0x88, 0x38, 0x58),
        rgb(0x6A, 0x48, 0x90),
        rgb(0x48, 0x60, 0xA0),
        rgb(0x38, 0x78, 0x80),
        rgb(0x3A, 0x80, 0x38),
        rgb(0x88, 0x78, 0x20),
    ],
    syn_keyword: rgb(0x88, 0x39, 0xef),
    syn_string: rgb(0x40, 0xa0, 0x2b),
    syn_number: rgb(0xfe, 0x64, 0x0b),
    syn_comment: rgb(0x9c, 0xa0, 0xb0),
    syn_type: rgb(0xdf, 0x8e, 0x1d),
    syn_property: rgb(0x1e, 0x66, 0xf5),
    syn_operator: rgb(0x6c, 0x6f, 0x85),
    syn_punct: rgb(0x9c, 0xa0, 0xb0),
    syn_atom: rgb(0xfe, 0x64, 0x0b),
    bg_hover: rgb(0xef, 0xef, 0xed), // = surface0
    bg_selected: rgb(0xf7, 0xf4, 0xec), // amber 4% over base (graph-row .selected, RevisionGraph.svelte)
    bg_checked: rgb(0xe8, 0xee, 0xe6), // green 8% over base
    bg_checked_selected: rgb(0xe0, 0xe9, 0xde), // green 12% over base
    bg_active: rgb(0xf5, 0xe9, 0xd4), // active seg-btn (amber 14% over base)
    bg_error: rgb(0xf3, 0xe3, 0xe1), // red 10% over base
    bg_warning: rgb(0xf6, 0xed, 0xdd), // amber 10% over base
    diff_add_bg: rgba(0x2e, 0x7d, 0x32, 26),    // green @ ~10%
    diff_remove_bg: rgba(0xc6, 0x28, 0x28, 26), // red @ ~10%
    diff_add_word: rgba(0x2e, 0x7d, 0x32, 56),  // green @ ~22%
    diff_remove_word: rgba(0xc6, 0x28, 0x28, 56),
    bg_hunk_header: rgb(0xf1, 0xf1, 0xf0), // text 3% over base
    btn_primary_hover: rgb(0xc7, 0x79, 0x05), // amber 85% + text 15%
    backdrop: rgba(0, 0, 0, 77),               // rgba(0,0,0,0.3) (light)
};

/// Graph node glyph opacity (renderer multiplies lane color alpha by this).
pub const GRAPH_NODE_OPACITY: f32 = 0.8;
/// Graph connector-line opacity.
pub const GRAPH_LINE_OPACITY: f32 = 0.45;
/// Immutable diamond opacity.
pub const GRAPH_IMMUTABLE_OPACITY: f32 = 0.5;

/// Layout metrics (px), border-box. docs/spec/ui-spec.md §3.
pub mod layout {
    pub const TOOLBAR_H: f64 = 34.0;
    pub const TAB_BAR_H: f64 = 26.0;
    pub const STATUSBAR_H: f64 = 24.0;
    pub const PANEL_HEADER_H: f64 = 34.0;
    pub const REVSET_BAR_H: f64 = 28.0; // padding 4 + input ~20

    pub const REVISION_PANEL_DEFAULT_W: f64 = 420.0;
    pub const REVISION_PANEL_MIN_W: f64 = 280.0;
    pub const REVISION_PANEL_MAX_W: f64 = 600.0;
    pub const PANEL_DIVIDER_W: f64 = 4.0;

    // Graph gutter (GraphSvg constants, §4.1)
    pub const ROW_H: f64 = 18.0;
    pub const CELL_W: f64 = 10.0; // per character column; lane = col/2 (20px lanes)
    pub const NODE_R: f64 = 4.0;
    pub const WC_R: f64 = 5.0;
    pub const GRAPH_LINE_W: f64 = 2.0;
    pub const NODE_GAP: f64 = 7.0;
    pub const CHECK_GUTTER_W: f64 = 14.0;
    pub const MAX_GUTTER_COLS: f64 = 12.0;

    // Diff
    pub const DIFF_LINE_H: f64 = 18.0;
}

/// Type scale (px) at the default 14px base. docs/spec/ui-spec.md §2.2.
pub mod font {
    pub const BASE: f32 = 14.0;
    pub const FS_3XS: f32 = 9.0;
    pub const FS_2XS: f32 = 10.0;
    pub const FS_XS: f32 = 11.0;
    pub const FS_SM: f32 = 12.0;
    pub const FS_MD: f32 = 13.0;
    pub const FS_LG: f32 = 15.0;
    pub const FS_XL: f32 = 17.0;
}

/// Apply an opacity multiplier to a color (for graph lane node/line dimming).
pub fn with_opacity(c: Color, opacity: f32) -> Color {
    c.multiply_alpha(opacity)
}
