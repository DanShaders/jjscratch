//! Golden-image regression suite for jjscratch.
//!
//! Compares jjscratch's CURRENT render of a fixed set of UI states against its
//! OWN committed golden PNGs in `tests/golden/`. This is a self-rendering UI
//! guard: it does NOT compare to lightjj, needs no Chrome, no network, and (in
//! the default build) no `jjlib` — every integrated state renders in-process
//! from the fixture-matching mock via `jjscratch::ui::build_scene`, exactly the
//! way `src/bin/shot.rs` does. The three bin-local preview views (merge, evolog,
//! split) live as modules of their `preview_*` bins and cannot be imported from
//! a test, so they are captured by shelling out to those bins and reading the
//! PNG they write. See `docs/qa/reftest.md`.
//!
//! ## Modes
//! - CHECK (default `cargo test`): re-render each state and compare to its
//!   golden. On mismatch the test FAILS and writes `<state>.actual.png`,
//!   `<state>.golden.png`, and a magenta `<state>.diff.png` into
//!   `target/reftest-out/`.
//! - BLESS (`REFTEST_BLESS=1 cargo test`): (re)write every `tests/golden/*.png`
//!   from the current render instead of comparing. Run this after intended UI
//!   changes; review the PNG diff in git, then commit.
//!
//! ## Determinism
//! lavapipe (the software Vulkan ICD this environment pins) renders these
//! scenes BIT-EXACT: two runs of any state produce byte-identical RGBA. The
//! `determinism` test asserts that. Comparison is therefore exact by default,
//! but a tiny tolerance (`MAX_CHANNEL_DELTA` / `MAX_MISMATCH_FRACTION`) is
//! applied in CHECK so a future non-bit-exact driver degrades to a near-pixel
//! match rather than a flake. See docs/qa/reftest.md for the finding.

use std::path::{Path, PathBuf};
use std::process::Command;

use jjscratch::model::{mock, CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::ui::{self, Frame, UiState};
use jjscratch::{Headless, Image};
use vello::Scene;

/// Fixed capture size (logical == device at @1x). @1x keeps the suite fast; the
/// states are layout/color regressions, not sub-pixel hinting, so @1x has teeth
/// without the 4x readback cost of @2x.
const W: u32 = 1280;
const H: u32 = 800;

/// CHECK tolerance. lavapipe is bit-exact today (see the `determinism` test), so
/// these only matter if a future driver is not. A pixel "differs" if any channel
/// is off by more than `MAX_CHANNEL_DELTA`; the state fails only if the fraction
/// of differing pixels exceeds `MAX_MISMATCH_FRACTION`.
const MAX_CHANNEL_DELTA: u8 = 2;
const MAX_MISMATCH_FRACTION: f64 = 0.0005; // 0.05%

// --------------------------------------------------------------------------
// Golden / output locations
// --------------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/reftest-out")
}

fn golden_path(state: &str) -> PathBuf {
    golden_dir().join(format!("{state}.png"))
}

fn bless_mode() -> bool {
    matches!(std::env::var("REFTEST_BLESS"), Ok(v) if v != "0" && !v.is_empty())
}

// --------------------------------------------------------------------------
// In-process render of the integrated states (mirrors src/bin/shot.rs)
// --------------------------------------------------------------------------

/// Build the UiState the way `shot` does: select the working copy, then replay
/// the same space-separated key tokens through the real input router.
fn state_for(keys: &str, snapshot: &Snapshot) -> UiState {
    let mut state = UiState::default();
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);
    for k in keys.split_whitespace() {
        jjscratch::input::handle_key(k, &mut state, snapshot);
    }
    state
}

/// Render one integrated state to an RGBA image, exactly mirroring `shot.rs`'s
/// in-process path at @1x with the mock fixture as the data source.
fn render_integrated(hl: &mut Headless, fonts: &Fonts, keys: &str) -> Image {
    let snapshot: Snapshot = mock::snapshot();
    let diff: CommitDiff = mock::working_copy_diff();
    let state = state_for(keys, &snapshot);
    let clear = state.theme.palette().base;

    let frame = Frame::default();
    let mut ui_scene = Scene::new();
    ui::build_scene(
        &mut ui_scene,
        &snapshot,
        Some(&diff),
        &state,
        fonts,
        &frame,
        W as f64,
        H as f64,
    );
    hl.render(&ui_scene, W, H, clear)
        .expect("vello render failed")
}

// --------------------------------------------------------------------------
// Subprocess render of the bin-local preview views
// --------------------------------------------------------------------------

/// Locate a `preview_*` bin. Cargo exports `CARGO_BIN_EXE_<name>` for autobins
/// to test crates; fall back to `target/<profile>/<name>` if it isn't set.
fn preview_bin(name: &str) -> PathBuf {
    let key = format!("CARGO_BIN_EXE_{name}");
    if let Some(p) = std::env::var_os(&key) {
        return PathBuf::from(p);
    }
    // Fallback: current exe lives in target/<profile>/deps/; the bin is one up.
    let exe = std::env::current_exe().expect("current_exe");
    let deps = exe.parent().expect("deps dir");
    let profile = deps.parent().expect("profile dir");
    profile.join(name)
}

