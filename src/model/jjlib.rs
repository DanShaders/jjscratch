//! The real jj-lib backend loader.
//!
//! Reads a Jujutsu repository in-process via `jj-lib` 0.42 and produces the
//! exact same render-ready [`Snapshot`] / [`CommitDiff`] value types that
//! [`mock`](super::mock) produces. No jj-lib types leak past this module.
//!
//! jj-lib's read APIs (repo loading, revset streams, diff/file reads) are all
//! async; we drive them synchronously with `pollster::block_on` and
//! `futures::StreamExt`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Context as _};
use futures::StreamExt as _;

use jj_lib::backend::CommitId;
use jj_lib::merge::MergedTreeValue;
use jj_lib::commit::Commit;
use jj_lib::conflict_labels::ConflictLabels;
use jj_lib::conflicts::{materialize_tree_value, MaterializedTreeValue};
use jj_lib::config::StackedConfig;
use jj_lib::graph::{GraphEdgeType, TopoGroupedGraph};
use jj_lib::matchers::EverythingMatcher;
use jj_lib::object_id::ObjectId as _;
use jj_lib::repo::{ReadonlyRepo, Repo, StoreFactories};
use jj_lib::repo_path::RepoPath;
use jj_lib::revset::{ResolvedRevsetExpression, RevsetExpression};
use jj_lib::settings::UserSettings;
use jj_lib::store::Store;
use jj_lib::workspace::{default_working_copy_factories, Workspace};

use super::{
    Bookmark, BookmarkKind, ChangeStatus, CommitDiff, CommitNode, DiffLine, EdgeType, FileDiff,
    GraphParent, Hunk, LineKind, Snapshot,
};

/// An opened jj workspace + the repo at the head operation, plus working-copy
/// info. Hold this for the lifetime of the rendered session.
pub struct Loaded {
    pub workspace: Workspace,
    pub repo: Arc<ReadonlyRepo>,
    wc_commit_id: Option<CommitId>,
    repo_name: String,
    workspace_name: String,
}

impl Loaded {
    /// The working-copy commit id (hex) for this workspace, if any.
    pub fn wc_commit_id_hex(&self) -> Option<String> {
        self.wc_commit_id.as_ref().map(|id| id.hex())
    }
}

/// Open the jj workspace whose working-copy root is `path`.
pub fn open(path: &Path) -> anyhow::Result<Loaded> {
    // Built-in default config variables are enough to open and read a repo.
    let config = StackedConfig::with_defaults();
    let settings = UserSettings::from_config(config).context("building UserSettings")?;

    let workspace = Workspace::load(
        &settings,
        path,
        &StoreFactories::default(),
        &default_working_copy_factories(),
    )
    .with_context(|| format!("loading jj workspace at {}", path.display()))?;

    // Load the repo at the current head operation (async).
    let repo: Arc<ReadonlyRepo> = pollster::block_on(workspace.repo_loader().load_at_head())
        .context("loading repo at head")?;

    let wc_name = workspace.workspace_name().to_owned();
    let wc_commit_id = repo.view().get_wc_commit_id(&wc_name).cloned();

    // repo_name from the workspace root directory name.
    let repo_name = workspace
        .workspace_root()
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    let workspace_name = wc_name.as_str().to_owned();

    Ok(Loaded {
        workspace,
        repo,
        wc_commit_id,
        repo_name,
        workspace_name,
    })
}

/// The commit ids of the fixture's `immutable_heads()` alias,
/// `immutable_heads() = present(main) | tags()`.
///
/// To avoid the revset symbol parser we read the head commit ids straight from
/// the view: the `main` local bookmark plus every tag's local + remote targets.
fn immutable_head_ids(repo: &ReadonlyRepo) -> Vec<CommitId> {
    let view = repo.view();
    let mut heads: Vec<CommitId> = Vec::new();

    // present(main): the local bookmark named "main", if it exists.
    for (name, target) in view.local_bookmarks() {
        if name.as_str() == "main" {
            heads.extend(target.added_ids().cloned());
        }
    }
    // tags(): every tag's local + remote targets.
    for (_name, target) in view.tags() {
        heads.extend(target.local_target.added_ids().cloned());
        for (_remote, remote_ref) in &target.remote_refs {
            heads.extend(remote_ref.target.added_ids().cloned());
        }
    }
    heads
}

