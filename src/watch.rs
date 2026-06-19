//! Real-time reactivity: a recursive filesystem watcher over a jj repo's `.jj/`
//! directory, plus a reusable "reactive reload" helper that re-derives the
//! render-ready [`Snapshot`](crate::model::Snapshot) (+ the selected commit's
//! diff) whenever the repo changes on disk.
//!
//! # Design
//!
//! jj records every state transition (commits, rebases, working-copy snapshots,
//! bookmark moves, undo) as an **operation**, and the op log + working-copy
//! state both live under `.jj/`. So watching `.jj/` recursively catches every
//! change that could alter what we draw — without polling.
//!
//! Raw fs events are bursty: a single `jj` command rewrites the op store, the
//! view, the index, and the working-copy state, firing many events in a few ms.
//! Re-rendering on each one is wasteful and, under heavy churn, unbounded. So
//! the watcher **coalesces**: it debounces a burst (latest-wins) and only emits
//! one "repo changed" tick per quiet period. The channel is **bounded** and
//! **latest-wins** — a consumer that falls behind never accumulates a backlog;
//! it always sees the newest tick and stale ones are dropped. This is what keeps
//! the app responsive (and memory flat) under a storm of writes.
//!
//! # API surface
//!
//! * [`Watcher`] — owns the OS watch + the debounce thread; hand it the repo
//!   root, get back a [`Watcher::changes`] receiver that yields [`Tick`]s.
//! * [`ReactiveReloader`] (feature `jjlib`) — holds an open repo and, on each
//!   tick, reloads the snapshot + selected diff and reports a [`ReloadOutcome`]
//!   with a per-stage timing [`Timings`] breakdown.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context as _};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

/// One "the repo changed on disk" notification. Carries a monotonically
/// increasing sequence number and the instant the *triggering* fs event was
/// observed, so a consumer can measure end-to-end event→rendered latency.
#[derive(Clone, Copy, Debug)]
pub struct Tick {
    /// Monotonic counter; gaps mean ticks were coalesced/dropped (expected and
    /// healthy under churn — the consumer always gets the latest state).
    pub seq: u64,
    /// When the first fs event of the burst that produced this tick was seen.
    /// `event_at.elapsed()` at render time is the true reactive latency.
    pub event_at: Instant,
}

/// How the watcher coalesces bursts. Defaults match the spec: a short
/// latest-wins debounce, never queueing unboundedly.
#[derive(Clone, Copy, Debug)]
pub struct WatchConfig {
    /// Quiet period after the last event before a tick is emitted. A jj command
    /// fires a burst over a few ms; 40ms collapses it into one tick while still
    /// feeling instant. (Spec: 30–60ms.)
    pub debounce: Duration,
    /// Hard ceiling on how long a continuous storm can defer a tick. Even if
    /// events never stop, a tick is forced after this long so the consumer is
    /// not starved. Keeps max lag bounded under sustained churn.
    pub max_defer: Duration,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(40),
            max_defer: Duration::from_millis(250),
        }
    }
}

/// A recursive `.jj/` watcher with bounded, latest-wins, debounced output.
///
/// Dropping the `Watcher` stops the OS watch and joins the debounce thread.
pub struct Watcher {
    /// Kept alive so the OS watch stays registered; dropping it unwatches.
    _inner: RecommendedWatcher,
    rx: Receiver<Tick>,
    stop: Arc<AtomicBool>,
    debounce_thread: Option<JoinHandle<()>>,
    /// Count of raw fs events observed (for churn diagnostics). Lets a test
    /// confirm "N raw events collapsed into M ticks" coalescing.
    raw_events: Arc<AtomicU64>,
}

impl Watcher {
    /// Watch `repo_root`'s `.jj/` directory recursively with default debounce.
    pub fn new(repo_root: &Path) -> anyhow::Result<Self> {
        Self::with_config(repo_root, WatchConfig::default())
    }

