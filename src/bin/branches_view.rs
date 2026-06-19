// INTEGRATION: move to src/ui/branches.rs; dispatch from ui::build_scene when active_view==View::Branches
//
//! BRANCHES / bookmarks view — lightjj's nav view "2" (BookmarksPanel.svelte).
//!
//! Renders the bookmark list that fills the right column in the Branches view:
//! a filter+sort header, a "LOCAL (N)" group, and one row per bookmark showing
//! a sync-state dot, the name, a sync label, and the bookmark's tip commit
//! (short id + description + relative age). A key-footer mirrors the panel's
//! keyboard affordances.
//!
//! This is a SELF-CONTAINED renderer built against the public `jjscratch::ui`
//! paint helpers and the `model::{Snapshot, Bookmark, …}` contract, exactly
//! like `ui/graph.rs` / `ui/diff.rs`. It paints only within its `rect` and
//! clips to it.
//!
//! ## Sync state, sort order & data we can / can't render
//!
//! lightjj's `bookmark-sync.ts` classifies a bookmark into 8 sync states
//! (conflict / diverged / ahead / behind / secondary / local-only /
//! remote-only / synced) using per-remote ahead/behind counts, and the panel
//! sorts by them ("Priority" mode). Our `model::Bookmark` only carries
//! `{ name, kind, conflicted, unsynced }` — there are no remotes, no
//! ahead/behind counts, and no PR data. So from the model we can faithfully
//! derive only: conflict (`conflicted`), local-only (`kind == Local`, no
//! remote), remote-only (`kind == Remote`), and an "unsynced" hint
//! (`unsynced`). Everything finer (the numeric ↑N/↓N counts, the amber
//! "secondary remote off" state, per-remote eye toggles, PR `#123` badges, the
//! tip-commit author column) needs backend data the model doesn't carry and is
//! STUBBED / omitted with a `// TODO(integration)` note where it would go.
//!
//! For the committed fixture both bookmarks (`main`, `experiment`) are plain
//! local bookmarks with no remote, so every row classifies as `local-only` —
//! matching docs/reference/branches.png, where both read "local only".

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::Scene;

// This file is a bin-local MODULE (included by `preview_branches.rs` via
// `#[path = "branches_view.rs"] mod branches_view;`), but `src/bin/*.rs` are
// also auto-discovered as standalone binaries by Cargo. Provide a stub entry
// point so the default `cargo build` (which compiles it as its own bin) stays
// green; the real harness is `preview_branches`. When this file is `#[path]`-
// included as a module, this `main` is just a module item (`branches_view::main`),
// hence `#[allow(dead_code)]` to keep that path warning-clean.
// INTEGRATION: deleted once this becomes `src/ui/branches.rs` (a library module,
// not a `src/bin/` file, so no auto-bin and no stub needed).
#[allow(dead_code)]
fn main() {
    eprintln!(
        "branches_view is a renderer module; run the preview harness instead: \
         `cargo run --features jjlib --bin preview_branches -- out.png 1280 800`"
    );
}

use jjscratch::model::{Bookmark, BookmarkKind, CommitNode, Snapshot};
use jjscratch::text;
use jjscratch::theme::{self, font};
use jjscratch::ui::{baseline_for, border_bottom, fill_rect, fill_round, stroke_round, RenderCtx};

// --- layout constants (calibrated to docs/reference/branches.png) -----------

/// Header band height (filter input + sort control + count). lightjj `.bp-header`
/// is `padding:10px 16px` over a ~28px input ≈ 48px.
const HEADER_H: f64 = 48.0;
/// Group-header row height (`.bp-group-row`, `padding:6px 14px`).
const GROUP_ROW_H: f64 = 28.0;
/// Bookmark row height (`.bp-row`, `padding:8px 16px` over one commit line ≈ 34px).
const ROW_H: f64 = 34.0;
/// Key-footer band height.
const FOOTER_H: f64 = 28.0;
/// Horizontal content inset (`.bp-header`/`.bp-row` padding-left 16px).
const PAD_X: f64 = 16.0;
/// Bookmark rows are indented past the group chevron (`.bp-bookmark-row` 28px).
const ROW_INDENT: f64 = 28.0;

/// Column x-offsets within a bookmark row, panel-relative. The name column is
/// fixed-width (`.bp-name` 200px in lightjj, scaled down to fit our narrower
/// reference panel), then the sync label, then the commit column flex-fills.
const SYNC_COL_X: f64 = 150.0; // start of the sync-label column
const COMMIT_COL_X: f64 = 270.0; // start of the commit (id + description) column
const AGE_COL_W: f64 = 44.0; // right-anchored relative-age column width