/// Build the immutable-heads ancestors revset, mirroring the fixture's
/// `immutable_heads() = present(main) | tags()` alias.
///
/// jj-lib has no built-in `immutable()`; we reconstruct `::(<heads> | root())`
/// from primitives: `commits(heads).ancestors() | root()`.
fn immutable_expr(repo: &ReadonlyRepo) -> Arc<ResolvedRevsetExpression> {
    let heads = immutable_head_ids(repo);

    let root: Arc<ResolvedRevsetExpression> = RevsetExpression::root();
    if heads.is_empty() {
        return root;
    }
    let heads_expr: Arc<ResolvedRevsetExpression> = RevsetExpression::commits(heads);
    heads_expr.ancestors().union(&root)
}

/// Build jj's default-log revset for this fixture:
///
/// ```text
/// present(@) | ancestors(immutable_heads().., 2) | present(trunk())
/// ```
///
/// where, for this fixture, `immutable_heads() = present(main) | tags()` and
/// `trunk()` resolves to `root()` (its default `latest(remote_bookmarks(main@origin)
/// | … | root())` falls back to `root()` because the fixture has no remote
/// bookmarks). The fixture overrides only `immutable_heads()`, not `trunk()`.
///
/// Translation to jj-lib combinators (all on the *resolved* expression state):
///
/// * `present(@)` → `commits([wc_id])` (omitted entirely if there is no working
///   copy, which is exactly what `present()` does for an absent name).
/// * `immutable_heads()..` → `immutable_heads().range(visible_heads())`. jj's
///   `x..` desugars to `~ancestors(x)`, which after resolution against the
///   visible DAG equals `ancestors(visible_heads()) ∖ ancestors(immutable_heads())`
///   — exactly what `range(roots = immutable_heads, heads = visible_heads)`
///   computes ("commits reachable from heads but not from roots").
/// * `ancestors(<range>, 2)` → `<range>.ancestors_range(0..2)`. This is the
///   generation-limited ancestors API; depth 2 keeps generations 0 and 1, so
///   deeper immutable ancestors drop out. That gap is what makes `stream_graph`
///   emit `Indirect` (elided) edges → the `(elided revisions)` marker.
/// * `trunk()` → `root()`.
///
/// All pieces are unioned. The result is the same row set, count, and elision
/// that `jj log` (and lightjj) show by default: `@`, wip, refactor, the
/// `experiment` fork, feat[main], then the elided gap, then the root commit.
fn default_log_expr(
    repo: &ReadonlyRepo,
    wc_commit_id: Option<&CommitId>,
) -> Arc<ResolvedRevsetExpression> {
    let mut parts: Vec<Arc<ResolvedRevsetExpression>> = Vec::new();

    // present(@)
    if let Some(wc_id) = wc_commit_id {
        parts.push(RevsetExpression::commits(vec![wc_id.clone()]));
    }

    // ancestors(immutable_heads().., 2)
    let immutable_heads: Arc<ResolvedRevsetExpression> = {
        let heads = immutable_head_ids(repo);
        if heads.is_empty() {
            RevsetExpression::root()
        } else {
            RevsetExpression::commits(heads)
        }
    };
    let after_immutable = immutable_heads.range(&RevsetExpression::visible_heads());
    parts.push(after_immutable.ancestors_range(0..2));

    // trunk() — resolves to root() for this fixture (no remote bookmarks).
    parts.push(RevsetExpression::root());

    RevsetExpression::union_all(&parts)
}

/// Evaluate a revset and eagerly test membership of `ids`, returning a map for
/// O(1) lookups. Uses the revset's `containing_fn` (the intended bulk-test API).
fn collect_membership(
    repo: &ReadonlyRepo,
    expr: Arc<ResolvedRevsetExpression>,
    ids: &[CommitId],
) -> anyhow::Result<HashMap<CommitId, bool>> {
    let revset = expr
        .evaluate(repo)
        .map_err(|e| anyhow!("evaluating revset: {e}"))?;
    let contains = revset.containing_fn();
    let mut out = HashMap::with_capacity(ids.len());
    for id in ids {
        let yes = contains(id).map_err(|e| anyhow!("revset membership: {e}"))?;
        out.insert(id.clone(), yes);
    }
    Ok(out)
}

