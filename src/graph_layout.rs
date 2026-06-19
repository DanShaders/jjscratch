//! Graph lane layout: turn a topologically-ordered DAG ([`CommitNode`] list with
//! parent edges) into per-row lane assignments and connector segments, so the
//! graph renderer can draw continuous pipes WITHOUT relying on jj's ASCII output.
//!
//! STATUS: STUB / contract only. The real implementation computes, for each
//! flattened row, which lane the node sits in and which connector cells
//! (vertical `│`, horizontal `─`, elbows `╭╮╰╯`, elision `~`) occupy the gutter,
//! matching lightjj's GraphSvg geometry (docs/spec/ui-spec.md §4.1: CELL_W=10,
//! lane=col/2, node center x = col*10+5, ROW_H=18). This is PURE logic — unit-test
//! it against the fixture DAG independently of rendering.
//!
//! Reference algorithm: a standard incremental lane assignment over the
//! topological order — maintain a list of "active lanes" (each tracking the
//! commit id it is currently routing to); when a node is emitted, place it in the
//! lane that was waiting for it (or a fresh lane), then replace that lane's target
//! with the node's first parent and append new lanes for additional parents,
//! emitting horizontal/elbow cells where lanes merge or fork.

use crate::model::CommitNode;

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

/// Full graph layout for a node list.
#[derive(Clone, Debug, Default)]
pub struct GraphLayout {
    pub rows: Vec<LayoutRow>,
    /// Max columns used (for sizing the gutter SVG width).
    pub width_cols: usize,
}

/// Compute the lane layout. STUB: places every node in lane 0 with a single
/// vertical connector — correct only for a linear history. Replace with the full
/// incremental lane-assignment algorithm described above.
pub fn layout(nodes: &[CommitNode]) -> GraphLayout {
    let rows = nodes
        .iter()
        .enumerate()
        .map(|(i, _)| LayoutRow {
            node_index: i,
            lane: 0,
            cells: vec![Cell::Vertical],
        })
        .collect();
    GraphLayout { rows, width_cols: 1 }
}