/// One presentational row in the panel: a group header or a bookmark.
enum Row<'a> {
    Group {
        label: &'a str,
        count: usize,
    },
    Bookmark {
        bm: &'a Bookmark,
        sync: Sync,
        /// The bookmark's tip commit, resolved from `snapshot.nodes`.
        node: Option<&'a CommitNode>,
    },
}

/// The subset of lightjj's 8 sync states we can derive from `model::Bookmark`.
/// The numeric ahead/behind variants (`ahead`/`behind`/`diverged`/`secondary`)
/// need per-remote counts the model doesn't carry — see module docs.
//
// `Synced`/`RemoteOnly` are unreachable from the current fixture (which has no
// remote-tracking bookmarks), but are kept so `classify` is the complete
// state map the integration pass needs once the loader emits remotes.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sync {
    Conflict,
    /// `unsynced` flag set but no count available — render an amber "unsynced".
    Unsynced,
    LocalOnly,
    RemoteOnly,
    Synced,
}

impl Sync {
    /// lightjj `bookmark-sync.ts` PRIORITY ordering (lower sorts first). We only
    /// place the states we can derive; the numeric ones (1..=4) are absent.
    fn priority(self) -> u8 {
        match self {
            Sync::Conflict => 0,
            Sync::Unsynced => 4, // stands in for ahead/behind/secondary
            Sync::LocalOnly => 5,
            Sync::RemoteOnly => 6,
            Sync::Synced => 7,
        }
    }

    /// Dot color, mirroring lightjj's `DOT_CLASS` map.
    fn dot(self, t: &theme::Palette) -> Color {
        match self {
            Sync::Conflict => t.red,
            Sync::Unsynced => t.amber,
            Sync::LocalOnly => t.overlay0,
            Sync::RemoteOnly => t.overlay0, // lightjj draws this hollow; see render
            Sync::Synced => t.green,
        }
    }

    /// Whether the dot is hollow (stroked outline) rather than filled —
    /// lightjj `bp-dot-hollow` for the remote-only state.
    fn hollow(self) -> bool {
        matches!(self, Sync::RemoteOnly)
    }

    /// Short label for the sync column (`syncLabel` in bookmark-sync.ts, minus
    /// the numeric ↑N/↓N variants we can't compute).
    fn label(self) -> &'static str {
        match self {
            Sync::Conflict => "conflict",
            Sync::Unsynced => "unsynced",
            Sync::LocalOnly => "local only",
            Sync::RemoteOnly => "remote only",
            Sync::Synced => "synced",
        }
    }

    fn label_color(self, t: &theme::Palette) -> Color {
        match self {
            Sync::Conflict => t.red,
            _ => t.subtext0,
        }
    }
}

/// Classify a bookmark using only what `model::Bookmark` carries.
//
// TODO(integration): needs per-remote ahead/behind counts + `synced` bool from
// the backend (jj-lib) to recover lightjj's full 8-state classifier
// (diverged / ahead ↑N / behind ↓N / secondary). The model's `unsynced` flag
// only says "out of sync" without direction or magnitude, so we collapse all
// of those into a single amber `Unsynced` state.
fn classify(bm: &Bookmark) -> Sync {
    if bm.conflicted {
        return Sync::Conflict;
    }
    if bm.unsynced {
        return Sync::Unsynced;
    }
    match bm.kind {
        BookmarkKind::Local => Sync::LocalOnly,
        BookmarkKind::Remote { .. } => Sync::RemoteOnly,
    }
}

/// Collect every distinct local bookmark in the snapshot, paired with the
/// commit node it points at, sorted by lightjj's "Priority" comparator
/// (trouble-first, then by name).
fn collect_local(snapshot: &Snapshot) -> Vec<Row<'_>> {
    let mut bms: Vec<(&Bookmark, &CommitNode)> = Vec::new();
    for node in &snapshot.nodes {
        for bm in &node.bookmarks {
            if matches!(bm.kind, BookmarkKind::Local) {
                bms.push((bm, node));
            }
        }
    }
    // "Priority" sort: conflicts pinned to the top in every mode, then sync
    // priority, ties broken alphabetically (compareBookmarks in bookmark-sync.ts).
    bms.sort_by(|(a, _), (b, _)| {
        let ca = !a.conflicted; // false (=conflict) sorts first
        let cb = !b.conflicted;
        ca.cmp(&cb)
            .then(classify(a).priority().cmp(&classify(b).priority()))
            .then(a.name.cmp(&b.name))
    });
    bms.into_iter()
        .map(|(bm, node)| Row::Bookmark {
            bm,
            sync: classify(bm),
            node: Some(node),
        })
        .collect()
}