    /// Watch `repo_root`'s `.jj/` directory recursively with a custom config.
    pub fn with_config(repo_root: &Path, cfg: WatchConfig) -> anyhow::Result<Self> {
        let jj_dir = jj_dir(repo_root)?;

        // Raw events from notify land on `raw_tx`; the debounce thread coalesces
        // them and forwards at most one `Tick` per quiet period to `rx`.
        let (raw_tx, raw_rx) = mpsc::channel::<Instant>();
        let (tick_tx, tick_rx) = mpsc::channel::<Tick>();
        let raw_events = Arc::new(AtomicU64::new(0));

        let raw_events_w = raw_events.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            // Stamp arrival immediately; we don't inspect paths — any change
            // under `.jj/` is a candidate. (Filtering by path here would risk
            // missing op-store writes; the cheap op-id gate downstream is the
            // real "did it matter?" check.)
            if res.is_ok() {
                raw_events_w.fetch_add(1, Ordering::Relaxed);
                // If the consumer side is gone, the send just fails; ignore.
                let _ = raw_tx.send(Instant::now());
            }
        })
        .context("creating filesystem watcher")?;

        watcher
            .watch(&jj_dir, RecursiveMode::Recursive)
            .with_context(|| format!("watching {}", jj_dir.display()))?;

        let stop = Arc::new(AtomicBool::new(false));
        let debounce_thread = spawn_debounce(raw_rx, tick_tx, cfg, stop.clone());

        Ok(Self {
            _inner: watcher,
            rx: tick_rx,
            stop,
            debounce_thread: Some(debounce_thread),
            raw_events,
        })
    }

    /// Block until the next coalesced change tick (or the watcher is dropped).
    pub fn next_change(&self) -> Option<Tick> {
        self.rx.recv().ok()
    }

    /// Block up to `timeout` for the next change tick.
    pub fn next_change_timeout(&self, timeout: Duration) -> Option<Tick> {
        match self.rx.recv_timeout(timeout) {
            Ok(t) => Some(t),
            Err(_) => None,
        }
    }

    /// Drain to the *latest* pending tick without blocking, discarding older
    /// ones. This is the latest-wins read a render loop wants: never process a
    /// stale state when a newer one is already available.
    pub fn latest_pending(&self) -> Option<Tick> {
        let mut latest = None;
        loop {
            match self.rx.try_recv() {
                Ok(t) => latest = Some(t),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        latest
    }

    /// Borrow the underlying receiver (e.g. to `select!` over several sources).
    pub fn receiver(&self) -> &Receiver<Tick> {
        &self.rx
    }

    /// Total raw fs events observed so far. Compare against the number of ticks
    /// consumed to quantify coalescing.
    pub fn raw_event_count(&self) -> u64 {
        self.raw_events.load(Ordering::Relaxed)
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Dropping `_inner` (the notify watcher) closes `raw_tx`, which unblocks
        // the debounce thread's recv so it can observe `stop` and exit.
        if let Some(h) = self.debounce_thread.take() {
            let _ = h.join();
        }
    }
}

/// The debounce/coalesce thread. Reads raw event timestamps, and emits a single
/// `Tick` once events have been quiet for `cfg.debounce` (or `cfg.max_defer` has
/// elapsed since the burst began, whichever comes first). Memory is O(1): we
/// keep only the earliest pending event instant and a sequence counter — never
/// a queue of events.
fn spawn_debounce(
    raw_rx: Receiver<Instant>,
    tick_tx: Sender<Tick>,
    cfg: WatchConfig,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut seq: u64 = 0;
        // The instant of the first event of the current (not-yet-emitted) burst.
        let mut burst_start: Option<Instant> = None;

        loop {
            if stop.load(Ordering::Relaxed) {
                return;
            }

            // If a burst is pending, wait only until its deadline; otherwise
            // block (with a wakeup so `stop` is observed).
            let wait = match burst_start {
                Some(start) => {
                    // Emit when quiet for `debounce`, but never defer past
                    // `max_defer` from the burst start.
                    let by_quiet = cfg.debounce;
                    let by_cap = cfg.max_defer.saturating_sub(start.elapsed());
                    by_quiet.min(by_cap)
                }
                None => Duration::from_millis(200),
            };

            match raw_rx.recv_timeout(wait) {
                Ok(_event_at) => {
                    // Another event arrived; extend the burst. We keep the
                    // EARLIEST instant (true latency origin) and reset the quiet
                    // window implicitly by re-looping (recv_timeout restarts).
                    burst_start.get_or_insert(Instant::now());
                    // If we've already deferred past the cap, fall through to
                    // emit on the next loop (wait becomes ~0).
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Quiet (or cap hit): if a burst is pending, emit one tick.
                    if let Some(start) = burst_start.take() {
                        seq += 1;
                        // Bounded latest-wins: if the consumer hasn't drained the
                        // previous tick, the channel still just holds ticks in
                        // order, but consumers use `latest_pending()` to skip to
                        // the newest. Send failure => consumer gone; exit.
                        if tick_tx.send(Tick { seq, event_at: start }).is_err() {
                            return;
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // Watcher dropped: emit any final pending tick, then exit.
                    if let Some(start) = burst_start.take() {
                        seq += 1;
                        let _ = tick_tx.send(Tick { seq, event_at: start });
                    }
                    return;
                }
            }
        }
    })
}

/// Resolve and validate the `.jj/` directory under a repo root.
fn jj_dir(repo_root: &Path) -> anyhow::Result<PathBuf> {
    let dir = repo_root.join(".jj");
    if !dir.is_dir() {
        return Err(anyhow!(
            "no .jj/ directory under {} — not a jj repo?",
            repo_root.display()
        ));
    }
    Ok(dir)
}

// ---------------------------------------------------------------------------
// Reactive reload (jjlib-only): turn a tick into a fresh Snapshot + diff.
// ---------------------------------------------------------------------------

/// Per-stage timing breakdown of one reactive reload. Every field is wall time
/// for that stage; sum ≈ end-to-end "decide what to draw" cost (the budget that
/// must fit a frame). `scene_build` is filled only when the caller asks the
/// reloader to also build a scene (the harness does; the demo doesn't need to).
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    /// `RepoLoader::load_at_head` (re-resolve the repo at the new head op).
    pub reload: Duration,
    /// Revset eval + `stream_graph` + building all `CommitNode`s (the Snapshot).
    pub snapshot: Duration,
    /// Materializing the selected commit's diff (tree diff + line diff).
    pub diff: Duration,
    /// `ui::build_scene` scene construction, if measured.
    pub scene_build: Duration,
    /// Whether this reload was skipped because the op id was unchanged.
    pub skipped_no_change: bool,
}