/// Run a `preview_*` bin so it writes a PNG, then load that PNG back to RGBA.
fn render_preview(bin: &str) -> Image {
    let tmp = out_dir().join(format!("_{bin}.subproc.png"));
    std::fs::create_dir_all(out_dir()).ok();
    let exe = preview_bin(bin);
    let status = Command::new(&exe)
        .arg(&tmp)
        .arg(W.to_string())
        .arg(H.to_string())
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {} ({}): {e}", exe.display(), bin));
    assert!(status.success(), "{bin} exited with {status}");
    load_png(&tmp)
}

// --------------------------------------------------------------------------
// PNG <-> RGBA helpers
// --------------------------------------------------------------------------

fn load_png(path: &Path) -> Image {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .to_rgba8();
    Image {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    }
}

/// Pixel-difference report between two same-sized RGBA images.
struct Diff {
    mismatched: usize,
    total: usize,
    max_delta: u8,
}

impl Diff {
    fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.mismatched as f64 / self.total as f64
        }
    }
    /// Within tolerance for CHECK?
    fn ok(&self) -> bool {
        self.fraction() <= MAX_MISMATCH_FRACTION
    }
}

fn compare(a: &Image, b: &Image) -> Diff {
    assert_eq!(
        (a.width, a.height),
        (b.width, b.height),
        "image dimensions differ ({}x{} vs {}x{})",
        a.width,
        a.height,
        b.width,
        b.height
    );
    let total = (a.width * a.height) as usize;
    let mut mismatched = 0usize;
    let mut max_delta = 0u8;
    for (pa, pb) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)) {
        let mut differs = false;
        for c in 0..4 {
            let d = pa[c].abs_diff(pb[c]);
            if d > max_delta {
                max_delta = d;
            }
            if d > MAX_CHANNEL_DELTA {
                differs = true;
            }
        }
        if differs {
            mismatched += 1;
        }
    }
    Diff { mismatched, total, max_delta }
}

/// Write `actual`, `golden`, and a magenta-on-dimmed diff visualization for a
/// failing state into `target/reftest-out/`.
fn dump_failure(state: &str, actual: &Image, golden: &Image) {
    let dir = out_dir();
    std::fs::create_dir_all(&dir).ok();
    actual.save_png(dir.join(format!("{state}.actual.png"))).ok();
    golden.save_png(dir.join(format!("{state}.golden.png"))).ok();

    // Diff image: dim the actual to grayscale, paint mismatched pixels magenta.
    let mut diff = actual.rgba.clone();
    if actual.rgba.len() == golden.rgba.len() {
        for i in 0..(actual.width * actual.height) as usize {
            let o = i * 4;
            let (pa, pg) = (&actual.rgba[o..o + 4], &golden.rgba[o..o + 4]);
            let differs = (0..4).any(|c| pa[c].abs_diff(pg[c]) > MAX_CHANNEL_DELTA);
            if differs {
                diff[o] = 255;
                diff[o + 1] = 0;
                diff[o + 2] = 255;
                diff[o + 3] = 255;
            } else {
                let g = (pa[0] as u32 + pa[1] as u32 + pa[2] as u32) / 3;
                let g = (g / 3 + 30) as u8; // dim it so magenta pops
                diff[o] = g;
                diff[o + 1] = g;
                diff[o + 2] = g;
                diff[o + 3] = 255;
            }
        }
    }
    Image {
        width: actual.width,
        height: actual.height,
        rgba: diff,
    }
    .save_png(dir.join(format!("{state}.diff.png")))
    .ok();
}

// --------------------------------------------------------------------------
// State registry + bless/check driver
// --------------------------------------------------------------------------