/// Build a full [`Snapshot`] by streaming jj's default-log revset and turning
/// each graph node into a [`CommitNode`], ordered to match `jj log`'s default
/// output.
///
/// # Revset
///
/// We evaluate [`default_log_expr`] (jj's built-in default-log revset,
/// `present(@) | ancestors(immutable_heads().., 2) | present(trunk())`) rather
/// than `all()`. That narrower set shows the same rows lightjj does and, because
/// the immutable ancestors are limited to 2 generations, `stream_graph` emits
/// `Indirect` (elided) edges across the dropped commits — which the graph
/// renderer draws as the `(elided revisions)` marker.
///
/// # Ordering
///
/// jj-lib's `Revset::stream_graph` yields nodes in *descending index position*
/// (children before parents, but heads are interleaved purely by insertion
/// position). For our fixture that puts the side branch `experiment` first,
/// which does not match `jj log` / lightjj.
///
/// jj's CLI does not render that raw stream; it feeds it through
/// [`jj_lib::graph::TopoGroupedGraph`], which DFS-groups each topological branch
/// so a branch is emitted contiguously (fork descendants visited at the fork,
/// merge ancestors visited last-first — the same shape Git uses). We do the
/// same here, and additionally call `prioritize_branch(working_copy)` so the
/// branch containing `@` (the working copy and its ancestors) is emitted before
/// any other head. That reproduces lightjj's default-log order exactly:
/// `@`, wip, refactor, then the `experiment` fork interleaved at its branch
/// point, then feat[main], then the elided gap and the root commit.
///
/// Re-grouping only changes the node *sequence*; each node keeps its original
/// parent edges from `stream_graph`, so the DAG/lane layout is unchanged.
pub fn snapshot(loaded: &Loaded) -> anyhow::Result<Snapshot> {
    let repo = loaded.repo.as_ref();
    let store = repo.store();
    let view = repo.view();

    // 1. Stream the DAG: (CommitId, edges) in jj-log order. We use jj's
    //    default-log revset (not all()), so the rows, count, and elision match
    //    lightjj.
    let expr = default_log_expr(repo, loaded.wc_commit_id.as_ref());
    let revset = expr
        .evaluate(repo)
        .map_err(|e| anyhow!("evaluating default-log revset: {e}"))?;

    let graph_nodes: Vec<(CommitId, Vec<(CommitId, EdgeType)>)> = pollster::block_on(async {
        // Re-group the topological stream into contiguous branches (jj's CLI
        // log ordering), prioritizing the working-copy branch so `@` and its
        // ancestors are emitted first.
        let mut topo = TopoGroupedGraph::new(revset.stream_graph(), |id: &CommitId| id);
        if let Some(wc_id) = &loaded.wc_commit_id {
            topo.prioritize_branch(wc_id.clone());
        }
        let mut stream = std::pin::pin!(topo.stream());

        let mut nodes = Vec::new();
        while let Some(node) = stream.next().await {
            let (commit_id, edges) = node.map_err(|e| anyhow!("graph stream: {e}"))?;
            let mapped: Vec<(CommitId, EdgeType)> = edges
                .into_iter()
                .map(|e| (e.target, map_edge(e.edge_type)))
                .collect();
            nodes.push((commit_id, mapped));
        }
        Ok::<_, anyhow::Error>(nodes)
    })?;

    let ids: Vec<CommitId> = graph_nodes.iter().map(|(id, _)| id.clone()).collect();

    // 2. Precompute immutability + divergence membership for all visible nodes.
    let immutable = collect_membership(repo, immutable_expr(repo), &ids)?;
    let divergent = collect_membership(repo, RevsetExpression::divergent(), &ids)?;

    // 3. Build CommitNode per graph node.
    let mut out_nodes = Vec::with_capacity(graph_nodes.len());
    for (commit_id, edges) in &graph_nodes {
        let commit: Commit = store
            .get_commit(commit_id)
            .with_context(|| format!("reading commit {}", commit_id.hex()))?;

        let change_id = commit.change_id().clone();
        let change_prefix_len = repo
            .shortest_unique_change_id_prefix_len(&change_id)
            .unwrap_or_else(|_| change_id.hex().len());
        let commit_prefix_len = repo
            .index()
            .shortest_unique_commit_id_prefix_len(commit_id)
            .unwrap_or_else(|_| commit_id.hex().len());

        let author = commit.author();
        let timestamp_ms = author.timestamp.timestamp.0;
        let description = commit.description().to_owned();

        let parents: Vec<GraphParent> = edges
            .iter()
            .map(|(target, edge_type)| GraphParent {
                commit_id: target.hex(),
                edge_type: *edge_type,
            })
            .collect();

        let bookmarks = bookmarks_at(repo, commit_id);

        let is_working_copy = view.is_wc_commit_id(commit_id);
        let is_immutable = immutable.get(commit_id).copied().unwrap_or(false);
        let is_divergent = divergent.get(commit_id).copied().unwrap_or(false);

        // "Empty" = no file changes vs the (first) parent — NOT an empty
        // description. Root (no parent) is empty by convention.
        let is_empty = match commit.parent_ids().first() {
            None => true,
            Some(pid) => match store.get_commit(pid) {
                Ok(parent) => commit.tree_ids() == parent.tree_ids(),
                Err(_) => false,
            },
        };

        out_nodes.push(CommitNode {
            // jj displays change ids in its reverse-hex (z-k) alphabet, not raw
            // hex — e.g. "lxtsspsu", not "e2677a75".
            change_id: change_id.reverse_hex(),
            change_prefix_len,
            commit_id: commit_id.hex(),
            commit_prefix_len,
            is_empty,
            description,
            author_name: author.name.clone(),
            author_email: author.email.clone(),
            timestamp_ms,
            is_working_copy,
            is_immutable,
            is_divergent,
            has_conflict: commit.has_conflict(),
            // The default-log revset only yields visible commits; hidden
            // commits are not shown.
            is_hidden: false,
            parents,
            bookmarks,
        });
    }

    Ok(Snapshot {
        repo_name: loaded.repo_name.clone(),
        workspace_name: loaded.workspace_name.clone(),
        wc_commit_id: loaded.wc_commit_id_hex(),
        nodes: out_nodes,
    })
}

