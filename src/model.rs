//! The data contract between the backend (jj-lib loader) and the UI renderer.
//!
//! These are plain, render-ready value types — no jj-lib types leak through.
//! The UI consumes a [`Snapshot`] (+ [`CommitDiff`]); the backend produces them.
//! This decoupling is deliberate: the renderer builds against [`mock`] while the
//! real [`jjlib`](crate::model::jjlib) loader is written in parallel, and both
//! fill the SAME types.

pub mod mock;
#[cfg(feature = "jjlib")]
pub mod jjlib;

/// A bookmark/ref badge attached to a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub name: String,
    pub kind: BookmarkKind,
    /// jj `??` conflicted marker.
    pub conflicted: bool,
    /// jj `*` un-synced (ahead/behind) marker.
    pub unsynced: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookmarkKind {
    Local,
    /// `{remote}/{name}` remote-tracking bookmark.
    Remote { remote: String },
}

/// One commit node in the DAG, in the order `stream_graph` yields (topological,
/// children before parents).
#[derive(Clone, Debug)]
pub struct CommitNode {
    pub change_id: String, // hex
    /// How many leading chars of `change_id` are the unique prefix (highlighted).
    pub change_prefix_len: usize,
    pub commit_id: String, // hex
    pub commit_prefix_len: usize,
    pub description: String, // may be empty
    pub author_name: String,
    pub author_email: String,
    /// Millis since epoch (author time).
    pub timestamp_ms: i64,
    pub is_working_copy: bool,
    pub is_immutable: bool,
    pub is_divergent: bool,
    pub has_conflict: bool,
    pub is_hidden: bool,
    /// Whether the description body is empty (shows "(no description)").
    pub is_empty: bool,
    /// Parent commit ids (hex) — the outgoing graph edges for layout.
    pub parents: Vec<GraphParent>,
    pub bookmarks: Vec<Bookmark>,
}

/// A parent edge, distinguishing direct vs elided (indirect) ancestry so the
/// graph renderer can draw `~` elision.
#[derive(Clone, Debug)]
pub struct GraphParent {
    pub commit_id: String,
    pub edge_type: EdgeType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeType {
    Direct,
    Indirect,
    Missing,
}

/// Everything the UI needs to render the revision panel chrome + graph.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub repo_name: String,
    pub workspace_name: String,
    /// Topologically ordered (children before parents).
    pub nodes: Vec<CommitNode>,
    pub wc_commit_id: Option<String>,
}

impl Snapshot {
    pub fn revision_count(&self) -> usize {
        self.nodes.len()
    }
    pub fn working_copy(&self) -> Option<&CommitNode> {
        self.nodes.iter().find(|n| n.is_working_copy)
    }
}

// --- Diff model -----------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Remove,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// Trailing context shown in the hunk header (e.g. enclosing fn name).
    pub header_context: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct FileDiff {
    pub path: String,
    pub status: ChangeStatus,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<Hunk>,
}

/// The diff of one commit/target against its parent (what fills the diff panel).
#[derive(Clone, Debug)]
pub struct CommitDiff {
    /// The commit (hex) this diff is for, for cache keying.
    pub commit_id: String,
    pub files: Vec<FileDiff>,
}

impl CommitDiff {
    pub fn total_added(&self) -> u32 {
        self.files.iter().map(|f| f.added).sum()
    }
    pub fn total_removed(&self) -> u32 {
        self.files.iter().map(|f| f.removed).sum()
    }
}