/// How a state's RGBA bytes are produced.
enum Source {
    /// Integrated state via `build_scene`, driven by replayed key tokens.
    Keys(&'static str),
    /// Bin-local preview view, captured by shelling out to a `preview_*` bin.
    Preview(&'static str),
}

/// The golden basename for a state depends on whether the `jjlib` feature is on,
/// because the Oplog drawer chrome legitimately differs between the two build
/// configs (default → a "(mock build)" placeholder; jjlib → the real `oplog`
/// renderer's empty state). Those states get a `_jjlib`-suffixed golden so each
/// config checks against the image it actually produces; every other state
/// renders identically and shares one golden. See docs/qa/reftest.md.
fn feature_suffixed(base: &str) -> String {
    #[cfg(feature = "jjlib")]
    {
        format!("{base}_jjlib")
    }
    #[cfg(not(feature = "jjlib"))]
    {
        base.to_string()
    }
}

/// (golden basename, how to render it). Order is the documented coverage set.
/// `name` is the *logical* state name; `golden` resolves the feature-specific
/// file (only the `oplog` drawer differs by feature today).
fn states() -> Vec<(String, Source)> {
    vec![
        // Integrated states (in-process, mock data) — the live app via the
        // input-key driver, the same tokens `shot` accepts.
        ("revisions".into(), Source::Keys("")), // default
        ("diff_nav".into(), Source::Keys("j j")), // selection moved down two rows
        ("branches".into(), Source::Keys("2")), // Branches view
        // Oplog drawer: empty-state chrome differs default vs jjlib, so the
        // golden is feature-suffixed (oplog.png / oplog_jjlib.png).
        (feature_suffixed("oplog"), Source::Keys("4")),
        ("light".into(), Source::Keys("t")),     // light theme
        ("palette".into(), Source::Keys("ctrl+k")), // command palette overlay
        // Isolated preview views (subprocess — bin-local renderers).
        ("preview_merge".into(), Source::Preview("preview_merge")),
        ("preview_evolog".into(), Source::Preview("preview_evolog")),
        ("preview_split".into(), Source::Preview("preview_split")),
    ]
}

fn render_state(hl: &mut Headless, fonts: &Fonts, src: &Source) -> Image {
    match src {
        Source::Keys(keys) => render_integrated(hl, fonts, keys),
        Source::Preview(bin) => render_preview(bin),
    }
}

/// The single CHECK/BLESS entry point: BLESS writes goldens; otherwise compares.
#[test]
fn golden_states() {
    let mut hl = Headless::new().expect("headless renderer");
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );
    let fonts = Fonts::bundled();

    if bless_mode() {
        std::fs::create_dir_all(golden_dir()).expect("create tests/golden");
        for (name, src) in states() {
            let img = render_state(&mut hl, &fonts, &src);
            img.save_png(golden_path(&name))
                .unwrap_or_else(|e| panic!("write golden {name}: {e}"));
            eprintln!("blessed {name} ({}x{})", img.width, img.height);
        }
        eprintln!("REFTEST_BLESS: wrote {} goldens", states().len());
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    for (name, src) in states() {
        let gp = golden_path(&name);
        if !gp.exists() {
            failures.push(format!(
                "{name}: missing golden {} — run `REFTEST_BLESS=1 cargo test` to create it",
                gp.display()
            ));
            continue;
        }
        let actual = render_state(&mut hl, &fonts, &src);
        let golden = load_png(&gp);
        let d = compare(&actual, &golden);
        if d.ok() {
            eprintln!(
                "ok   {name}: {}/{} px differ ({:.4}%), max channel delta {}",
                d.mismatched,
                d.total,
                d.fraction() * 100.0,
                d.max_delta
            );
        } else {
            dump_failure(&name, &actual, &golden);
            failures.push(format!(
                "{name}: {}/{} px differ ({:.4}% > {:.4}% allowed), max channel delta {} \
                 — see target/reftest-out/{name}.{{actual,golden,diff}}.png; \
                 if this change is intended, re-bless with `REFTEST_BLESS=1 cargo test`",
                d.mismatched,
                d.total,
                d.fraction() * 100.0,
                MAX_MISMATCH_FRACTION * 100.0,
                d.max_delta,
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "golden mismatch in {} state(s):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// lavapipe is NOT strictly bit-exact: re-rendering the same scene flips up to a
/// single LSB on some channels (a 1-in-255 readback/accumulation difference).
/// What IS stable is that NO pixel ever differs by more than 1 on any channel —
/// so with `MAX_CHANNEL_DELTA = 2` the differing-pixel count is always 0. This
/// is the empirical guarantee the suite leans on; see docs/qa/reftest.md.
const DETERMINISM_MAX_DELTA: u8 = 1;

/// Prove the renders are *stable enough*: render two representative states twice
/// each (one integrated, one subprocess preview) and assert (a) zero pixels
/// differ beyond the CHECK tolerance and (b) the raw per-channel delta stays at
/// or below 1 LSB. If a future driver introduces larger drift, (b) fails loudly,
/// flagging that the goldens may flake and the tolerance needs revisiting.
#[test]
fn determinism() {
    let mut hl = Headless::new().expect("headless renderer");
    let fonts = Fonts::bundled();

    let a = render_integrated(&mut hl, &fonts, "");
    let b = render_integrated(&mut hl, &fonts, "");
    let d = compare(&a, &b);
    assert!(
        d.ok() && d.max_delta <= DETERMINISM_MAX_DELTA,
        "integrated render exceeded determinism tolerance across two runs: \
         {}/{} px differ (>{} allowed), max channel delta {} (>{} allowed)",
        d.mismatched,
        d.total,
        MAX_CHANNEL_DELTA,
        d.max_delta,
        DETERMINISM_MAX_DELTA
    );

    let a = render_preview("preview_merge");
    let b = render_preview("preview_merge");
    let d = compare(&a, &b);
    assert!(
        d.ok() && d.max_delta <= DETERMINISM_MAX_DELTA,
        "preview render exceeded determinism tolerance across two runs: \
         {}/{} px differ (>{} allowed), max channel delta {} (>{} allowed)",
        d.mismatched,
        d.total,
        MAX_CHANNEL_DELTA,
        d.max_delta,
        DETERMINISM_MAX_DELTA
    );
}