fn map_edge(kind: GraphEdgeType) -> EdgeType {
    match kind {
        GraphEdgeType::Direct => EdgeType::Direct,
        GraphEdgeType::Indirect => EdgeType::Indirect,
        GraphEdgeType::Missing => EdgeType::Missing,
    }
}

/// Local + remote bookmarks pointing at `commit_id`.
fn bookmarks_at(repo: &ReadonlyRepo, commit_id: &CommitId) -> Vec<Bookmark> {
    let view = repo.view();
    let mut out = Vec::new();

    for (name, target) in view.local_bookmarks_for_commit(commit_id) {
        out.push(Bookmark {
            name: name.as_str().to_owned(),
            kind: BookmarkKind::Local,
            conflicted: target.added_ids().count() > 1,
            unsynced: false,
        });
    }

    for (symbol, remote_ref) in view.all_remote_bookmarks() {
        if remote_ref.target.added_ids().any(|id| id == commit_id) {
            out.push(Bookmark {
                name: symbol.name.as_str().to_owned(),
                kind: BookmarkKind::Remote {
                    remote: symbol.remote.as_str().to_owned(),
                },
                conflicted: remote_ref.target.added_ids().count() > 1,
                unsynced: false,
            });
        }
    }

    out
}

// --- Diff -----------------------------------------------------------------

/// The materialized state of one side of a file (before or after).
enum Side {
    /// File absent on this side (added when before is absent, deleted when after is).
    Absent,
    /// Regular text file, split into lines (without trailing newline markers).
    Text(Vec<String>),
    /// Binary / symlink / conflict / submodule — diffed as opaque.
    Opaque,
}

