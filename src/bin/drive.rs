//! `drive` — the jjscratch side of the cross-driver INTERACTION harness.
//!
//! It replays a shared interaction SCRIPT against the native jjscratch UI: it
//! loads a real jj repo (via the `jjlib` feature, `JJSCRATCH_REPO`, exactly like
//! `shot.rs`), seeds the cursor on the working-copy node (matching lightjj's
//! default), then steps through the script applying keys through the SAME pure
//! [`input::handle_key`] router the app would use, and at each `shot` step
//! renders the CURRENT state to a numbered `step-NN-<name>.png`.
//!
//! Crucially the diff panel FOLLOWS the selection: at every shot we recompute the
//! diff for the currently-selected commit (`jjlib::commit_diff`) and feed it to
//! [`ui::build_scene`], so navigating with j/k changes the shown diff just like
//! lightjj.
//!
//! ## Interaction-script format (shared with the lightjj driver)
//!
//! A plain text file, one token per line:
//! - `key <k>`     — send a single key to the input router (`j`, `k`, `g`, `G`,
//!                   `1`, `2`, `3`, or browser names like `ArrowDown`).
//! - `shot <name>` — capture the current frame as `step-NN-<name>.png`.
//! - blank lines and lines beginning with `#` are ignored.
//!
//! ## Usage
//!
//! ```text
//! JJSCRATCH_REPO=$PWD/fixture/repo \
//!   cargo run --features jjlib --bin drive -- <script> <out-dir> [width] [height] [--scale N]
//! ```
//!
//! `width`/`height` are LOGICAL pixels (default 1280x800). Default scale is 2
//! (@2x), matching the lightjj reference shots (2560x1600), so each `step-NN`
//! PNG is directly comparable to the lightjj driver's output.

use anyhow::{bail, Result};

#[cfg(feature = "jjlib")]
fn run() -> Result<()> {
    use anyhow::Context;
    use jjscratch::input;
    use jjscratch::model::jjlib;
    use jjscratch::text::Fonts;
    use jjscratch::ui::{self, Frame, UiState};
    use jjscratch::Headless;
    use vello::kurbo::Affine;
    use vello::Scene;

    // ---- args --------------------------------------------------------------
    let mut scale: u32 = 2;
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
    let script_path = positional
        .next()
        .context("usage: drive <script> <out-dir> [width] [height] [--scale N]")?;
    let out_dir = positional.next().unwrap_or_else(|| "docs/parity/interaction".to_string());
    let width: u32 = positional.next().and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = positional.next().and_then(|s| s.parse().ok()).unwrap_or(800);
    let scale = scale.max(1);

    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("creating out dir {out_dir}"))?;

    // ---- load the real repo (same path as shot.rs) -------------------------
    let repo_path = std::env::var_os("JJSCRATCH_REPO")
        .map(std::path::PathBuf::from)
        .context("JJSCRATCH_REPO must be set (the jjlib drive binary needs a real repo)")?;
    eprintln!("loading real repo: {}", repo_path.display());
    let loaded = jjlib::open(&repo_path)?;
    let snapshot = jjlib::snapshot(&loaded)?;
    // Load the op log once so an open Oplog drawer (the `4` key) shows real ops.
    let oplog = jjlib::oplog(&loaded, 30).unwrap_or_default();
    eprintln!(
        "loaded {} revisions from {} (workspace {}), {} ops",
        snapshot.revision_count(),
        snapshot.repo_name,
        snapshot.workspace_name,
        oplog.len(),
    );

    // ---- seed cursor on the working copy (matching lightjj's default) ------
    let mut state = UiState::default();
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);

    // ---- render setup ------------------------------------------------------
    let mut hl = Headless::new()?;
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );
    let fonts = Fonts::bundled();

    // Render the CURRENT state to step-NN-<name>.png, computing the diff for the
    // selected commit so the diff panel follows the cursor.
    let render_shot = |hl: &mut Headless,
                       state: &UiState,
                       idx: usize,
                       name: &str|
     -> Result<()> {
        let commit_id = &snapshot.nodes[state.selected].commit_id;
        let diff = jjlib::commit_diff(&loaded, commit_id)
            .with_context(|| format!("diffing selected commit {commit_id}"))?;
        // Clear color follows the active theme (the `t` key flips it).
        let clear = state.theme.palette().base;
        let frame = Frame {
            oplog: &oplog,
            ..Default::default()
        };

        let mut ui_scene = Scene::new();
        ui::build_scene(
            &mut ui_scene,
            &snapshot,
            Some(&diff),
            state,
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
        let out = format!("{out_dir}/step-{idx:02}-{name}.png");
        img.save_png(&out)?;
        eprintln!(
            "  shot #{idx:02} {name:<12} selected={} commit={} -> {out} ({dev_w}x{dev_h})",
            state.selected,
            &commit_id[..commit_id.len().min(8)],
        );
        Ok(())
    };

    // ---- replay the script -------------------------------------------------
    let script = std::fs::read_to_string(&script_path)
        .with_context(|| format!("reading script {script_path}"))?;
    let mut shot_idx = 0usize;
    for (lineno, raw) in script.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap();
        let arg = parts.next();
        match cmd {
            "key" => {
                let k = arg.with_context(|| format!("line {}: `key` needs an argument", lineno + 1))?;
                let changed = input::handle_key(k, &mut state, &snapshot);
                eprintln!("  key {k:<10} -> selected={} (changed={changed})", state.selected);
            }
            "shot" => {
                let name = arg.unwrap_or("frame");
                render_shot(&mut hl, &state, shot_idx, name)?;
                shot_idx += 1;
            }
            other => bail!("line {}: unknown command `{other}`", lineno + 1),
        }
    }

    if shot_idx == 0 {
        bail!("script produced no `shot` steps");
    }
    eprintln!("drive: captured {shot_idx} step(s) into {out_dir}");
    Ok(())
}

#[cfg(not(feature = "jjlib"))]
fn run() -> Result<()> {
    bail!("the `drive` binary requires the `jjlib` feature: cargo run --features jjlib --bin drive");
}

fn main() -> Result<()> {
    run()
}
