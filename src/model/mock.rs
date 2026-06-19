//! A mock [`Snapshot`]/[`CommitDiff`] mirroring the committed fixture repo
//! (`fixture/repo`). Lets the UI renderer develop against real-shaped data with
//! no jj-lib dependency. Keep these ids in sync with the fixture if it changes
//! (regenerate via `scripts/make-fixture.sh` then re-read with `jj log`).

use super::*;

const TS: i64 = 1_768_471_200_000; // 2026-01-15T10:00:00Z

fn node(
    change_id: &str,
    change_prefix_len: usize,
    commit_id: &str,
    commit_prefix_len: usize,
    description: &str,
    parents: &[&str],
) -> CommitNode {
    CommitNode {
        change_id: change_id.into(),
        change_prefix_len,
        commit_id: commit_id.into(),
        commit_prefix_len,
        description: description.into(),
        author_name: "Ada Lovelace".into(),
        author_email: "ada@example.com".into(),
        timestamp_ms: TS,
        is_working_copy: false,
        is_immutable: false,
        is_divergent: false,
        has_conflict: false,
        is_hidden: false,
        is_empty: false,
        parents: parents
            .iter()
            .map(|p| GraphParent { commit_id: (*p).into(), edge_type: EdgeType::Direct })
            .collect(),
        bookmarks: Vec::new(),
    }
}

fn local_bookmark(name: &str) -> Bookmark {
    Bookmark { name: name.into(), kind: BookmarkKind::Local, conflicted: false, unsynced: false }
}

/// The fixture snapshot, in display (topological) order: @, wip, refactor,
/// experiment fork, then the immutable trunk (feat[main], docs, init, root).
pub fn snapshot() -> Snapshot {
    let mut wc = node(
        "lklqykmzmoppymtspqtnyopooonlkuzn", 2,
        "e1d4d4b4cd3a112b7e030c75684edad842e702d2", 2,
        "", &["fe57d6c53c3aa59b8fb9dabdfb52d2e3d7cb4e0d"],
    );
    wc.is_working_copy = true;

    let wip = node(
        "nvvxqotkykqnlrlknvqprwkywyyltosw", 1,
        "fe57d6c53c3aa59b8fb9dabdfb52d2e3d7cb4e0d", 2,
        "wip: cli flags", &["b26934293642e06cc4e242fd9beccfa3ca47b5e9"],
    );

    let refactor = node(
        "yuttpwnnozuprqrzzwrwnxwwzyrqmplr", 1,
        "b26934293642e06cc4e242fd9beccfa3ca47b5e9", 1,
        "refactor: document and test parser",
        &["e2d8d951ea64af1a60574c4e8bedcd8817b596c8"],
    );

    let mut experiment = node(
        "lxtsspsuxlstturvtoynkzkoskoqrytr", 2,
        "f5c69e1b325a80a782ed19c2a5379f08a831f2ab", 2,
        "experiment: alternative tokenizer",
        &["e2d8d951ea64af1a60574c4e8bedcd8817b596c8"],
    );
    experiment.bookmarks.push(local_bookmark("experiment"));

    let mut feat = node(
        "ppxmzrnuvoovytumzqwyrwrrnynkssux", 1,
        "e2d8d951ea64af1a60574c4e8bedcd8817b596c8", 2,
        "feat: add whitespace parser",
        &["0379b900130300d6cca71992f628259cc31a63c4"],
    );
    feat.is_immutable = true;
    feat.bookmarks.push(local_bookmark("main"));

    let mut docs = node(
        "wmxvvywsunrryuquxxntqxrxnrlyrmwy", 1,
        "0379b900130300d6cca71992f628259cc31a63c4", 2,
        "docs: add building section",
        &["42a32a6fc16abbc6c8f4134fadae9fa1907bb903"],
    );
    docs.is_immutable = true;

    let mut init = node(
        "qpztqtznpvqknvllzmwrkpntsuqlyrto", 1,
        "42a32a6fc16abbc6c8f4134fadae9fa1907bb903", 3,
        "init project",
        &["0000000000000000000000000000000000000000"],
    );
    init.is_immutable = true;

    let mut root = node(
        "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", 1,
        "0000000000000000000000000000000000000000", 1,
        "", &[],
    );
    root.is_immutable = true;
    root.is_empty = true;

    Snapshot {
        repo_name: "demo".into(),
        workspace_name: "default".into(),
        wc_commit_id: Some(wc.commit_id.clone()),
        nodes: vec![wc, wip, refactor, experiment, feat, docs, init, root],
    }
}

/// The diff for the working copy `@` (matches the fixture's live edits:
/// `src/main.rs` modified, `src/parser.rs` deleted).
pub fn working_copy_diff() -> CommitDiff {
    let main_rs = FileDiff {
        path: "src/main.rs".into(),
        status: ChangeStatus::Modified,
        added: 3,
        removed: 0,
        hunks: vec![Hunk {
            old_start: 4,
            old_len: 4,
            new_start: 4,
            new_len: 7,
            header_context: "fn main()".into(),
            lines: vec![
                ctx(6, 6, "    let tokens = parser::parse(&args.join(\" \"));"),
                ctx(7, 7, "    println!(\"{} tokens\", tokens.len());"),
                ctx(8, 8, "}"),
                add(9, ""),
                add(10, "// scratch: trying out a new entry point"),
                add(11, "fn run() {}"),
            ],
        }],
    };
    let parser_rs = FileDiff {
        path: "src/parser.rs".into(),
        status: ChangeStatus::Deleted,
        added: 0,
        removed: 14,
        hunks: vec![Hunk {
            old_start: 1,
            old_len: 14,
            new_start: 0,
            new_len: 0,
            header_context: String::new(),
            lines: vec![
                rem(1, "/// Split input into tokens, collapsing repeated whitespace."),
                rem(2, "pub fn parse(input: &str) -> Vec<&str> {"),
                rem(3, "    input.split_whitespace().filter(|s| !s.is_empty()).collect()"),
                rem(4, "}"),
                rem(5, ""),
                rem(6, "#[cfg(test)]"),
                rem(7, "mod tests {"),
                rem(8, "    use super::*;"),
                rem(9, "    #[test]"),
                rem(10, "    fn splits() {"),
                rem(11, "        assert_eq!(parse(\"a  b\"), vec![\"a\", \"b\"]);"),
                rem(12, "    }"),
                rem(13, "}"),
            ],
        }],
    };
    CommitDiff {
        commit_id: "e1d4d4b4cd3a112b7e030c75684edad842e702d2".into(),
        files: vec![main_rs, parser_rs],
    }
}

fn ctx(old: u32, new: u32, t: &str) -> DiffLine {
    DiffLine { kind: LineKind::Context, old_no: Some(old), new_no: Some(new), text: t.into() }
}
fn add(new: u32, t: &str) -> DiffLine {
    DiffLine { kind: LineKind::Add, old_no: None, new_no: Some(new), text: t.into() }
}
fn rem(old: u32, t: &str) -> DiffLine {
    DiffLine { kind: LineKind::Remove, old_no: Some(old), new_no: None, text: t.into() }
}
