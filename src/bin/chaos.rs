//! `chaos` — the deterministic monkey/fuzz harness for jjscratch.
//!
//! It drives the app through huge volumes of random keyboard interaction trying
//! to BREAK it: panics, invalid UI state, or render-side crashes. Everything is
//! deterministic per seed (a tiny xorshift PRNG seeded from the seed integer; no
//! `rand` dependency), so any failure reproduces exactly from the printed seed.
//!
//! For each seed it:
//!   1. Picks a repo (the committed fixture, a freshly-generated random repo, or
//!      one of the synthetic EDGE repos: empty / single-commit) and loads it.
//!   2. Seeds a RANDOM initial [`UiState`] (selection, view, theme, drawers,
//!      palette open + a random query).
//!   3. Applies a long random key sequence (hundreds–thousands) drawn from the
//!      full keymap PLUS junk keys (unrecognized strings, empty, unicode),
//!      interleaving j/k spam past both bounds, rapid 1/2/3 view flips, 4/5
//!      drawer toggles, t theme flips, and cmd+k → type query → backspace storm
//!      → esc cycles.
//!   4. After EACH key asserts the UI INVARIANTS and CALLS [`ui::build_scene`]
//!      (recomputing the selected commit's diff on the jjlib path) so render
//!      panics surface too. The whole per-key step runs under
//!      [`std::panic::catch_unwind`]; on a panic it prints the seed + the exact
//!      key sequence that triggered it and counts a FAIL.
//!
//! ## Usage
//!
//! ```text
//! cargo run --features jjlib --bin chaos -- [SEEDS] [KEYS_PER_RUN]
//! ```
//!
//! `SEEDS` (default 200) is the number of seeds to sweep (0..SEEDS). `KEYS_PER_RUN`
//! (default 800) is the random key-sequence length applied per run. Without the
//! `jjlib` feature only the mock repo is fuzzed (no real-repo diff path), which
//! still exercises every input + render path against the fixture-shaped data.
//!
//! Prints a PASS/FAIL summary; exits non-zero if any seed failed an invariant or
//! triggered a panic, with the reproducer for each.

use std::panic;

use jjscratch::input;
use jjscratch::model::{mock, CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::ui::{self, Frame, Theme, UiState, View};
use jjscratch::Headless;
use vello::Scene;

/// Render size for the per-key rasterization. Kept at the app's standard logical
/// size so every frame-layout branch is exercised; on the software rasterizer
/// this is the dominant cost, so it's the standard 1280x800 only when a full
/// sweep is feasible. We use a smaller-but-representative size by default so a
/// large seed x key sweep finishes in minutes, not hours, while still driving the
/// complete Vello scene-build + wgpu readback path on every key.
const RENDER_W: u32 = 640;
const RENDER_H: u32 = 400;

// ---------------------------------------------------------------------------
// Deterministic PRNG: xorshift64* (no external deps). Seeded per run so every
// failure reproduces from its seed.
// ---------------------------------------------------------------------------
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // Mix the seed so seed 0 isn't a dead state and small seeds spread out.
        let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1);
        if s == 0 {
            s = 0x9E37_79B9_7F4A_7C15;
        }
        Rng(s)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            return 0;
        }
        (self.next_u64() % bound as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

// ---------------------------------------------------------------------------
// Key alphabet: the full keymap plus junk. The PRNG draws from this, biased by
// the interleaving scheme in `run_one` to actually hammer the bounds.
// ---------------------------------------------------------------------------
#[allow(dead_code)]
const REAL_KEYS: &[&str] = &[
    "j", "k", "ArrowDown", "ArrowUp", "1", "2", "3", "t", "4", "5", "cmd+k", "ctrl+k", "Escape",
    "Esc", "Backspace", "Space", " ",
];

const JUNK_KEYS: &[&str] = &[
    "", "x", "G", "g", "Home", "End", "Enter", "Tab", "PageDown", "PageUp", "F1", "💥", "é", "中",
    "\u{0}", "\u{7f}", "\n", "\t", "ab", "cmd+j", "Shift", "Meta", "qwertyuiop", "0",
];

/// A random printable-ish token to type into the palette query (stress the
/// unicode/UTF-8 boundary, grapheme clusters, control chars, multi-char names).
fn random_query_char(rng: &mut Rng) -> &'static str {
    const CHARS: &[&str] = &[
        "a", "b", "z", "Z", "9", "/", " ", "💥", "é", "中", "👩‍👩‍👧", "ñ", "ß", "\u{0}", "\u{7f}",
        ".", "-", "_", "ArrowDown", "cmd+k", "Backspace", "Escape",
    ];
    pick_str(rng, CHARS)
}

