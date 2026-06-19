//! `watch-demo` — tail a jj repo's `.jj/` and, on every detected change, reload
//! the [`Snapshot`](jjscratch::model::Snapshot) + the selected commit's diff,
//! logging the end-to-end reactive latency (fs event → new state ready) and a
//! one-line "what changed".
//!
//! Usage:
//! ```text
//! cargo run --features jjlib --bin watch-demo -- [repo_path]
//! ```
//! Defaults to the committed fixture. Run a `jj` op against the repo in another
//! terminal (e.g. `jj describe -m hi`) and watch a tick land here.
//!
//! Ctrl-C to stop.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use jjscratch::watch::{ReactiveReloader, Watcher};

fn main() -> Result<()> {
    let repo: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_fixture);

    eprintln!("watch-demo: watching {}", repo.display());

    let watcher = Watcher::new(&repo)?;
    let mut reloader = ReactiveReloader::open(&repo)?;

    // Prime an initial render so the first real change shows a meaningful delta.
    if let Some(out) = reloader.reload()? {
        eprintln!(
            "initial: {} revisions, op {}, diff files {}",
            out.snapshot.revision_count(),
            &out.op_id[..out.op_id.len().min(12)],
            out.diff.as_ref().map(|d| d.files.len()).unwrap_or(0),
        );
    }

    eprintln!("watch-demo: ready — make a change (e.g. `jj describe -m hi`)\n");

    loop {
        // Block for a tick, then drain to the latest pending one (latest-wins):
        // if several ticks queued while we rendered, only the newest matters.
        let Some(tick) = watcher.next_change_timeout(Duration::from_secs(3600)) else {
            continue;
        };
        let tick = watcher.latest_pending().unwrap_or(tick);

        match reloader.reload()? {
            None => {
                // Op id unchanged: a `.jj/` touch that doesn't move the log
                // (the cheap-gate fast path). Report it as a near-zero no-op.
                eprintln!(
                    "[tick {:>4}] no-op  (op unchanged)  latency {:>7.2}ms",
                    tick.seq,
                    tick.event_at.elapsed().as_secs_f64() * 1000.0,
                );
            }
            Some(out) => {
                let latency_ms = tick.event_at.elapsed().as_secs_f64() * 1000.0;
                let t = out.timings;
                let delta = if out.rev_delta == 0 {
                    "0".to_string()
                } else {
                    format!("{:+}", out.rev_delta)
                };
                eprintln!(
                    "[tick {:>4}] reload latency {:>7.2}ms  | revs {:>4} ({delta})  diff files {:>3}  | \
                     reload {:>6.2} snapshot {:>6.2} diff {:>6.2} (cache {})",
                    tick.seq,
                    latency_ms,
                    out.snapshot.revision_count(),
                    out.diff.as_ref().map(|d| d.files.len()).unwrap_or(0),
                    t.reload.as_secs_f64() * 1000.0,
                    t.snapshot.as_secs_f64() * 1000.0,
                    t.diff.as_secs_f64() * 1000.0,
                    reloader.diff_cache_len(),
                );
            }
        }
    }
}

fn default_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixture/repo")
}