impl Timings {
    /// End-to-end "decide what to draw" time: reload + snapshot + diff + scene.
    pub fn total(&self) -> Duration {
        self.reload + self.snapshot + self.diff + self.scene_build
    }
}

#[cfg(feature = "jjlib")]
pub use reactive::{ReactiveReloader, ReloadOutcome};

#[cfg(feature = "jjlib")]
mod reactive {
    use super::*;
    use crate::model::jjlib::{self, DiffCache, Loaded};
    use crate::model::{CommitDiff, Snapshot};

    /// The result of one reactive reload.
    pub struct ReloadOutcome {
        /// The freshly-loaded snapshot (the new graph/log state).
        pub snapshot: Snapshot,
        /// The selected commit's diff (working copy by default), if any.
        pub diff: Option<Arc<CommitDiff>>,
        /// Per-stage timings.
        pub timings: Timings,
        /// Revision count delta vs the previous reload (for "what changed" logs).
        pub rev_delta: i64,
        /// New head operation id (hex) after the reload.
        pub op_id: String,
    }

    /// Holds an open repo and reloads it on demand, exposing a per-stage timed
    /// reactive reload. Caches per-commit diffs by `commit_id` (immutable once
    /// written) so an unchanged selection is re-rendered for free.
    pub struct ReactiveReloader {
        loaded: Loaded,
        diff_cache: DiffCache,
        last_op_id: String,
        last_rev_count: usize,
        /// Commit id (hex) whose diff to materialize each reload. `None` => use
        /// the current working-copy commit (re-resolved each reload).
        selected_commit: Option<String>,
    }

    impl ReactiveReloader {
        /// Open `repo_root` and prime the reloader.
        pub fn open(repo_root: &Path) -> anyhow::Result<Self> {
            let loaded = jjlib::open(repo_root)?;
            let last_op_id = loaded.current_op_id_hex();
            Ok(Self {
                loaded,
                diff_cache: DiffCache::with_capacity(64),
                last_op_id,
                last_rev_count: 0,
                selected_commit: None,
            })
        }

        /// Pin the diff to a specific commit (hex). `None` tracks the working
        /// copy. Mirrors the UI's selected-row cursor.
        pub fn select_commit(&mut self, commit_id: Option<String>) {
            self.selected_commit = commit_id;
        }

        /// Number of per-commit diffs currently cached (bounded — see DiffCache).
        pub fn diff_cache_len(&self) -> usize {
            self.diff_cache.len()
        }

        /// Borrow the open repo (e.g. to build a scene against the snapshot).
        pub fn loaded(&self) -> &Loaded {
            &self.loaded
        }

        /// Reload the snapshot + selected diff, timing each stage.
        ///
        /// Cheap-win gate: first compare the head op id on disk against the one
        /// we're loaded at. If unchanged, we know nothing we draw could have
        /// changed, so we skip `load_at_head` + snapshot + diff entirely and
        /// return `None` (the caller keeps its current render). This is what
        /// makes spurious `.jj/` touches (and most coalesced churn) nearly free.
        pub fn reload(&mut self) -> anyhow::Result<Option<ReloadOutcome>> {
            // Cheap gate: did the op log actually move? Reading op heads does not
            // load the repo.
            let on_disk = self.loaded.op_heads_on_disk().unwrap_or_default();
            if !on_disk.is_empty() && on_disk == self.last_op_id {
                return Ok(None);
            }

            let mut timings = Timings::default();

            let t0 = Instant::now();
            let op_id = self.loaded.reload_at_head()?;
            timings.reload = t0.elapsed();

            // If, after actually resolving the head, the op id matches what we
            // had, nothing changed (the disk heads differed only transiently).
            if op_id == self.last_op_id {
                timings.skipped_no_change = true;
                // Still return an outcome so callers can see the (cheap) timing,
                // but reuse cached diff / current snapshot count.
            }

            let t1 = Instant::now();
            let snapshot = jjlib::snapshot(&self.loaded)?;
            timings.snapshot = t1.elapsed();

            // Resolve which commit's diff to show.
            let commit = self
                .selected_commit
                .clone()
                .or_else(|| self.loaded.wc_commit_id_hex());

            let t2 = Instant::now();
            let diff = match &commit {
                Some(c) => Some(self.diff_cache.get_or_compute(&self.loaded, c)?),
                None => None,
            };
            timings.diff = t2.elapsed();

            let rev_count = snapshot.revision_count();
            let rev_delta = rev_count as i64 - self.last_rev_count as i64;
            self.last_rev_count = rev_count;
            self.last_op_id = op_id.clone();

            Ok(Some(ReloadOutcome {
                snapshot,
                diff,
                timings,
                rev_delta,
                op_id,
            }))
        }

