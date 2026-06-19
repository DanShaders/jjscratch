//! Graph lane layout: turn a topologically-ordered DAG ([`CommitNode`] list with
//! parent edges) into per-row lane assignments and connector segments, so the
//! graph renderer can draw continuous pipes WITHOUT relying on jj's ASCII output.
//!
//! The algorithm is a standard incremental lane assignment over the topological
//! order (children before parents). We maintain a list of "active lanes", each
//! tracking the commit id it is currently routing toward. When a node is emitted
//! we place it in the lane that was waiting for it (or a fresh lane), then replace
//! that lane's target with the node's first parent and append new lanes for
//! additional parents, emitting the gutter [`Cell`] connectors (vertical, the four
//! elbows, horizontal stubs, elided `~`) needed to route lanes between rows.
//!
//! Geometry (docs/spec/ui-spec.md §4.1): CELL_W = 10, ROW_H = 18, lane = col/2,
//! node center x = col*10 + 5. Lane color = `palette[lane % 8]`. This module is
//! PURE; it is unit-tested against hand-built and fixture-shaped DAGs below.

use crate::model::{CommitNode, EdgeType};

/// A connector cell kind within one row's gutter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Vertical,
    Horizontal,
    ElbowTopLeft,     // ╮
    ElbowTopRight,    // ╭
    ElbowBottomLeft,  // ╯
    ElbowBottomRight, // ╰
    Elided,           // ~
}

/// One laid-out row: which lane the node occupies, the node's column, and the
/// gutter cells (indexed by column) for connectors passing through this row.
#[derive(Clone, Debug)]
pub struct LayoutRow {
    /// Index into the input `nodes` slice.
    pub node_index: usize,
    /// Lane the node sits in (column = lane*2).
    pub lane: usize,
    /// Connector cells for this row, one per character column.
    pub cells: Vec<Cell>,
}

impl LayoutRow {
    /// Node column = lane * 2.
    pub fn node_col(&self) -> usize {
        self.lane * 2
    }
}

/// Full graph layout for a node list.
#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    pub rows: Vec<LayoutRow>,
    /// Max columns used (for sizing the gutter SVG width).
    pub width_cols: usize,
}

/// Set `cell` at column `col` in `cells`, growing the row with `Empty` as needed.
fn set_cell(cells: &mut Vec<Cell>, col: usize, cell: Cell) {
    if col >= cells.len() {
        cells.resize(col + 1, Cell::Empty);
    }
    cells[col] = cell;
}

/// Fill the cells strictly between columns `a` and `b` with `Horizontal`, without
/// stomping marks already placed there.
fn fill_horizontal(cells: &mut Vec<Cell>, a: usize, b: usize) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    for col in (lo + 1)..hi {
        if col >= cells.len() {
            cells.resize(col + 1, Cell::Empty);
        }
        if cells[col] == Cell::Empty {
            cells[col] = Cell::Horizontal;
        }
    }
}