/// Compute the diff of `commit_id` against its first parent and turn it into a
/// [`CommitDiff`].
pub fn commit_diff(loaded: &Loaded, commit_id: &str) -> anyhow::Result<CommitDiff> {
    let repo = loaded.repo.as_ref();
    let store = repo.store();

    let id = CommitId::try_from_hex(commit_id)
        .ok_or_else(|| anyhow!("invalid commit id hex: {commit_id}"))?;
    let commit = store
        .get_commit(&id)
        .with_context(|| format!("reading commit {commit_id}"))?;

    let to_tree = commit.tree();
    let from_tree = match commit.parent_ids().first() {
        Some(pid) => store.get_commit(pid).context("reading parent commit")?.tree(),
        None => store.empty_merged_tree(),
    };

    let files = pollster::block_on(async {
        let mut files = Vec::new();
        let mut stream = from_tree.diff_stream(&to_tree, &EverythingMatcher);
        while let Some(entry) = stream.next().await {
            let path = entry.path;
            let diff = entry.values.map_err(|e| anyhow!("diff entry: {e}"))?;

            let before_present = diff.before.is_present();
            let after_present = diff.after.is_present();

            let before = read_side(store, &path, diff.before).await?;
            let after = read_side(store, &path, diff.after).await?;

            let status = match (before_present, after_present) {
                (false, true) => ChangeStatus::Added,
                (true, false) => ChangeStatus::Deleted,
                _ => ChangeStatus::Modified,
            };

            let path_str = path.as_internal_file_string().to_owned();
            files.push(build_file_diff(path_str, status, before, after));
        }
        Ok::<_, anyhow::Error>(files)
    })?;

    Ok(CommitDiff {
        commit_id: id.hex(),
        files,
    })
}

async fn read_side(
    store: &Arc<Store>,
    path: &RepoPath,
    value: MergedTreeValue,
) -> anyhow::Result<Side> {
    if value.is_absent() {
        return Ok(Side::Absent);
    }
    match materialize_tree_value(store, path, value, &ConflictLabels::unlabeled())
        .await
        .map_err(|e| anyhow!("materializing {}: {e}", path.as_internal_file_string()))?
    {
        MaterializedTreeValue::Absent => Ok(Side::Absent),
        MaterializedTreeValue::File(mut f) => {
            let bytes = f
                .read_all(path)
                .await
                .map_err(|e| anyhow!("reading file {}: {e}", path.as_internal_file_string()))?;
            match std::str::from_utf8(&bytes) {
                Ok(s) => Ok(Side::Text(split_lines(s))),
                Err(_) => Ok(Side::Opaque), // binary
            }
        }
        // Symlinks, conflicts, submodules, trees, access-denied: opaque.
        _ => Ok(Side::Opaque),
    }
}

/// Split text into logical lines, dropping a single trailing newline so a file
/// ending in "\n" does not produce a spurious empty final line.
fn split_lines(s: &str) -> Vec<String> {
    if s.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = s.split('\n').map(|l| l.to_owned()).collect();
    if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }
    lines
}

fn build_file_diff(path: String, status: ChangeStatus, before: Side, after: Side) -> FileDiff {
    let (old_lines, new_lines) = match (before, after) {
        (Side::Text(o), Side::Text(n)) => (o, n),
        (Side::Absent, Side::Text(n)) => (Vec::new(), n),
        (Side::Text(o), Side::Absent) => (o, Vec::new()),
        // Opaque (binary/symlink/conflict): no textual hunks, just a status.
        _ => {
            return FileDiff {
                path,
                status,
                added: 0,
                removed: 0,
                hunks: Vec::new(),
            };
        }
    };

    let ops = diff_lines(&old_lines, &new_lines);
    let hunks = group_hunks(&ops, &old_lines, &new_lines, 3);

    let mut added = 0u32;
    let mut removed = 0u32;
    for op in &ops {
        match op {
            Op::Add(_) => added += 1,
            Op::Remove(_) => removed += 1,
            Op::Context(_, _) => {}
        }
    }

    FileDiff {
        path,
        status,
        added,
        removed,
        hunks,
    }
}

// --- A small LCS line diff (no extra deps) --------------------------------

#[derive(Clone, Copy)]
enum Op {
    /// Context line: (old index, new index) into the respective line vectors.
    Context(usize, usize),
    /// Removed line at the given old index.
    Remove(usize),
    /// Added line at the given new index.
    Add(usize),
}