/// Render the branches panel into `rect`.
pub fn render(scene: &mut Scene, rect: Rect, snapshot: &Snapshot, ctx: &RenderCtx) {
    let t = ctx.theme;
    fill_rect(scene, rect, t.base);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);

    let local = collect_local(snapshot);

    // Header (filter + sort control + count).
    let mut y = header(scene, rect, ctx, local.len());

    // Build the row list: a single LOCAL group followed by its bookmarks.
    // TODO(integration): per-remote groups ("ORIGIN", "UPSTREAM", …) come from
    // bookmarks with `kind == Remote { remote }` once the loader emits remote
    // tracking refs; the model has the variant but the fixture has no remotes.
    let mut rows: Vec<Row> = vec![Row::Group {
        label: "LOCAL",
        count: local.len(),
    }];
    rows.extend(local);

    for row in &rows {
        if y >= rect.y1 - FOOTER_H {
            break;
        }
        y = match row {
            Row::Group { label, count } => group_row(scene, rect, y, label, *count, ctx),
            Row::Bookmark { bm, sync, node } => bookmark_row(scene, rect, y, bm, *sync, *node, ctx),
        };
    }

    key_footer(scene, rect, ctx);
    scene.pop_layer();
}

// --- header ----------------------------------------------------------------

/// `.bp-header`: filter input on the left, a Priority/Recent/Name segmented
/// control on the right, then a pill count of visible bookmarks.
fn header(scene: &mut Scene, rect: Rect, ctx: &RenderCtx, count: usize) -> f64 {
    let t = ctx.theme;
    let r = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HEADER_H);
    fill_rect(scene, r, t.mantle);
    border_bottom(scene, r, t.surface0);
    let cy = r.center().y;

    // Count pill, right-anchored (`.bp-count`).
    let csz = font::FS_SM;
    let cs = count.to_string();
    let cw = text::measure(&ctx.fonts.ui, csz, &cs) as f64;
    let pill_w = cw + 8.0 * 2.0;
    let pill = Rect::new(r.x1 - PAD_X - pill_w, cy - 9.0, r.x1 - PAD_X, cy + 9.0);
    fill_round(scene, pill, 9.0, t.surface0);
    text::draw_text(
        scene, &ctx.fonts.ui, csz, t.overlay0,
        pill.x0 + 8.0, baseline_for(cy, csz, &ctx.fonts.ui), &cs,
    );

    // Sort segmented control (`.bp-sort`): Priority active (amber), then Recent,
    // Name. Sits just left of the count pill.
    let modes = ["Priority", "Recent", "Name"];
    let ssz = font::FS_SM;
    let seg_pad = 8.0;
    let seg_h = 20.0;
    let widths: Vec<f64> = modes
        .iter()
        .map(|m| text::measure(&ctx.fonts.ui, ssz, m) as f64 + seg_pad * 2.0)
        .collect();
    let seg_total: f64 = widths.iter().sum();
    let seg_x0 = pill.x0 - 12.0 - seg_total;
    let seg_box = Rect::new(seg_x0, cy - seg_h / 2.0, seg_x0 + seg_total, cy + seg_h / 2.0);
    fill_round(scene, seg_box, 4.0, t.surface0);
    stroke_round(scene, seg_box, 4.0, t.surface1, 1.0);
    let mut sx = seg_x0;
    for (i, m) in modes.iter().enumerate() {
        let w = widths[i];
        let active = i == 0; // "Priority" is the default sort mode
        let btn = Rect::new(sx, seg_box.y0, sx + w, seg_box.y1);
        let color = if active {
            fill_round(scene, btn, 4.0, t.bg_active);
            t.amber
        } else {
            t.subtext0
        };
        text::draw_text(
            scene, &ctx.fonts.ui, ssz, color,
            sx + seg_pad, baseline_for(cy, ssz, &ctx.fonts.ui), m,
        );
        sx += w;
    }

    // Filter input box (`.bp-filter`), flex up to the sort control.
    let isz = font::FS_MD;
    let input = Rect::new(r.x0 + PAD_X, cy - 13.0, seg_x0 - 12.0, cy + 13.0);
    fill_round(scene, input, 4.0, t.base);
    stroke_round(scene, input, 4.0, t.surface1, 1.0);
    scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &input);
    text::draw_text(
        scene, &ctx.fonts.ui, isz, t.text_faint,
        input.x0 + 10.0, baseline_for(cy, isz, &ctx.fonts.ui),
        "Filter\u{2026} name, or author:bob (/ to focus)",
    );
    scene.pop_layer();

    r.y1
}