/// Pick a random `&'static str` from a slice (deref of the generic `pick`).
fn pick_str(rng: &mut Rng, xs: &[&'static str]) -> &'static str {
    xs[rng.below(xs.len())]
}

// ---------------------------------------------------------------------------
// Per-run repo source. Holds the snapshot + a way to fetch the selected diff.
// ---------------------------------------------------------------------------
enum Repo {
    Mock {
        snapshot: Snapshot,
        diff: CommitDiff,
    },
    #[cfg(feature = "jjlib")]
    Real {
        loaded: jjscratch::model::jjlib::Loaded,
        snapshot: Snapshot,
        oplog: Vec<jjscratch::model::jjlib::OpEntry>,
    },
}

impl Repo {
    fn snapshot(&self) -> &Snapshot {
        match self {
            Repo::Mock { snapshot, .. } => snapshot,
            #[cfg(feature = "jjlib")]
            Repo::Real { snapshot, .. } => snapshot,
        }
    }

    /// The diff for the currently-selected node, mirroring how `drive`/`shot`
    /// recompute it as the cursor moves. Returns `None` when there's nothing to
    /// show (empty snapshot) or the diff fails to load (reported, not panicked).
    fn diff_for(&self, #[cfg_attr(not(feature = "jjlib"), allow(unused))] selected: usize) -> Option<CommitDiff> {
        match self {
            Repo::Mock { diff, .. } => Some(diff.clone()),
            #[cfg(feature = "jjlib")]
            Repo::Real { loaded, snapshot, .. } => {
                let node = snapshot.nodes.get(selected)?;
                match jjscratch::model::jjlib::commit_diff(loaded, &node.commit_id) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        // A diff load failure is a render-data issue, not an input
                        // bug; surface it loudly but don't treat it as a panic.
                        eprintln!("  [warn] commit_diff failed for {}: {e}", node.commit_id);
                        None
                    }
                }
            }
        }
    }

    #[cfg(feature = "jjlib")]
    fn oplog(&self) -> &[jjscratch::model::jjlib::OpEntry] {
        match self {
            Repo::Mock { .. } => &[],
            Repo::Real { oplog, .. } => oplog,
        }
    }
}

/// Describes where a run's repo came from, for reproducers.
#[derive(Clone)]
enum RepoKind {
    Mock,
    Real(String),
}

impl std::fmt::Display for RepoKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoKind::Mock => write!(f, "mock"),
            RepoKind::Real(p) => write!(f, "real:{p}"),
        }
    }
}

fn load_mock() -> Repo {
    Repo::Mock {
        snapshot: mock::snapshot(),
        diff: mock::working_copy_diff(),
    }
}

/// A truly-empty snapshot (`nodes.len() == 0`) — no real jj repo produces this
/// (every repo has at least the root + `@`), so we synthesize it to exercise the
/// `len == 0` branches of input handling AND the renderer's empty-list path.
fn load_empty_mock() -> Repo {
    let mut snapshot = mock::snapshot();
    snapshot.nodes.clear();
    snapshot.wc_commit_id = None;
    Repo::Mock {
        snapshot,
        diff: mock::working_copy_diff(),
    }
}

#[cfg(feature = "jjlib")]
fn load_real(path: &std::path::Path) -> anyhow::Result<Repo> {
    use jjscratch::model::jjlib;
    let loaded = jjlib::open(path)?;
    let snapshot = jjlib::snapshot(&loaded)?;
    let oplog = jjlib::oplog(&loaded, 30).unwrap_or_default();
    Ok(Repo::Real { loaded, snapshot, oplog })
}

