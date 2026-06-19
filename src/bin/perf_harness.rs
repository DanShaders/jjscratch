//! `perf-harness` — generate and measure the reactivity-critical scenarios, and
//! report p50/p99 end-to-end latency with a per-stage breakdown (repo reload vs
//! snapshot/revset/graph vs diff materialize vs `ui::build_scene`).
//!
//! Scenarios (each built in a throwaway repo via the absolute-path `jj` + file
//! writes):
//!   * `tiny`      — touch one file + one `jj` op (the common case).
//!   * `huge-diff` — a working-copy commit changing a very large file + hundreds
//!                   of files; measures the diff-materialize + scene cost.
//!   * `large-log` — hundreds–thousands of commits; measures snapshot reload +
//!                   scene build over a big graph.
//!
//! Plus a `windowing` micro-benchmark that compares `build_scene` for a viewport
//! height vs a "tall" height that forces all rows/lines to be built, quantifying
//! how much build_scene time is the OFF-SCREEN portion (the case for O(viewport)
//! windowing in the render files).
//!
//! Usage:
//! ```text
//! cargo run --release --features jjlib --bin perf-harness            # all
//! cargo run --release --features jjlib --bin perf-harness -- tiny    # one
//! ```
//! Honors `JJ_BIN` to override the jj binary (default: the absolute tools path).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use jjscratch::model::{CommitDiff, Snapshot};
use jjscratch::text::Fonts;
use jjscratch::ui::{self, Frame, UiState};
use jjscratch::watch::{ReactiveReloader, Timings};
use vello::Scene;

const ITERS: usize = 25;