/// Classic LCS dynamic-programming line diff. Returns an edit script in order.
fn diff_lines(old: &[String], new: &[String]) -> Vec<Op> {
    let n = old.len();
    let m = new.len();

    // lcs[i][j] = length of LCS of old[i..] and new[j..].
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            ops.push(Op::Context(i, j));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(Op::Remove(i));
            i += 1;
        } else {
            ops.push(Op::Add(j));
            j += 1;
        }
    }
    while i < n {
        ops.push(Op::Remove(i));
        i += 1;
    }
    while j < m {
        ops.push(Op::Add(j));
        j += 1;
    }
    ops
}

/// Group an edit script into unified-diff hunks with `context` lines of
/// surrounding context around each run of changes.
fn group_hunks(ops: &[Op], old: &[String], new: &[String], context: usize) -> Vec<Hunk> {
    // Indices of ops that are changes (non-context).
    let change_idxs: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| !matches!(op, Op::Context(_, _)))
        .map(|(i, _)| i)
        .collect();
    if change_idxs.is_empty() {
        return Vec::new();
    }

    // Merge change runs whose context windows overlap into hunk op-ranges.
    let mut ranges: Vec<(usize, usize)> = Vec::new(); // [start, end) over `ops`
    for &ci in &change_idxs {
        let start = ci.saturating_sub(context);
        let end = (ci + context + 1).min(ops.len());
        match ranges.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => ranges.push((start, end)),
        }
    }

    let mut hunks = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        let slice = &ops[start..end];
        let mut lines = Vec::with_capacity(slice.len());

        // 1-based starting line numbers for the hunk header.
        let mut old_start = 0u32;
        let mut new_start = 0u32;
        let mut old_len = 0u32;
        let mut new_len = 0u32;
        let mut header_context = String::new();

        for op in slice {
            match *op {
                Op::Context(oi, ni) => {
                    if old_start == 0 {
                        old_start = (oi + 1) as u32;
                    }
                    if new_start == 0 {
                        new_start = (ni + 1) as u32;
                    }
                    old_len += 1;
                    new_len += 1;
                    lines.push(DiffLine {
                        kind: LineKind::Context,
                        old_no: Some((oi + 1) as u32),
                        new_no: Some((ni + 1) as u32),
                        text: old[oi].clone(),
                    });
                }
                Op::Remove(oi) => {
                    if old_start == 0 {
                        old_start = (oi + 1) as u32;
                    }
                    old_len += 1;
                    lines.push(DiffLine {
                        kind: LineKind::Remove,
                        old_no: Some((oi + 1) as u32),
                        new_no: None,
                        text: old[oi].clone(),
                    });
                }
                Op::Add(ni) => {
                    if new_start == 0 {
                        new_start = (ni + 1) as u32;
                    }
                    new_len += 1;
                    lines.push(DiffLine {
                        kind: LineKind::Add,
                        old_no: None,
                        new_no: Some((ni + 1) as u32),
                        text: new[ni].clone(),
                    });
                }
            }
        }

        // header_context: nearest preceding non-blank old line before the hunk.
        let first_old = match slice[0] {
            Op::Context(oi, _) | Op::Remove(oi) => Some(oi),
            Op::Add(_) => None,
        };
        if let Some(oi) = first_old {
            let mut k = oi;
            while k > 0 {
                k -= 1;
                if !old[k].trim().is_empty() {
                    header_context = old[k].clone();
                    break;
                }
            }
        }

        hunks.push(Hunk {
            old_start,
            old_len,
            new_start,
            new_len,
            header_context,
            lines,
        });
    }

    hunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BookmarkKind;

    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixture/repo")
    }

    #[test]
    fn loads_fixture_snapshot() {
        let loaded = open(&fixture_path()).expect("open fixture");
        let snap = snapshot(&loaded).expect("snapshot");

        // jj's default-log revset (present(@) | ancestors(immutable_heads().., 2)
        // | trunk()) yields 6 rows in jj-log order: @, wip, refactor, experiment
        // (fork), feat[main], root. The intermediate immutable ancestors (docs,
        // init) are elided — the 2-generation ancestors limit drops them, which
        // shows up as an Indirect (elided) edge toward the root.
        assert_eq!(snap.nodes.len(), 6, "expected 6 revisions");
        assert_eq!(snap.workspace_name, "default");
        assert_eq!(snap.repo_name, "repo");

        // Elision: at least one Indirect edge exists (the dropped immutable
        // ancestors between feat and root), which the graph renderer draws as
        // the "(elided revisions)" marker.
        let has_indirect = snap
            .nodes
            .iter()
            .flat_map(|n| &n.parents)
            .any(|p| p.edge_type == EdgeType::Indirect);
        assert!(
            has_indirect,
            "expected an Indirect (elided) edge for the dropped ancestors"
        );

        // Ordering: the working copy must be first (jj prioritizes the
        // working-copy branch), and the side branch `experiment` must come
        // after `refactor` and at/before `feat` (it forks off `feat`).
        assert!(
            snap.nodes[0].is_working_copy,
            "first node should be the working copy"
        );
        let pos = |pred: &dyn Fn(&CommitNode) -> bool| {
            snap.nodes.iter().position(|n| pred(n)).expect("node present")
        };
        let refactor_pos = pos(&|n| n.description.starts_with("refactor"));
        let experiment_pos = pos(&|n| n.bookmarks.iter().any(|b| b.name == "experiment"));
        let feat_pos = pos(&|n| n.description.starts_with("feat:"));
        assert!(
            refactor_pos < experiment_pos,
            "experiment ({experiment_pos}) should come after refactor ({refactor_pos})"
        );
        assert!(
            experiment_pos <= feat_pos,
            "experiment ({experiment_pos}) should come at or before feat ({feat_pos})"
        );

        // Working copy: empty description, flagged, NOT empty (has live edits:
        // src/main.rs modified, src/parser.rs deleted).
        let wc = snap.working_copy().expect("working-copy node present");
        assert!(wc.is_working_copy);
        assert!(wc.description.is_empty(), "wc description should be empty");
        assert!(!wc.is_empty, "wc has file changes, so it is not empty");
        // change_id is rendered in jj's reverse-hex alphabet (z-k), e.g. starts
        // with a letter, never a hex digit.
        assert!(
            wc.change_id.chars().next().is_some_and(|c| ('k'..='z').contains(&c)),
            "change_id should be reverse-hex (got {:?})",
            wc.change_id,
        );
        assert_eq!(snap.wc_commit_id.as_deref(), Some(wc.commit_id.as_str()));

        // feat: immutable, carries the `main` local bookmark.
        let feat = snap
            .nodes
            .iter()
            .find(|n| n.description.starts_with("feat:"))
            .expect("feat node present");
        assert!(feat.is_immutable, "feat should be immutable");
        let has_main = feat
            .bookmarks
            .iter()
            .any(|b| b.name == "main" && b.kind == BookmarkKind::Local);
        assert!(has_main, "feat should carry the `main` bookmark");

        // experiment: a local bookmark; root is immutable.
        let experiment = snap
            .nodes
            .iter()
            .find(|n| n.bookmarks.iter().any(|b| b.name == "experiment"))
            .expect("experiment bookmark present");
        assert!(!experiment.is_immutable, "experiment should be mutable");

        let root = snap
            .nodes
            .last()
            .expect("at least one node");
        assert!(root.is_immutable, "root should be immutable");
    }

    #[test]
    fn working_copy_diff_shape() {
        let loaded = open(&fixture_path()).expect("open fixture");
        let wc = loaded.wc_commit_id_hex().expect("wc id");
        let diff = commit_diff(&loaded, &wc).expect("diff");

        let main_rs = diff
            .files
            .iter()
            .find(|f| f.path == "src/main.rs")
            .expect("src/main.rs in diff");
        assert_eq!(main_rs.status, ChangeStatus::Modified);

        let parser_rs = diff
            .files
            .iter()
            .find(|f| f.path == "src/parser.rs")
            .expect("src/parser.rs in diff");
        assert_eq!(parser_rs.status, ChangeStatus::Deleted);
        assert!(parser_rs.removed > 0);
    }
}