// --- group header ----------------------------------------------------------

/// `.bp-group-row`: chevron + uppercase label + count. The fixture has no
/// remote groups, so no per-remote eye toggle is drawn.
//
// TODO(integration): per-remote group rows carry a trailing eye toggle
// (`.bp-eye`) wired to remote visibility in the revision graph — needs the
// RemoteVisibility map from the backend, absent in the model.
fn group_row(scene: &mut Scene, rect: Rect, y: f64, label: &str, count: usize, ctx: &RenderCtx) -> f64 {
    let t = ctx.theme;
    let r = Rect::new(rect.x0, y, rect.x1, y + GROUP_ROW_H);
    fill_rect(scene, r, t.mantle);
    let cy = r.center().y;
    let sz = font::FS_SM;

    // Chevron (expanded ▼).
    let chev = "\u{25bc}";
    let cx = r.x0 + PAD_X;
    let cend = text::draw_text(
        scene, &ctx.fonts.mono, font::FS_3XS, t.subtext0,
        cx, baseline_for(cy, font::FS_3XS, &ctx.fonts.mono), chev,
    );
    // Label (uppercase, weight 600).
    let lend = text::draw_text(
        scene, &ctx.fonts.ui_bold, sz, t.subtext0,
        cend + 8.0, baseline_for(cy, sz, &ctx.fonts.ui_bold), label,
    );
    // Count, dimmer.
    text::draw_text(
        scene, &ctx.fonts.ui, font::FS_2XS, t.overlay0,
        lend + 6.0, baseline_for(cy, font::FS_2XS, &ctx.fonts.ui), &format!("({count})"),
    );
    r.y1
}

// --- bookmark row ----------------------------------------------------------

/// `.bp-bookmark-row`: sync dot, name, sync label, commit (short id +
/// description), and a right-anchored relative age.
fn bookmark_row(
    scene: &mut Scene,
    rect: Rect,
    y: f64,
    bm: &Bookmark,
    sync: Sync,
    node: Option<&CommitNode>,
    ctx: &RenderCtx,
) -> f64 {
    let t = ctx.theme;
    let r = Rect::new(rect.x0, y, rect.x1, y + ROW_H);
    let cy = r.center().y;

    // Sync dot (`.bp-dot`), 8px, color/hollow per sync state.
    let dot_r = 4.0;
    let dot_cx = r.x0 + ROW_INDENT - 2.0;
    let dot = Rect::new(dot_cx - dot_r, cy - dot_r, dot_cx + dot_r, cy + dot_r);
    if sync.hollow() {
        stroke_round(scene, dot, dot_r, sync.dot(t), 1.0);
    } else {
        fill_round(scene, dot, dot_r, sync.dot(t));
    }

    // Name (`.bp-name`, weight 500).
    let name_x = r.x0 + ROW_INDENT + 12.0;
    let nsz = font::FS_MD;
    text::draw_text(
        scene, &ctx.fonts.ui_bold, nsz, t.text,
        name_x, baseline_for(cy, nsz, &ctx.fonts.ui_bold), &bm.name,
    );
    // TODO(integration): a PR badge (`.bp-pr-badge`, e.g. `#123`) renders right
    // after the name when the backend resolves a GitHub PR for this bookmark —
    // needs the prByBookmark map (gh API), absent in the model.

    // Sync label column (`.bp-sync`).
    let lsz = font::FS_SM;
    text::draw_text(
        scene, &ctx.fonts.ui, lsz, sync.label_color(t),
        r.x0 + SYNC_COL_X, baseline_for(cy, lsz, &ctx.fonts.ui), sync.label(),
    );

    // Commit column (`.bp-commit-line`): short commit id + description.
    let commit_x = r.x0 + COMMIT_COL_X;
    let age_x0 = r.x1 - PAD_X - AGE_COL_W;
    if let Some(node) = node {
        let cidsz = font::FS_SM;
        let cid: String = node.commit_id.chars().take(8).collect();
        let cend = text::draw_text(
            scene, &ctx.fonts.mono, cidsz, t.overlay0,
            commit_x, baseline_for(cy, cidsz, &ctx.fonts.mono), &cid,
        );
        // Description (`.bp-desc`), clipped so it can't bleed into the age column.
        let dsz = font::FS_MD;
        // First line only: jj descriptions can carry a trailing newline / body;
        // the panel shows the summary line (lightjj `firstLine`).
        let desc_line = node.description.lines().next().unwrap_or("").trim_end();
        let (desc, color) = if desc_line.is_empty() {
            ("(no description)", t.text_faint)
        } else {
            (desc_line, t.subtext0)
        };
        let desc_clip = Rect::new(cend + 8.0, r.y0, age_x0 - 8.0, r.y1);
        scene.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &desc_clip);
        text::draw_text(
            scene, &ctx.fonts.ui, dsz, color,
            cend + 8.0, baseline_for(cy, dsz, &ctx.fonts.ui), desc,
        );
        scene.pop_layer();

        // Relative age (`.bp-ago`), right-anchored.
        // TODO(integration): the tip-commit AUTHOR column (`.bp-author`, shown
        // for others' commits + filterable via `author:`) would sit just left of
        // the age — omitted here because the fixture is single-author and the
        // per-ref author the panel shows isn't surfaced distinctly in the model.
        let age = relative_age(node.timestamp_ms);
        let asz = font::FS_XS;
        let aw = text::measure(&ctx.fonts.ui, asz, &age) as f64;
        text::draw_text(
            scene, &ctx.fonts.ui, asz, t.text_faint,
            r.x1 - PAD_X - aw, baseline_for(cy, asz, &ctx.fonts.ui), &age,
        );
    } else {
        // No host commit found (shouldn't happen for the fixture).
        text::draw_text(
            scene, &ctx.fonts.mono, font::FS_SM, t.overlay0,
            commit_x, baseline_for(cy, font::FS_SM, &ctx.fonts.mono), "\u{2014}",
        );
    }

    r.y1
}