fn main() -> Result<()> {
    let which: Vec<String> = std::env::args().skip(1).collect();
    let run = |name: &str| which.is_empty() || which.iter().any(|w| w == name);

    let fonts = Fonts::bundled();

    println!("perf-harness — jjscratch reactivity benchmarks");
    println!("jj binary: {}", jj_bin().display());
    println!("iterations per measured op: {ITERS}\n");

    if run("tiny") {
        scenario_tiny(&fonts)?;
    }
    if run("huge-diff") {
        scenario_huge_diff(&fonts)?;
    }
    if run("large-log") {
        scenario_large_log(&fonts)?;
    }
    if run("windowing") {
        scenario_windowing(&fonts)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// Tiny change: a one-file edit + one describe op. Repeatedly mutate + reload.
fn scenario_tiny(fonts: &Fonts) -> Result<()> {
    let repo = TempRepo::init("tiny")?;
    repo.write("a.txt", "hello\n")?;
    repo.jj(&["commit", "-m", "seed"])?;

    let mut reloader = ReactiveReloader::open(repo.path())?;
    let mut samples: Vec<Timings> = Vec::with_capacity(ITERS);

    for i in 0..ITERS {
        // One small change each iteration: rewrite the working-copy file +
        // describe the wc, which moves the op log.
        repo.write("a.txt", &format!("hello {i}\n"))?;
        repo.jj(&["describe", "-m", &format!("edit {i}")])?;

        let out = reloader.reload_forced()?;
        let scene_build = time_scene_build(fonts, &out.snapshot, out.diff.as_deref());
        let mut t = out.timings;
        t.scene_build = scene_build;
        samples.push(t);
    }

    report("tiny change (1 file, 1 op)", &samples);
    Ok(())
}

/// Huge diff: a working-copy commit that changes a very large file AND hundreds
/// of files. Measures diff-materialize + scene over a big diff.
fn scenario_huge_diff(fonts: &Fonts) -> Result<()> {
    let repo = TempRepo::init("huge-diff")?;
    // Seed: a large file and many small files in the PARENT commit.
    repo.write("big.txt", &big_text(20_000, 0))?;
    for k in 0..400 {
        repo.write(&format!("f{k:03}.txt", k = k), &format!("base {k}\n"))?;
    }
    repo.jj(&["commit", "-m", "seed huge"])?;

    let mut reloader = ReactiveReloader::open(repo.path())?;
    let mut samples: Vec<Timings> = Vec::with_capacity(ITERS);

    for i in 0..ITERS {
        // Working-copy now diverges hugely from its parent: rewrite the big file
        // (every other line changed) and touch all 400 small files.
        repo.write("big.txt", &big_text(20_000, i + 1))?;
        for k in 0..400 {
            repo.write(&format!("f{k:03}.txt", k = k), &format!("changed {k} iter {i}\n"))?;
        }
        // Snapshot the working copy into the op log (no describe needed: the
        // wc-snapshot op moves the head). `jj status` triggers a snapshot.
        repo.jj(&["status"])?;

        let out = reloader.reload_forced()?;
        let scene_build = time_scene_build(fonts, &out.snapshot, out.diff.as_deref());
        let mut t = out.timings;
        t.scene_build = scene_build;
        samples.push(t);
    }

    let nfiles = reloader
        .loaded()
        .wc_commit_id_hex()
        .and_then(|_| None::<usize>)
        .unwrap_or(0);
    let _ = nfiles;
    // Report the diff size for context.
    if let Some(d) = last_diff(&mut reloader)? {
        println!(
            "  (huge-diff: {} files, {} added / {} removed lines)",
            d.files.len(),
            d.total_added(),
            d.total_removed()
        );
    }
    report("huge diff (large file + 400 files)", &samples);
    Ok(())
}

/// Large log: generate many commits, then measure snapshot reload + scene build
/// over the resulting big graph.
fn scenario_large_log(fonts: &Fonts) -> Result<()> {
    let repo = TempRepo::init("large-log")?;
    let n = std::env::var("PERF_LOG_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000usize);

    eprintln!("  generating {n} commits (this is the slow setup, not measured)...");
    repo.write("log.txt", "0\n")?;
    repo.jj(&["commit", "-m", "c0"])?;
    // Batch commits: each `jj commit` is an op. To make a long linear history
    // cheaply, write + commit in a loop. (jj has no bulk-commit; this is setup.)
    for i in 1..n {
        repo.append("log.txt", &format!("{i}\n"))?;
        repo.jj(&["commit", "-m", &format!("c{i}")])?;
    }

    // Use a revset that actually surfaces the whole history so the graph is big.
    // (The default-log revset elides immutable ancestors; for this benchmark we
    // want the full chain. We set every commit mutable by not marking trunk, so
    // the default revset still walks the wc ancestors — but to be safe we also
    // report the snapshot size the reloader produces.)
    let mut reloader = ReactiveReloader::open(repo.path())?;
    let mut samples: Vec<Timings> = Vec::with_capacity(ITERS);

    for i in 0..ITERS {
        // Move the head a little each iteration so reload actually re-resolves.
        repo.append("log.txt", &format!("touch {i}\n"))?;
        repo.jj(&["describe", "-m", &format!("c{} touch {i}", n + i)])?;

        let out = reloader.reload_forced()?;
        let scene_build = time_scene_build(fonts, &out.snapshot, out.diff.as_deref());
        let mut t = out.timings;
        t.scene_build = scene_build;
        samples.push(t);
    }

    if let Some(snap) = peek_snapshot(&mut reloader)? {
        println!("  (large-log: snapshot has {} visible rows)", snap.revision_count());
    }
    report(&format!("large log ({n} commits)"), &samples);
    Ok(())
}

/// Windowing micro-benchmark: time `build_scene` at a normal viewport height vs
/// a "tall" height that forces the renderer to lay out every row/line, to
/// quantify the off-screen build cost. Uses the large-log + huge-diff repo so
/// there is plenty off-screen.
fn scenario_windowing(fonts: &Fonts) -> Result<()> {
    println!("\n== windowing analysis (build_scene: viewport vs full) ==");

    // Build a sizeable snapshot + diff to render.
    let repo = TempRepo::init("windowing")?;
    repo.write("big.txt", &big_text(5_000, 0))?;
    repo.jj(&["commit", "-m", "seed"])?;
    repo.write("big.txt", &big_text(5_000, 1))?;
    for k in 0..120 {
        repo.write(&format!("w{k:03}.txt", k = k), &format!("x {k}\n"))?;
    }
    repo.jj(&["status"])?;
    for i in 0..200 {
        repo.append("big.txt", &format!("h{i}\n"))?;
        repo.jj(&["commit", "-m", &format!("h{i}")])?;
    }

    let mut reloader = ReactiveReloader::open(repo.path())?;
    let out = reloader.reload_forced()?;
    let snapshot = &out.snapshot;
    let diff = out.diff.as_deref();

    println!(
        "  scene inputs: {} graph rows, {} diff files ({} lines)",
        snapshot.revision_count(),
        diff.map(|d| d.files.len()).unwrap_or(0),
        diff.map(|d| d.files.iter().map(|f| f.hunks.iter().map(|h| h.lines.len()).sum::<usize>()).sum::<usize>())
            .unwrap_or(0),
    );

    // Viewport: a typical window. "Full": tall enough that every row + diff line
    // would be on-screen, forcing the renderer to build all of them. The delta
    // is the off-screen build cost that O(viewport) windowing would eliminate.
    let viewport_h = 800.0;
    let full_h = (snapshot.revision_count() as f64 * 18.0)
        .max(diff.map(|d| total_diff_lines(d) as f64 * 18.0).unwrap_or(0.0))
        + 400.0;

    let vp = bench(|| {
        let mut sc = Scene::new();
        build(&mut sc, snapshot, diff, fonts, 1280.0, viewport_h);
    });
    let full = bench(|| {
        let mut sc = Scene::new();
        build(&mut sc, snapshot, diff, fonts, 1280.0, full_h);
    });

    println!("  build_scene @ viewport ({viewport_h:.0}px): p50 {:>7.3}ms", ms(vp.0));
    println!("  build_scene @ full     ({full_h:.0}px): p50 {:>7.3}ms", ms(full.0));
    let off = full.0.saturating_sub(vp.0);
    let pct = if full.0.as_secs_f64() > 0.0 {
        off.as_secs_f64() / full.0.as_secs_f64() * 100.0
    } else {
        0.0
    };
    println!(
        "  => off-screen portion ~{:.3}ms ({pct:.0}% of full build) — eliminable by O(viewport) windowing",
        ms(off)
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Scene build timing
// ---------------------------------------------------------------------------

/// Time a single `build_scene` over the given snapshot+diff at a default frame.
fn time_scene_build(fonts: &Fonts, snapshot: &Snapshot, diff: Option<&CommitDiff>) -> Duration {
    let mut scene = Scene::new();
    let t = Instant::now();
    build(&mut scene, snapshot, diff, fonts, 1280.0, 800.0);
    t.elapsed()
}

/// Replicates shot.rs's minimal `build_scene` setup (default UiState with the
/// working-copy selected) — read-only; does NOT touch shot.rs.
fn build(scene: &mut Scene, snapshot: &Snapshot, diff: Option<&CommitDiff>, fonts: &Fonts, w: f64, h: f64) {
    let mut state = UiState::default();
    state.selected = snapshot
        .nodes
        .iter()
        .position(|n| n.is_working_copy)
        .unwrap_or(0);
    let frame = Frame {
        #[cfg(feature = "jjlib")]
        oplog: &[],
        ..Default::default()
    };
    ui::build_scene(scene, snapshot, diff, &state, fonts, &frame, w, h);
}

fn total_diff_lines(d: &CommitDiff) -> usize {
    d.files
        .iter()
        .map(|f| f.hunks.iter().map(|h| h.lines.len()).sum::<usize>())
        .sum()
}

// ---------------------------------------------------------------------------
// Stats / reporting
// ---------------------------------------------------------------------------

fn report(title: &str, samples: &[Timings]) {
    println!("\n== {title} ==");
    let total: Vec<Duration> = samples.iter().map(|t| t.total()).collect();
    let reload: Vec<Duration> = samples.iter().map(|t| t.reload).collect();
    let snap: Vec<Duration> = samples.iter().map(|t| t.snapshot).collect();
    let diff: Vec<Duration> = samples.iter().map(|t| t.diff).collect();
    let scene: Vec<Duration> = samples.iter().map(|t| t.scene_build).collect();

    println!(
        "  stage           p50        p99        mean       (n={})",
        samples.len()
    );
    line("end-to-end", &total);
    line("  reload", &reload);
    line("  snapshot", &snap);
    line("  diff", &diff);
    line("  build_scene", &scene);

    let frame = Duration::from_micros(16_600);
    let p99 = pct(&total, 99.0);
    let verdict = if p99 <= frame { "WITHIN" } else { "OVER" };
    println!(
        "  budget: p99 end-to-end {} the 16.6ms frame ({:.1}x)",
        verdict,
        p99.as_secs_f64() / frame.as_secs_f64()
    );
}

fn line(name: &str, v: &[Duration]) {
    println!(
        "  {:<14} {:>8.3}ms {:>8.3}ms {:>8.3}ms",
        name,
        ms(pct(v, 50.0)),
        ms(pct(v, 99.0)),
        ms(mean(v)),
    );
}

fn bench<F: FnMut()>(mut f: F) -> (Duration, Vec<Duration>) {
    let mut v = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let t = Instant::now();
        f();
        v.push(t.elapsed());
    }
    (pct(&v, 50.0), v)
}

fn pct(v: &[Duration], p: f64) -> Duration {
    if v.is_empty() {
        return Duration::ZERO;
    }
    let mut s = v.to_vec();
    s.sort();
    let idx = ((p / 100.0) * (s.len() as f64 - 1.0)).round() as usize;
    s[idx.min(s.len() - 1)]
}

fn mean(v: &[Duration]) -> Duration {
    if v.is_empty() {
        return Duration::ZERO;
    }
    let sum: Duration = v.iter().sum();
    sum / v.len() as u32
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

// ---------------------------------------------------------------------------
// Reloader peeking helpers (so we can print sizes without re-timing)
// ---------------------------------------------------------------------------

fn last_diff(reloader: &mut ReactiveReloader) -> Result<Option<CommitDiff>> {
    let out = reloader.reload_forced()?;
    Ok(out.diff.map(|a| (*a).clone()))
}

fn peek_snapshot(reloader: &mut ReactiveReloader) -> Result<Option<Snapshot>> {
    let out = reloader.reload_forced()?;
    Ok(Some(out.snapshot))
}

// ---------------------------------------------------------------------------
// Test-repo scaffolding (absolute-path jj + file writes)
// ---------------------------------------------------------------------------

fn big_text(lines: usize, salt: usize) -> String {
    let mut s = String::with_capacity(lines * 24);
    for i in 0..lines {
        // Change every other line per-salt so the diff is large but realistic.
        if i % 2 == 0 {
            s.push_str(&format!("line {i} static content here\n"));
        } else {
            s.push_str(&format!("line {i} salt {salt} mutable content\n"));
        }
    }
    s
}

fn jj_bin() -> PathBuf {
    std::env::var_os("JJ_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/danklishch/work/jjscratch/tools/bin/jj"))
}

/// A throwaway jj repo built under the system temp dir, removed on drop.
struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn init(tag: &str) -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "jjscratch-perf-{tag}-{}-{}",
            std::process::id(),
            now_nanos()
        ));
        std::fs::create_dir_all(&dir)?;
        let me = Self { dir };
        // Non-colocated jj repo (only `.jj/`), matching the fixture style.
        me.jj(&["git", "init", "--colocate=false"])
            .or_else(|_| me.jj(&["git", "init"]))
            .context("jj git init")?;
        // Deterministic identity so commits don't depend on host config.
        Ok(me)
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn write(&self, rel: &str, content: &str) -> Result<()> {
        let p = self.dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, content)?;
        Ok(())
    }

    fn append(&self, rel: &str, content: &str) -> Result<()> {
        use std::io::Write as _;
        let p = self.dir.join(rel);
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
        f.write_all(content.as_bytes())?;
        Ok(())
    }

    fn jj(&self, args: &[&str]) -> Result<()> {
        let out = Command::new(jj_bin())
            .current_dir(&self.dir)
            .args(args)
            // Quiet, deterministic, no pager/editor.
            .env("JJ_CONFIG", "/dev/null")
            .env("JJ_USER", "perf")
            .env("JJ_EMAIL", "perf@example.com")
            .env("EDITOR", "true")
            .output()
            .with_context(|| format!("running jj {args:?}"))?;
        if !out.status.success() {
            return Err(anyhow!(
                "jj {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}