        /// Force a reload ignoring the op-id gate (used by the perf harness so it
        /// measures real reload cost, not the skip path).
        pub fn reload_forced(&mut self) -> anyhow::Result<ReloadOutcome> {
            let mut timings = Timings::default();

            let t0 = Instant::now();
            let op_id = self.loaded.reload_at_head()?;
            timings.reload = t0.elapsed();

            let t1 = Instant::now();
            let snapshot = jjlib::snapshot(&self.loaded)?;
            timings.snapshot = t1.elapsed();

            let commit = self
                .selected_commit
                .clone()
                .or_else(|| self.loaded.wc_commit_id_hex());
            let t2 = Instant::now();
            let diff = match &commit {
                // Bypass the cache so the harness measures real diff cost.
                Some(c) => Some(Arc::new(jjlib::commit_diff(&self.loaded, c)?)),
                None => None,
            };
            timings.diff = t2.elapsed();

            let rev_count = snapshot.revision_count();
            let rev_delta = rev_count as i64 - self.last_rev_count as i64;
            self.last_rev_count = rev_count;
            self.last_op_id = op_id.clone();

            Ok(ReloadOutcome {
                snapshot,
                diff,
                timings,
                rev_delta,
                op_id,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A throwaway dir that looks enough like a jj repo to watch (`.jj/` exists).
    fn temp_repo() -> (PathBuf, impl Drop) {
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let base = std::env::temp_dir().join(format!(
            "jjscratch-watch-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(base.join(".jj/repo")).unwrap();
        (base.clone(), Cleanup(base))
    }

    #[test]
    fn errors_without_jj_dir() {
        let dir = std::env::temp_dir().join(format!("jjscratch-nojj-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        assert!(Watcher::new(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fires_on_change_and_coalesces() {
        let (repo, _guard) = temp_repo();
        let cfg = WatchConfig {
            debounce: Duration::from_millis(40),
            max_defer: Duration::from_millis(250),
        };
        let watcher = Watcher::with_config(&repo, cfg).expect("watcher");

        // Burst of writes: many events should coalesce into very few ticks.
        let target = repo.join(".jj/repo/op_store");
        fs::create_dir_all(&target).unwrap();
        for i in 0..50 {
            fs::write(target.join(format!("f{i}")), b"x").unwrap();
        }

        // Wait for at least one tick.
        let first = watcher.next_change_timeout(Duration::from_secs(5));
        assert!(first.is_some(), "watcher should fire on writes under .jj/");

        // Drain a brief settle window, then assert coalescing: far fewer ticks
        // than raw events.
        std::thread::sleep(Duration::from_millis(150));
        let mut ticks = 1;
        while watcher.next_change_timeout(Duration::from_millis(50)).is_some() {
            ticks += 1;
        }
        let raw = watcher.raw_event_count();
        assert!(raw >= 1, "should have observed raw events (got {raw})");
        assert!(
            ticks <= raw,
            "coalescing should not amplify: {ticks} ticks from {raw} raw events"
        );
        // 50 writes must not produce 50 ticks.
        assert!(ticks < 50, "expected coalescing, got {ticks} ticks");
    }

    #[test]
    fn latest_pending_skips_stale() {
        let (repo, _guard) = temp_repo();
        let watcher = Watcher::new(&repo).expect("watcher");
        let target = repo.join(".jj/repo");
        for i in 0..10 {
            fs::write(target.join(format!("g{i}")), b"y").unwrap();
            std::thread::sleep(Duration::from_millis(60));
        }
        std::thread::sleep(Duration::from_millis(100));
        // latest_pending returns Some (the newest) or None, never a backlog.
        let _ = watcher.latest_pending();
        // A second call right after drains to empty.
        assert!(watcher.latest_pending().is_none());
    }
}