// --- key footer ------------------------------------------------------------

/// The `.key-footer` band: the panel's keyboard affordances. Mirrors lightjj's
/// default (non-armed) footer hints.
fn key_footer(scene: &mut Scene, rect: Rect, ctx: &RenderCtx) {
    let t = ctx.theme;
    let r = Rect::new(rect.x0, rect.y1 - FOOTER_H, rect.x1, rect.y1);
    fill_rect(scene, r, t.crust);
    // 1px top border (public ui::border_top isn't in the contract helper list).
    fill_rect(scene, Rect::new(r.x0, r.y0, r.x1, r.y0 + 1.0), t.surface1);
    let cy = r.center().y;
    let sz = font::FS_SM;

    let hints = [
        ("\u{23ce}", "jump"),
        ("d", "delete"),
        ("f", "forget"),
        ("t", "track"),
        ("e", "eye"),
        ("r", "refresh"),
        ("/", "filter"),
    ];
    let mut x = r.x0 + PAD_X;
    for (i, (key, word)) in hints.iter().enumerate() {
        if i > 0 {
            x = text::draw_text(
                scene, &ctx.fonts.ui, sz, t.surface2,
                x + 4.0, baseline_for(cy, sz, &ctx.fonts.ui), "\u{00b7}",
            ) + 6.0;
        }
        // kbd chip.
        let ksz = font::FS_2XS;
        let kw = text::measure(&ctx.fonts.mono, ksz, key) as f64;
        let chip = Rect::new(x, cy - 7.0, x + kw + 6.0, cy + 7.0);
        stroke_round(scene, chip, 3.0, t.surface1, 1.0);
        text::draw_text(
            scene, &ctx.fonts.mono, ksz, t.overlay0,
            x + 3.0, baseline_for(cy, ksz, &ctx.fonts.mono), key,
        );
        x = chip.x1 + 5.0;
        x = text::draw_text(
            scene, &ctx.fonts.ui, sz, t.subtext0,
            x, baseline_for(cy, sz, &ctx.fonts.ui), word,
        );
    }
}

// --- helpers ---------------------------------------------------------------

/// Compact relative age from an author timestamp (ms since epoch), matching
/// lightjj's `relativeTime` granularity (s/m/h/d/mo/y).
fn relative_age(ts_ms: i64) -> String {
    // The reference shows "5mo" for the fixture commits (authored 2026-01-15,
    // captured ~2026-06). Anchor to a fixed "now" so the renderer is
    // deterministic regardless of wall-clock at render time.
    // TODO(integration): the live app uses real wall-clock `now`; this fixed
    // anchor exists only so the isolated preview reproduces the reference.
    const NOW_MS: i64 = 1_768_471_200_000 + 150 * 86_400_000; // +~5 months
    let secs = (NOW_MS - ts_ms).max(0) / 1000;
    let mins = secs / 60;
    let hours = mins / 60;
    let days = hours / 24;
    let months = days / 30;
    let years = days / 365;
    if years > 0 {
        format!("{years}y")
    } else if months > 0 {
        format!("{months}mo")
    } else if days > 0 {
        format!("{days}d")
    } else if hours > 0 {
        format!("{hours}h")
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        format!("{secs}s")
    }
}