/// Compute the lane layout via incremental lane assignment.
pub fn layout(nodes: &[CommitNode]) -> GraphLayout {
    // active[lane] = Some(commit_id this lane is currently routing toward), or None.
    let mut active: Vec<Option<String>> = Vec::new();
    let mut rows: Vec<LayoutRow> = Vec::with_capacity(nodes.len());
    let mut width_cols = 1usize;

    for (idx, node) in nodes.iter().enumerate() {
        // 1. Find the lane awaiting this commit, else the first free slot, else append.
        let my_lane = match active
            .iter()
            .position(|l| l.as_deref() == Some(node.commit_id.as_str()))
        {
            Some(l) => l,
            None => match active.iter().position(|l| l.is_none()) {
                Some(l) => l,
                None => {
                    active.push(None);
                    active.len() - 1
                }
            },
        };
        if my_lane >= active.len() {
            active.resize(my_lane + 1, None);
        }

        // Other lanes also awaiting us (children converging here = merge).
        let converging: Vec<usize> = active
            .iter()
            .enumerate()
            .filter(|(l, t)| *l != my_lane && t.as_deref() == Some(node.commit_id.as_str()))
            .map(|(l, _)| l)
            .collect();

        // 2. Carry every *other* active lane through this row as a vertical pipe.
        let mut cells: Vec<Cell> = Vec::new();
        for (lane, target) in active.iter().enumerate() {
            if lane == my_lane {
                continue;
            }
            if target.is_some() {
                set_cell(&mut cells, lane * 2, Cell::Vertical);
            }
        }
        // The node's own lane carries a vertical (renderer draws the glyph on top).
        set_cell(&mut cells, my_lane * 2, Cell::Vertical);

        // 3. Route converging children into my_lane with an elbow + horizontal stub.
        for &cl in &converging {
            if cl < my_lane {
                set_cell(&mut cells, cl * 2, Cell::ElbowBottomRight); // ╰
            } else {
                set_cell(&mut cells, cl * 2, Cell::ElbowBottomLeft); // ╯
            }
            fill_horizontal(&mut cells, cl * 2, my_lane * 2);
            active[cl] = None;
        }

        // 4. Assign this node's parents to lanes. First parent reuses my_lane;
        //    additional parents fork into fresh lanes.
        let mut first_parent_done = false;
        let mut new_forks: Vec<usize> = Vec::new();
        for p in &node.parents {
            if matches!(p.edge_type, EdgeType::Missing) {
                continue;
            }
            if !first_parent_done {
                active[my_lane] = Some(p.commit_id.clone());
                first_parent_done = true;
            } else {
                let lane = match active.iter().position(|l| l.is_none()) {
                    Some(l) => l,
                    None => {
                        active.push(None);
                        active.len() - 1
                    }
                };
                active[lane] = Some(p.commit_id.clone());
                new_forks.push(lane);
            }
        }
        if !first_parent_done {
            active[my_lane] = None;
        }

        // 5. Draw fork elbows for additional parents (node lane branching outward).
        for &lane in &new_forks {
            if lane > my_lane {
                set_cell(&mut cells, lane * 2, Cell::ElbowTopLeft); // ╮
            } else {
                set_cell(&mut cells, lane * 2, Cell::ElbowTopRight); // ╭
            }
            fill_horizontal(&mut cells, my_lane * 2, lane * 2);
        }

        // 6. Elision: an Indirect first-parent edge marks the node lane as an elided
        //    (dashed `~`) pipe so the renderer draws the gap.
        if let Some(first) = node
            .parents
            .iter()
            .find(|p| !matches!(p.edge_type, EdgeType::Missing))
        {
            if first.edge_type == EdgeType::Indirect {
                set_cell(&mut cells, my_lane * 2, Cell::Elided);
            }
        }

        // Trim trailing free lanes so the gutter stays tight.
        while matches!(active.last(), Some(None)) {
            active.pop();
        }

        width_cols = width_cols.max(cells.len());
        rows.push(LayoutRow {
            node_index: idx,
            lane: my_lane,
            cells,
        });
    }

    GraphLayout { rows, width_cols }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Bookmark, GraphParent};

    fn n(commit: &str, parents: &[(&str, EdgeType)]) -> CommitNode {
        CommitNode {
            change_id: format!("change-{commit}"),
            change_prefix_len: 1,
            commit_id: commit.into(),
            commit_prefix_len: 1,
            description: format!("desc {commit}"),
            author_name: "a".into(),
            author_email: "a@e".into(),
            timestamp_ms: 0,
            is_working_copy: false,
            is_immutable: false,
            is_divergent: false,
            has_conflict: false,
            is_hidden: false,
            is_empty: false,
            parents: parents
                .iter()
                .map(|(c, e)| GraphParent { commit_id: (*c).into(), edge_type: *e })
                .collect(),
            bookmarks: Vec::<Bookmark>::new(),
        }
    }

    #[test]
    fn linear_history_all_lane_zero() {
        let nodes = vec![
            n("c", &[("b", EdgeType::Direct)]),
            n("b", &[("a", EdgeType::Direct)]),
            n("a", &[]),
        ];
        let g = layout(&nodes);
        assert_eq!(g.rows.len(), 3);
        for r in &g.rows {
            assert_eq!(r.lane, 0, "linear history stays in lane 0");
            assert_eq!(r.cells[0], Cell::Vertical);
        }
        assert_eq!(g.width_cols, 1);
    }

    #[test]
    fn simple_merge_uses_two_lanes_with_elbow() {
        // b and c both have parent root. Display order: b, c, root.
        let nodes = vec![
            n("b", &[("root", EdgeType::Direct)]),
            n("c", &[("root", EdgeType::Direct)]),
            n("root", &[]),
        ];
        let g = layout(&nodes);
        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 1, "second child opens a new lane");
        assert_eq!(g.rows[2].lane, 0, "merge target lands in lane 0");
        let merge = &g.rows[2];
        assert!(
            merge
                .cells
                .iter()
                .any(|c| matches!(c, Cell::ElbowBottomLeft | Cell::ElbowBottomRight)),
            "merge row should draw a converging elbow, got {:?}",
            merge.cells
        );
        assert!(g.width_cols >= 2);
    }

    #[test]
    fn node_with_two_parents_forks_into_new_lane() {
        let nodes = vec![
            n("p", &[("p1", EdgeType::Direct), ("p2", EdgeType::Direct)]),
            n("p1", &[]),
            n("p2", &[]),
        ];
        let g = layout(&nodes);
        assert_eq!(g.rows[0].lane, 0);
        let fork = &g.rows[0];
        assert!(
            fork.cells
                .iter()
                .any(|c| matches!(c, Cell::ElbowTopLeft | Cell::ElbowTopRight)),
            "fork row should open a lane with an elbow, got {:?}",
            fork.cells
        );
        assert_eq!(g.rows[1].lane, 0, "first parent reuses lane 0");
        assert_eq!(g.rows[2].lane, 1, "second parent lands in lane 1");
    }

    #[test]
    fn indirect_edge_marks_elision() {
        let nodes = vec![n("c", &[("p", EdgeType::Indirect)]), n("p", &[])];
        let g = layout(&nodes);
        assert_eq!(
            g.rows[0].cells[0],
            Cell::Elided,
            "indirect first-parent edge marks the node lane elided"
        );
    }

    #[test]
    fn fixture_shaped_graph() {
        // Mirror mock::snapshot()'s shape: a linear main stack with one fork to
        // 'experiment' off the 'feat' commit, the experiment->feat edge elided.
        let nodes = vec![
            n("wc", &[("wip", EdgeType::Direct)]),
            n("wip", &[("refactor", EdgeType::Direct)]),
            n("refactor", &[("feat", EdgeType::Direct)]),
            n("experiment", &[("feat", EdgeType::Indirect)]),
            n("feat", &[("docs", EdgeType::Direct)]),
            n("docs", &[("init", EdgeType::Direct)]),
            n("init", &[("root", EdgeType::Direct)]),
            n("root", &[]),
        ];
        let g = layout(&nodes);
        assert_eq!(g.rows.len(), 8);

        assert_eq!(g.rows[0].lane, 0);
        assert_eq!(g.rows[1].lane, 0);
        assert_eq!(g.rows[2].lane, 0);
        // experiment has no awaiting lane (refactor already routes to feat in lane 0),
        // so it opens lane 1, and its edge to feat is elided.
        assert_eq!(g.rows[3].lane, 1, "experiment opens its own lane");
        assert_eq!(g.rows[3].cells[g.rows[3].node_col()], Cell::Elided);
        // feat lands in lane 0 (awaited by refactor); experiment's lane-1 pipe
        // converges into it with an elbow.
        assert_eq!(g.rows[4].lane, 0, "feat merges back to lane 0");
        let feat = &g.rows[4];
        assert!(
            feat.cells
                .iter()
                .any(|c| matches!(c, Cell::ElbowBottomLeft | Cell::ElbowBottomRight)),
            "feat row should show experiment converging via an elbow, got {:?}",
            feat.cells
        );
        assert_eq!(g.rows[5].lane, 0);
        assert_eq!(g.rows[6].lane, 0);
        assert_eq!(g.rows[7].lane, 0);

        assert!(g.width_cols >= 2, "fork widened the gutter");
    }

    /// The structural invariants the `graphstress` differential harness asserts
    /// on every random repo, distilled to a unit regression: for any laid-out
    /// DAG, (a) rows map 1:1 to nodes in order, (b) each node's own cell is a
    /// pipe/elided marker (never Empty/elbow), (c) node columns stay within
    /// `width_cols`, and (d) no two nodes collide at a (row, node-col).
    fn assert_layout_consistent(nodes: &[CommitNode], g: &GraphLayout) {
        assert_eq!(g.rows.len(), nodes.len(), "one row per node");
        let mut seen = std::collections::HashSet::new();
        for (i, row) in g.rows.iter().enumerate() {
            assert_eq!(row.node_index, i, "row {i} indexes node {i}");
            let col = row.node_col();
            assert!(col < g.width_cols, "row {i} node_col {col} within width");
            assert!(seen.insert((i, col)), "no (row,col) collision at row {i}");
            assert!(
                matches!(row.cells.get(col), Some(Cell::Vertical) | Some(Cell::Elided)),
                "row {i} node cell at col {col} must be a pipe/elided, got {:?}",
                row.cells.get(col),
            );
        }
        // Direct edges may only reference nodes present in the list.
        let present: std::collections::HashSet<&str> =
            nodes.iter().map(|n| n.commit_id.as_str()).collect();
        for node in nodes {
            for p in &node.parents {
                if p.edge_type == EdgeType::Direct {
                    assert!(
                        present.contains(p.commit_id.as_str()),
                        "Direct edge {}->{} references an absent node",
                        node.commit_id,
                        p.commit_id,
                    );
                }
            }
        }
    }

    #[test]
    fn merge_with_elided_parent_is_consistent() {
        // A merge whose second parent is filtered out (Indirect edge to an
        // absent commit) plus a converging side branch — exercises forks,
        // merges, elision, and a dangling-but-Indirect edge all at once.
        let nodes = vec![
            n("merge", &[("a", EdgeType::Direct), ("gone", EdgeType::Indirect)]),
            n("a", &[("base", EdgeType::Direct)]),
            n("side", &[("base", EdgeType::Direct)]),
            n("base", &[("root", EdgeType::Indirect)]),
            n("root", &[]),
        ];
        let g = layout(&nodes);
        assert_layout_consistent(&nodes, &g);
        // The merge node forks a lane for its second (Indirect) parent.
        assert!(
            g.rows[0]
                .cells
                .iter()
                .any(|c| matches!(c, Cell::ElbowTopLeft | Cell::ElbowTopRight)),
            "merge row should fork a lane for the second parent, got {:?}",
            g.rows[0].cells,
        );
        // base's Indirect first-parent edge marks its lane elided.
        assert_eq!(g.rows[3].cells[g.rows[3].node_col()], Cell::Elided);
    }

    #[test]
    fn linear_and_fork_shapes_stay_consistent() {
        // Re-validate the canonical shapes through the same invariant checker.
        let linear = vec![
            n("c", &[("b", EdgeType::Direct)]),
            n("b", &[("a", EdgeType::Direct)]),
            n("a", &[]),
        ];
        assert_layout_consistent(&linear, &layout(&linear));

        let fork = vec![
            n("p", &[("p1", EdgeType::Direct), ("p2", EdgeType::Direct)]),
            n("p1", &[]),
            n("p2", &[]),
        ];
        assert_layout_consistent(&fork, &layout(&fork));
    }
}