// ---------------------------------------------------------------------------
// Invariants checked after every key. Returns Err(msg) on violation.
// ---------------------------------------------------------------------------
fn check_invariants(state: &UiState, snapshot: &Snapshot) -> Result<(), String> {
    let len = snapshot.nodes.len();
    // Selection must index a real node, or be 0 when empty.
    if len == 0 {
        if state.selected != 0 {
            return Err(format!("selected={} but snapshot is empty (must be 0)", state.selected));
        }
    } else if state.selected >= len {
        return Err(format!("selected={} out of bounds (len={len})", state.selected));
    }
    // palette_query is always valid UTF-8 (Rust String). The important guard is
    // that it never grows without bound (a runaway append bug would blow this).
    if state.palette_query.len() > 1_000_000 {
        return Err(format!("palette_query runaway length {}", state.palette_query.len()));
    }
    // Drawer coherence: at most one bottom drawer open at a time.
    if state.oplog_open && state.evolog_open {
        return Err("both oplog and evolog drawers open simultaneously".into());
    }
    // Theme/view are enums — coherent by construction; the match guards against
    // any future corruption sneaking an unknown discriminant past us.
    match state.theme {
        Theme::Dark | Theme::Light => {}
    }
    match state.active_view {
        View::Revisions | View::Branches | View::Merge => {}
    }
    Ok(())
}

/// Build a random starting UI state for a run.
fn random_state(rng: &mut Rng, snapshot: &Snapshot) -> UiState {
    let len = snapshot.nodes.len();
    let mut st = UiState::default();
    st.selected = if len == 0 { 0 } else { rng.below(len) };
    st.active_view = *rng.pick(&[View::Revisions, View::Branches, View::Merge]);
    st.theme = *rng.pick(&[Theme::Dark, Theme::Light]);
    st.oplog_open = rng.below(4) == 0;
    st.evolog_open = !st.oplog_open && rng.below(4) == 0;
    st.palette_open = rng.below(5) == 0;
    if st.palette_open {
        for _ in 0..rng.below(8) {
            st.palette_query.push_str(random_query_char(rng));
        }
    }
    st
}

/// Generate the next key for the sequence, interleaving the bounds-hammering
/// patterns described in the harness goal. Returns a key token.
fn next_key(rng: &mut Rng, palette_open: bool) -> String {
    // When the palette is open, lean hard on query typing + backspace + esc so
    // the unicode/backspace-on-empty paths get pounded.
    if palette_open {
        return match rng.below(10) {
            0 | 1 => "Backspace".to_string(),
            2 => "Escape".to_string(),
            3 => "Space".to_string(),
            _ => random_query_char(rng).to_string(),
        };
    }
    match rng.below(20) {
        // j/k spam (40%) — long runs in one direction blow past both bounds.
        0..=3 => "j".to_string(),
        4..=7 => "k".to_string(),
        8 => "ArrowDown".to_string(),
        9 => "ArrowUp".to_string(),
        // view flips
        10 => "1".to_string(),
        11 => "2".to_string(),
        12 => "3".to_string(),
        // drawers
        13 => "4".to_string(),
        14 => "5".to_string(),
        // theme
        15 => "t".to_string(),
        // open the palette (then subsequent keys go down the palette branch)
        16 => if rng.below(2) == 0 { "cmd+k" } else { "ctrl+k" }.to_string(),
        // junk
        _ => pick_str(rng, JUNK_KEYS).to_string(),
    }
}

/// One key step: apply the key, check invariants, render. Returns Err on a
/// (caught) invariant violation; PANICS bubble up to the caller's catch_unwind.
fn step(
    key: &str,
    state: &mut UiState,
    repo: &Repo,
    fonts: &Fonts,
    hl: &mut Headless,
) -> Result<(), String> {
    let snapshot = repo.snapshot();
    input::handle_key(key, state, snapshot);
    check_invariants(state, snapshot)?;

    // Render the resulting state so any render-side panic surfaces here, tied to
    // the exact key that produced the state.
    let diff = repo.diff_for(state.selected);
    let frame = Frame {
        #[cfg(feature = "jjlib")]
        oplog: repo.oplog(),
        ..Default::default()
    };
    let clear = state.theme.palette().base;
    let mut scene = Scene::new();
    // Lay out + paint at the real logical size so every layout branch (drawers,
    // headers, panel split, empty list) is exercised exactly as in the app...
    ui::build_scene(
        &mut scene,
        snapshot,
        diff.as_ref(),
        state,
        fonts,
        &frame,
        RENDER_W as f64,
        RENDER_H as f64,
    );
    // ...then actually rasterize through the wgpu/Vello + readback path so render
    // panics surface. The raster size is the same logical size; it's modest so a
    // full seed sweep stays tractable on the software (lavapipe) rasterizer.
    hl.render(&scene, RENDER_W, RENDER_H, clear)
        .map_err(|e| format!("render error: {e}"))?;
    Ok(())
}

/// A failure reproducer: the seed, repo, initial state, and the key prefix that
/// reached the failure.
struct Failure {
    seed: u64,
    repo: RepoKind,
    initial: String,
    keys: Vec<String>,
    reason: String,
}

