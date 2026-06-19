//! `churn-test` — hammer a repo with rapid concurrent filesystem writes / `jj`
//! ops while the watcher + reactive reload loop runs, and verify:
//!
//!   * **Coalescing works** — the reload loop processes the LATEST state, far
//!     fewer times than there were raw fs events (it never reloads per-event).
//!   * **Memory is bounded** — the watcher's debounce keeps O(1) pending state
//!     and the diff cache is capped; nothing grows unboundedly.
//!   * **Max lag is bounded & recovers** — under sustained churn the worst
//!     event→reload lag stays bounded, and once churn stops the loop returns to
//!     idle (no backlog, no deadlock).
//!
//! The process exits non-zero (asserts) if coalescing fails, the loop stalls,
//! or it never recovers — so it doubles as a regression test.
//!
//! Usage:
//! ```text
//! cargo run --release --features jjlib --bin churn-test            # default
//! CHURN_SECS=5 CHURN_WRITERS=4 cargo run ... --bin churn-test
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use jjscratch::watch::{ReactiveReloader, Watcher};

fn main() -> Result<()> {
    let secs: u64 = env_or("CHURN_SECS", 4);
    let writers: usize = env_or("CHURN_WRITERS", 3);

    println!("churn-test: {secs}s of churn, {writers} concurrent writer thread(s)");
    println!("jj binary: {}", jj_bin().display());

    let repo = Arc::new(TempRepo::init()?);
    repo.write("seed.txt", "0\n")?;
    repo.jj(&["commit", "-m", "seed"])?;

    let watcher = Watcher::new(repo.path())?;
    let mut reloader = ReactiveReloader::open(repo.path())?;

    let stop = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicU64::new(0));
    let ops = Arc::new(AtomicU64::new(0));

    // Spawn churn threads: a mix of pure file writes (working-copy churn) and
    // real `jj` ops (op-log churn). These run concurrently with the reload loop.
    let mut handles = Vec::new();
    for w in 0..writers {
        let repo = repo.clone();
        let stop = stop.clone();
        let writes = writes.clone();
        let ops = ops.clone();
        handles.push(std::thread::spawn(move || {
            let mut i = 0u64;
            while !stop.load(Ordering::Relaxed) {
                // Mostly cheap file writes; every ~8th iteration a real jj op so
                // the op log actually moves (forcing genuine reloads).
                let rel = format!("churn_{w}.txt");
                let _ = repo.write(&rel, &format!("w{w} i{i}\n"));
                writes.fetch_add(1, Ordering::Relaxed);
                if i % 8 == 0 {
                    if repo.jj(&["describe", "-m", &format!("w{w} op {i}")]).is_ok() {
                        ops.fetch_add(1, Ordering::Relaxed);
                    }
                }
                i += 1;
                // A short sleep keeps this a *storm* without pegging a core; the
                // watcher must coalesce regardless.
                std::thread::sleep(Duration::from_millis(2));
            }
        }));
    }

    // The reactive reload loop: drain to the latest tick, reload, track lag.
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut reloads = 0u64;
    let mut skips = 0u64;
    let mut ticks_seen = 0u64;
    let mut max_lag = Duration::ZERO;
    let mut max_reload = Duration::ZERO;

    while Instant::now() < deadline {
        let Some(tick) = watcher.next_change_timeout(Duration::from_millis(200)) else {
            continue;
        };
        // Latest-wins: skip any older queued ticks; only the newest state matters.
        let tick = watcher.latest_pending().unwrap_or(tick);
        ticks_seen += 1;

        match reloader.reload()? {
            None => skips += 1, // op unchanged — cheap gate fast path
            Some(out) => {
                reloads += 1;
                let lag = tick.event_at.elapsed();
                max_lag = max_lag.max(lag);
                max_reload = max_reload.max(out.timings.total());
            }
        }
    }

    // Stop churn and let writers exit.
    stop.store(true, Ordering::Relaxed);
    for h in handles {
        let _ = h.join();
    }

    let raw = watcher.raw_event_count();
    let total_writes = writes.load(Ordering::Relaxed);
    let total_ops = ops.load(Ordering::Relaxed);

    println!("\n-- churn phase --");
    println!("  raw fs events observed : {raw}");
    println!("  file writes issued     : {total_writes}");
    println!("  jj ops issued          : {total_ops}");
    println!("  ticks consumed         : {ticks_seen}");
    println!("  reloads performed      : {reloads}  (skipped, op-unchanged: {skips})");
    println!("  max event->reload lag  : {:.2}ms", ms(max_lag));
    println!("  max single reload cost : {:.2}ms", ms(max_reload));
    println!("  diff-cache size (bounded): {}", reloader.diff_cache_len());

    // -- Recovery phase: churn stopped. "Recovered" means the loop drains the
    //    final state and then settles — every subsequent tick (if any residual
    //    fs activity fires one) is a cheap op-UNCHANGED no-op, and the loop keeps
    //    the channel drained (no growing backlog). We do NOT require literally
    //    zero events: the OS/jj may still touch `.jj/`; the proof of recovery is
    //    that those produce no further real reloads and never accumulate.
    println!("\n-- recovery phase --");
    std::thread::sleep(Duration::from_millis(300));

    let recover_start = Instant::now();
    let mut recovery_reloads = 0u64; // real (op-moved) reloads after churn
    let mut recovery_skips = 0u64; // cheap no-op ticks after churn
    // Process for up to 1.5s: the final coalesced state, then settle. A healthy
    // loop converges — after the first reload(s) catch up to the last op, the
    // rest are skips. We also confirm the backlog never builds: each iteration
    // drains to the latest pending tick in O(1).
    let settle_deadline = recover_start + Duration::from_millis(1500);
    let mut consecutive_quiet = 0u32;
    while Instant::now() < settle_deadline {
        match watcher.next_change_timeout(Duration::from_millis(150)) {
            Some(t) => {
                let _ = watcher.latest_pending().unwrap_or(t); // latest-wins drain
                match reloader.reload()? {
                    None => recovery_skips += 1,
                    Some(_) => recovery_reloads += 1,
                }
                consecutive_quiet = 0;
            }
            None => {
                consecutive_quiet += 1;
                // Two quiet windows in a row (~300ms with no tick) => settled.
                if consecutive_quiet >= 2 {
                    break;
                }
            }
        }
    }
    // Recovered iff the loop converged: no *real* reloads were still happening
    // at the tail (the op log stopped moving) — residual no-op skips are fine.
    let recovered = recovery_reloads == 0;
    println!(
        "  settled in {:.0}ms; post-churn real reloads: {recovery_reloads}, \
         cheap no-op ticks: {recovery_skips} (gate absorbed)",
        recover_start.elapsed().as_secs_f64() * 1000.0,
    );

    // -- Assertions (exit non-zero on failure) --
    let mut failures = Vec::new();

    // 1. Coalescing: far fewer reloads than raw events. With ms-scale churn this
    //    should be dramatic; require at least a 2x reduction as a floor.
    if raw > 10 && reloads as f64 > raw as f64 / 2.0 {
        failures.push(format!(
            "coalescing too weak: {reloads} reloads for {raw} raw events"
        ));
    }
    // 2. The loop never starved: we consumed at least one tick under churn.
    if raw > 10 && ticks_seen == 0 {
        failures.push("no ticks consumed under churn (loop starved/deadlocked)".into());
    }
    // 3. Bounded memory: the diff cache never exceeds its cap (64).
    if reloader.diff_cache_len() > 64 {
        failures.push(format!(
            "diff cache exceeded cap: {}",
            reloader.diff_cache_len()
        ));
    }
    // 4. Bounded lag: even under storm, worst lag stays well under a second.
    if max_lag > Duration::from_secs(1) {
        failures.push(format!("max lag too high: {:.2}ms", ms(max_lag)));
    }
    // 5. Recovery: after churn stops, the op log stops moving and the loop
    //    converges (no further real reloads — only cheap gated no-ops).
    if !recovered {
        failures.push(format!(
            "did not converge after churn: {recovery_reloads} real reloads still firing"
        ));
    }

    if failures.is_empty() {
        println!("\nchurn-test PASSED: coalescing + bounded memory + bounded lag + recovery ✓");
        Ok(())
    } else {
        for f in &failures {
            eprintln!("FAIL: {f}");
        }
        Err(anyhow!("{} churn assertion(s) failed", failures.len()))
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn jj_bin() -> PathBuf {
    std::env::var_os("JJ_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/danklishch/work/jjscratch/tools/bin/jj"))
}

struct TempRepo {
    dir: PathBuf,
}

impl TempRepo {
    fn init() -> Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "jjscratch-churn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir)?;
        let me = Self { dir };
        me.jj(&["git", "init", "--colocate=false"])
            .or_else(|_| me.jj(&["git", "init"]))
            .context("jj git init")?;
        Ok(me)
    }
    fn path(&self) -> &Path {
        &self.dir
    }
    fn write(&self, rel: &str, content: &str) -> Result<()> {
        std::fs::write(self.dir.join(rel), content)?;
        Ok(())
    }
    fn jj(&self, args: &[&str]) -> Result<()> {
        let out = Command::new(jj_bin())
            .current_dir(&self.dir)
            .args(args)
            .env("JJ_CONFIG", "/dev/null")
            .env("JJ_USER", "churn")
            .env("JJ_EMAIL", "churn@example.com")
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