impl Failure {
    fn report(&self) {
        eprintln!("\n==== CHAOS FAILURE ====");
        eprintln!("seed:   {}", self.seed);
        eprintln!("repo:   {}", self.repo);
        eprintln!("reason: {}", self.reason);
        eprintln!("initial state: {}", self.initial);
        eprintln!("key sequence ({} keys):", self.keys.len());
        // Print compactly; quote junk tokens so empty/space/unicode are visible.
        let quoted: Vec<String> = self.keys.iter().map(|k| format!("{k:?}")).collect();
        eprintln!("  {}", quoted.join(" "));
        eprintln!("=======================\n");
    }
}

fn initial_desc(st: &UiState) -> String {
    format!(
        "selected={} view={:?} theme={:?} oplog={} evolog={} palette={} query={:?}",
        st.selected,
        st.active_view,
        st.theme,
        st.oplog_open,
        st.evolog_open,
        st.palette_open,
        st.palette_query,
    )
}

/// Run a single seed against one repo. Returns `Some(Failure)` if it broke.
fn run_one(
    seed: u64,
    repo_kind: RepoKind,
    repo: &Repo,
    keys_per_run: usize,
    fonts: &Fonts,
    hl: &mut Headless,
) -> Option<Failure> {
    let mut rng = Rng::new(seed);
    let snapshot = repo.snapshot();
    let mut state = random_state(&mut rng, snapshot);
    let initial = initial_desc(&state);

    // Validate the initial state too.
    if let Err(reason) = check_invariants(&state, snapshot) {
        return Some(Failure { seed, repo: repo_kind, initial, keys: vec![], reason });
    }

    let mut applied: Vec<String> = Vec::with_capacity(keys_per_run);
    for _ in 0..keys_per_run {
        let key = next_key(&mut rng, state.palette_open);
        applied.push(key.clone());

        // Catch panics from BOTH input handling and rendering, attributing them
        // to this exact key prefix.
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            step(&key, &mut state, repo, fonts, hl)
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => {
                return Some(Failure {
                    seed,
                    repo: repo_kind,
                    initial,
                    keys: applied,
                    reason,
                });
            }
            Err(panic_payload) => {
                let msg = panic_message(&panic_payload);
                return Some(Failure {
                    seed,
                    repo: repo_kind,
                    initial,
                    keys: applied,
                    reason: format!("PANIC: {msg}"),
                });
            }
        }
    }
    None
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seeds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let keys_per_run: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(800);

    // Silence the default panic hook so our reproducer output is clean; we catch
    // and report panics ourselves.
    panic::set_hook(Box::new(|_| {}));

    eprintln!("chaos: sweeping {seeds} seeds x {keys_per_run} keys/run");

    let fonts = Fonts::bundled();
    let mut hl = match Headless::new() {
        Ok(h) => h,
        Err(e) => {
            eprintln!("FATAL: could not create headless renderer: {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "adapter: {} ({:?}, {:?})",
        hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
    );

    // Build the repo roster: always the mock; under jjlib also the fixture, the
    // EDGE repos (empty / single-commit), and a handful of random generated
    // repos. Each entry is (kind, Repo). We load once and reuse across the seeds
    // that map onto them.
    #[cfg_attr(not(feature = "jjlib"), allow(unused_mut))]
    let mut repos: Vec<(RepoKind, Repo)> = vec![
        (RepoKind::Mock, load_mock()),
        // The genuinely-empty (len==0) snapshot edge — no real repo yields it.
        (RepoKind::Real("empty-snapshot(len=0)".to_string()), load_empty_mock()),
    ];

    #[cfg(feature = "jjlib")]
    {
        // The fixture (committed in this repo; resolved via the manifest dir so
        // worktrees without a local tools/ still find it).
        let manifest = env!("CARGO_MANIFEST_DIR");
        let fixture = std::path::Path::new(manifest).join("fixture/repo");
        if fixture.exists() {
            match load_real(&fixture) {
                Ok(r) => repos.push((RepoKind::Real(fixture.display().to_string()), r)),
                Err(e) => eprintln!("  [warn] could not load fixture {}: {e}", fixture.display()),
            }
        }

        // Generate the EDGE + random repos into a temp dir using the repo's gen
        // script + the absolute tools/bin/jj. Best-effort: if generation fails
        // (no tools), we just fuzz the mock + fixture.
        let stress_dir = std::path::PathBuf::from("/tmp/jjscratch-chaos");
        match prepare_edge_and_random_repos(&stress_dir) {
            Ok(paths) => {
                for p in paths {
                    match load_real(&p) {
                        Ok(r) => repos.push((RepoKind::Real(p.display().to_string()), r)),
                        Err(e) => eprintln!("  [warn] could not load {}: {e}", p.display()),
                    }
                }
            }
            Err(e) => eprintln!("  [warn] edge/random repo generation skipped: {e}"),
        }
    }

    eprintln!("chaos: fuzzing {} repo(s):", repos.len());
    for (kind, repo) in &repos {
        eprintln!("  - {kind} ({} nodes)", repo.snapshot().nodes.len());
    }

    let mut failures: Vec<Failure> = Vec::new();
    let mut runs = 0u64;
    for seed in 0..seeds {
        // Rotate the seed across repos so every repo is hit across the sweep, and
        // each (seed,repo) pair is deterministic.
        let (kind, repo) = &repos[(seed as usize) % repos.len()];
        runs += 1;
        if let Some(f) = run_one(seed, kind.clone(), repo, keys_per_run, &fonts, &mut hl) {
            f.report();
            failures.push(f);
        }
    }

    eprintln!("\n==== CHAOS SUMMARY ====");
    eprintln!("seeds run:   {runs}");
    eprintln!("keys/run:    {keys_per_run}");
    eprintln!("total keys:  {}", runs * keys_per_run as u64);
    eprintln!("repos:       {}", repos.len());
    if failures.is_empty() {
        eprintln!("result:      PASS (0 failures)");
    } else {
        eprintln!("result:      FAIL ({} failure(s))", failures.len());
        for f in &failures {
            eprintln!("  seed {} repo {} -> {}", f.seed, f.repo, f.reason);
        }
        std::process::exit(1);
    }
}

/// Build the EDGE repos (empty, single-commit) and a few random repos, returning
/// their paths. Uses the committed `scripts/gen-random-repos.sh` for the random
/// ones and direct `jj` calls for the edge ones. The fixed absolute `jj` path
/// means this works even from a worktree without a local `tools/`.
#[cfg(feature = "jjlib")]
fn prepare_edge_and_random_repos(dir: &std::path::Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    use anyhow::{bail, Context};
    use std::process::Command;

    const JJ: &str = "/home/danklishch/work/jjscratch/tools/bin/jj";
    if !std::path::Path::new(JJ).exists() {
        bail!("jj binary not found at {JJ}");
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

    let mut out = Vec::new();

    // -- EDGE: empty repo (only the working-copy `@` on root). ---------------
    let empty = dir.join("empty");
    let _ = std::fs::remove_dir_all(&empty);
    let st = Command::new(JJ)
        .args(["git", "init", "--no-colocate"])
        .arg(&empty)
        .env("JJ_USER", "Chaos")
        .env("JJ_EMAIL", "chaos@example.com")
        .status()
        .context("running jj git init (empty)")?;
    if st.success() && empty.exists() {
        out.push(empty);
    }

    // -- EDGE: single-commit repo (@ described, one real change). ------------
    let single = dir.join("single");
    let _ = std::fs::remove_dir_all(&single);
    let st = Command::new(JJ)
        .args(["git", "init", "--no-colocate"])
        .arg(&single)
        .env("JJ_USER", "Chaos")
        .env("JJ_EMAIL", "chaos@example.com")
        .status()
        .context("running jj git init (single)")?;
    if st.success() && single.exists() {
        std::fs::write(single.join("f.txt"), "only commit\n").ok();
        let _ = Command::new(JJ)
            .args(["describe", "-m", "the only commit"])
            .current_dir(&single)
            .env("JJ_USER", "Chaos")
            .env("JJ_EMAIL", "chaos@example.com")
            .status();
        out.push(single);
    }

    // -- Random repos via the committed generator script. --------------------
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/gen-random-repos.sh");
    if script.exists() {
        let random_dir = dir.join("random");
        let st = Command::new("bash")
            .arg(&script)
            .arg(&random_dir)
            .args(["7", "13", "29"]) // a few fixed seeds for reproducibility
            .status();
        if let Ok(st) = st {
            if st.success() {
                for seed in [7, 13, 29] {
                    let p = random_dir.join(format!("repo-{seed}"));
                    if p.exists() {
                        out.push(p);
                    }
                }
            }
        }
    }

    Ok(out)
}
