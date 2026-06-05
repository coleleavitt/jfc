//! Program slicing — forward and backward.
//!
//! Roadmap idiom: *"statements and functions that can affect variable / field
//! X"* (backward slice) and *"what this assignment can influence"* (forward
//! slice). Both are BFS over a dataflow graph; what differs is the edge
//! direction (use→def vs def→use).
//!
//! # Forward-infrastructure caveat
//!
//! Real slicing requires a **dataflow IR** — def-use chains within and across
//! function boundaries. We don't have that yet: the call graph in
//! [`crate::graph::CodeGraph`] is structural (callers/callees) but does not
//! track which value flows where. This module implements the slicing
//! algorithms over an **abstract dataflow oracle** ([`DataflowOracle`]) so
//! that:
//!
//! 1. The BFS walk is fully implemented and tested today against a mock
//!    oracle, and
//! 2. The taint v2 / dataflow pass can drop in a real oracle implementation
//!    without changes to the slicing algorithms.
//!
//! Until that lands, [`EmptyOracle`] returns no dependencies — slices over it
//! contain only the seed.

use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::ir::IrFunction;
use crate::nodes::NodeId;
use crate::points_to::{PointsToTable, analyze_interprocedural};

/// Pluggable dataflow source: provides def-use and use-def relationships
/// over [`NodeId`]s.
///
/// Implementors will eventually be:
/// - A coarse interprocedural pass derived from `Calls` edges + parameter /
///   return-value tracking (see `taint_v2`),
/// - A fine-grained intraprocedural pass once per-function CFGs land.
///
/// The trait stays narrow on purpose: anything BFS-able fits, including
/// hand-written test fixtures.
pub trait DataflowOracle {
    /// Nodes whose *defined values* the given `node` **uses**.
    ///
    /// Reading: "the values flowing **into** `node` come from these nodes."
    /// Used by [`backward_slice`] to walk from a sink back toward sources.
    fn def_uses(&self, node: &NodeId) -> Vec<NodeId>;

    /// Nodes that **use** the *value defined* by the given `node`.
    ///
    /// Reading: "the values flowing **out of** `node` are consumed here."
    /// Used by [`forward_slice`] to walk from a source toward sinks.
    fn use_defs(&self, node: &NodeId) -> Vec<NodeId>;
}

/// Default oracle returning no dependencies. Useful as a placeholder until
/// the dataflow pass lands and as a `no-op` baseline in unit tests.
///
/// Slices over `EmptyOracle` contain only the seed (or are empty if the seed
/// is not in the graph).
pub struct EmptyOracle;

impl DataflowOracle for EmptyOracle {
    fn def_uses(&self, _: &NodeId) -> Vec<NodeId> {
        Vec::new()
    }

    fn use_defs(&self, _: &NodeId) -> Vec<NodeId> {
        Vec::new()
    }
}

/// Points-to-backed dataflow oracle. Uses interprocedural alias analysis
/// results to derive coarse def-use / use-def chains across functions.
///
/// The edges are derived from the call graph:
/// - `def_uses(callee)` → callers that pass aliased arguments into callee.
/// - `use_defs(caller)` → callees whose parameters alias caller's values.
pub struct PointsToOracle {
    /// Per-function points-to tables (interprocedural results).
    tables: HashMap<NodeId, PointsToTable>,
    /// caller → callees (forward call edges).
    callees_of: HashMap<NodeId, Vec<NodeId>>,
    /// callee → callers (backward call edges).
    callers_of: HashMap<NodeId, Vec<NodeId>>,
}

impl PointsToOracle {
    /// Build from a code graph and IR map by running interprocedural
    /// points-to analysis and caching the call graph topology.
    pub fn build(graph: &CodeGraph, ir_map: &HashMap<NodeId, IrFunction>) -> Self {
        let tables = analyze_interprocedural(graph, ir_map);
        let (callees_of, callers_of) = extract_call_edges(graph, &tables);
        Self {
            tables,
            callees_of,
            callers_of,
        }
    }

    /// Check whether two functions share aliased parameter/return flow.
    fn has_alias_flow(&self, from: &NodeId, to: &NodeId) -> bool {
        let (Some(from_pts), Some(to_pts)) = (self.tables.get(from), self.tables.get(to)) else {
            return false;
        };
        // Check if any variable in `from` shares locations with any param in `to`.
        for from_set in from_pts.vars.values() {
            for to_set in to_pts.vars.values() {
                if from_set.intersection(to_set).next().is_some() {
                    return true;
                }
            }
        }
        false
    }
}

impl DataflowOracle for PointsToOracle {
    fn def_uses(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(callers) = self.callers_of.get(node) else {
            return Vec::new();
        };
        callers
            .iter()
            .filter(|caller| self.has_alias_flow(caller, node))
            .cloned()
            .collect()
    }

    fn use_defs(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(callees) = self.callees_of.get(node) else {
            return Vec::new();
        };
        callees
            .iter()
            .filter(|callee| self.has_alias_flow(node, callee))
            .cloned()
            .collect()
    }
}

/// Extract forward/backward call edges from the graph for nodes in the
/// analysis set.
fn extract_call_edges(
    graph: &CodeGraph,
    tables: &HashMap<NodeId, PointsToTable>,
) -> (HashMap<NodeId, Vec<NodeId>>, HashMap<NodeId, Vec<NodeId>>) {
    let mut callees_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    let mut callers_of: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    for node_id in tables.keys() {
        for (target, ed) in graph.get_edges_from(node_id) {
            if !matches!(ed.kind, EdgeKind::Calls) {
                continue;
            }
            if !tables.contains_key(target) {
                continue;
            }
            callees_of
                .entry(node_id.clone())
                .or_default()
                .push(target.clone());
            callers_of
                .entry(target.clone())
                .or_default()
                .push(node_id.clone());
        }
    }
    (callees_of, callers_of)
}

/// Backward slice: every node that can affect the value at `seed`.
///
/// BFS over `oracle.def_uses` starting at `seed`, capped at `max_depth`
/// hops to bound work in cyclic / dense graphs. Result is a sorted set
/// (the deterministic ordering matters for snapshot tests and graph
/// fingerprints — same reason [`NodeId`] is `Ord`).
///
/// If `seed` is not present in `graph` the result is empty (we never hand
/// out NodeIds that don't exist).
pub fn backward_slice(
    graph: &CodeGraph,
    oracle: &dyn DataflowOracle,
    seed: &NodeId,
    max_depth: usize,
) -> BTreeSet<NodeId> {
    bfs_slice(graph, seed, max_depth, |n| oracle.def_uses(n))
}

/// Forward slice: every node that can be influenced by the value at `seed`.
///
/// BFS over `oracle.use_defs` starting at `seed`. See [`backward_slice`]
/// for shared semantics (depth cap, set ordering, missing-seed handling).
pub fn forward_slice(
    graph: &CodeGraph,
    oracle: &dyn DataflowOracle,
    seed: &NodeId,
    max_depth: usize,
) -> BTreeSet<NodeId> {
    bfs_slice(graph, seed, max_depth, |n| oracle.use_defs(n))
}

/// Shared BFS engine for forward / backward slicing. Caller supplies the
/// neighbor function (`def_uses` for backward, `use_defs` for forward).
///
/// Includes `seed` in the result by convention — slices are inclusive of
/// the seed, matching the textbook definition (Weiser 1981).
fn bfs_slice<F>(
    graph: &CodeGraph,
    seed: &NodeId,
    max_depth: usize,
    neighbors: F,
) -> BTreeSet<NodeId>
where
    F: Fn(&NodeId) -> Vec<NodeId>,
{
    let mut out = BTreeSet::new();
    if !graph.contains_node(seed) {
        return out;
    }
    out.insert(seed.clone());

    if max_depth == 0 {
        return out;
    }

    let mut frontier: VecDeque<(NodeId, usize)> = VecDeque::new();
    frontier.push_back((seed.clone(), 0));

    while let Some((current, depth)) = frontier.pop_front() {
        if depth >= max_depth {
            continue;
        }
        for next in neighbors(&current) {
            // Drop neighbors the graph doesn't know about — the oracle may
            // legitimately reference nodes that have since been pruned.
            if !graph.contains_node(&next) {
                continue;
            }
            // BTreeSet::insert returns true iff the value is new — gates
            // the BFS visit and prevents reprocessing on cycles.
            if out.insert(next.clone()) {
                frontier.push_back((next, depth + 1));
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Hard-coded test oracle: a directed dependency graph keyed by NodeId.
    /// `defs[node]` = nodes whose values `node` uses (def_uses).
    /// `uses[node]` = nodes that use the value `node` defines (use_defs).
    #[derive(Default)]
    struct MockOracle {
        defs: HashMap<NodeId, Vec<NodeId>>,
        uses: HashMap<NodeId, Vec<NodeId>>,
    }

    impl DataflowOracle for MockOracle {
        fn def_uses(&self, node: &NodeId) -> Vec<NodeId> {
            self.defs.get(node).cloned().unwrap_or_default()
        }
        fn use_defs(&self, node: &NodeId) -> Vec<NodeId> {
            self.uses.get(node).cloned().unwrap_or_default()
        }
    }

    fn mk_node(name: &str) -> NodeId {
        NodeId::new("src/test.rs", name, NodeKind::Function)
    }

    fn mk_node_data(name: &str) -> NodeData {
        NodeData {
            id: mk_node(name),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from("src/test.rs"),
            span: Span {
                file: PathBuf::from("src/test.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..0,
            },
            visibility: Visibility::Private,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    /// Build a graph with the given function names, returning the graph and
    /// a list of their NodeIds in declaration order.
    fn graph_with(names: &[&str]) -> (CodeGraph, Vec<NodeId>) {
        let mut g = CodeGraph::new();
        let ids = names.iter().map(|n| g.add_node(mk_node_data(n))).collect();
        (g, ids)
    }

    #[test]
    fn backward_slice_walks_def_uses_chain_normal() {
        // sink <- mid <- source : backward from `sink` should reach `source`.
        let (graph, ids) = graph_with(&["source", "mid", "sink"]);
        let (source, mid, sink) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

        let mut oracle = MockOracle::default();
        oracle.defs.insert(sink.clone(), vec![mid.clone()]);
        oracle.defs.insert(mid.clone(), vec![source.clone()]);

        let slice = backward_slice(&graph, &oracle, &sink, 10);

        assert!(slice.contains(&sink));
        assert!(slice.contains(&mid));
        assert!(slice.contains(&source));
        assert_eq!(slice.len(), 3);
    }

    #[test]
    fn forward_slice_walks_use_defs_chain_normal() {
        // source -> mid -> sink : forward from `source` should reach `sink`.
        let (graph, ids) = graph_with(&["source", "mid", "sink"]);
        let (source, mid, sink) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

        let mut oracle = MockOracle::default();
        oracle.uses.insert(source.clone(), vec![mid.clone()]);
        oracle.uses.insert(mid.clone(), vec![sink.clone()]);

        let slice = forward_slice(&graph, &oracle, &source, 10);

        assert_eq!(slice.len(), 3);
        assert!(slice.contains(&source));
        assert!(slice.contains(&mid));
        assert!(slice.contains(&sink));
    }

    #[test]
    fn slice_respects_max_depth_boundary() {
        // Linear chain a -> b -> c -> d. Forward from `a` with depth=2 should
        // include {a, b, c} but NOT `d`.
        let (graph, ids) = graph_with(&["a", "b", "c", "d"]);
        let (a, b, c, d) = (
            ids[0].clone(),
            ids[1].clone(),
            ids[2].clone(),
            ids[3].clone(),
        );

        let mut oracle = MockOracle::default();
        oracle.uses.insert(a.clone(), vec![b.clone()]);
        oracle.uses.insert(b.clone(), vec![c.clone()]);
        oracle.uses.insert(c.clone(), vec![d.clone()]);

        let slice = forward_slice(&graph, &oracle, &a, 2);

        assert!(slice.contains(&a));
        assert!(slice.contains(&b));
        assert!(slice.contains(&c));
        assert!(!slice.contains(&d), "depth cap should exclude d");
    }

    #[test]
    fn slice_handles_cycles_boundary() {
        // a -> b -> a. BFS must terminate (visited set blocks revisit).
        let (graph, ids) = graph_with(&["a", "b"]);
        let (a, b) = (ids[0].clone(), ids[1].clone());

        let mut oracle = MockOracle::default();
        oracle.uses.insert(a.clone(), vec![b.clone()]);
        oracle.uses.insert(b.clone(), vec![a.clone()]);

        let slice = forward_slice(&graph, &oracle, &a, 100);

        assert_eq!(slice.len(), 2);
        assert!(slice.contains(&a));
        assert!(slice.contains(&b));
    }

    #[test]
    fn slice_with_empty_oracle_returns_seed_only_boundary() {
        let (graph, ids) = graph_with(&["seed"]);
        let seed = ids[0].clone();

        let slice = forward_slice(&graph, &EmptyOracle, &seed, 10);

        assert_eq!(slice.len(), 1);
        assert!(slice.contains(&seed));
    }

    #[test]
    fn slice_seed_not_in_graph_returns_empty_boundary() {
        let (graph, _) = graph_with(&["unrelated"]);
        // Build a NodeId that the graph doesn't contain.
        let phantom = mk_node("does_not_exist");

        let slice = forward_slice(&graph, &EmptyOracle, &phantom, 10);

        assert!(slice.is_empty());
    }

    #[test]
    fn slice_filters_oracle_neighbors_not_in_graph_boundary() {
        // Oracle reports a dependency the graph has since pruned — the slice
        // should not include the missing node.
        let (graph, ids) = graph_with(&["seed"]);
        let seed = ids[0].clone();
        let phantom = mk_node("phantom");

        let mut oracle = MockOracle::default();
        oracle.uses.insert(seed.clone(), vec![phantom.clone()]);

        let slice = forward_slice(&graph, &oracle, &seed, 10);

        assert_eq!(slice.len(), 1);
        assert!(slice.contains(&seed));
        assert!(!slice.contains(&phantom));
    }

    // ─── PointsToOracle tests ────────────────────────────────────────────

    use crate::edges::{EdgeData, EdgeKind};
    use crate::ir::{IrFunction, IrOp, Operand, Var};

    fn calls_edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: Span {
                file: PathBuf::from("test.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..0,
            },
            weight: 1.0,
        }
    }

    #[test]
    fn points_to_oracle_forward_slice_connected_chain() {
        // source → main → sink (all connected via Calls edges).
        // source returns param "taint", main passes it to sink.
        let mut source_ir = IrFunction::new("source");
        source_ir.params.push(Var::new("taint"));
        source_ir.push(IrOp::Return {
            value: Some(Operand::Var(Var::new("taint"))),
        });

        let mut main_ir = IrFunction::new("main");
        main_ir.push(IrOp::Call {
            dst: Some(Var::new("tmp")),
            callee: "source".into(),
            args: vec![],
        });
        main_ir.push(IrOp::Call {
            dst: None,
            callee: "sink".into(),
            args: vec![Operand::Var(Var::new("tmp"))],
        });

        let mut sink_ir = IrFunction::new("sink");
        sink_ir.params.push(Var::new("x"));

        let (mut graph, ids) = graph_with(&["source", "main", "sink"]);
        let (source_id, main_id, sink_id) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

        graph.add_edge(&main_id, &source_id, calls_edge()).unwrap();
        graph.add_edge(&main_id, &sink_id, calls_edge()).unwrap();

        let mut ir_map: HashMap<NodeId, IrFunction> = HashMap::new();
        ir_map.insert(source_id, source_ir);
        ir_map.insert(main_id.clone(), main_ir);
        ir_map.insert(sink_id.clone(), sink_ir);

        let oracle = super::PointsToOracle::build(&graph, &ir_map);

        // Forward slice from main should reach sink (use_defs follows flow).
        let fwd = forward_slice(&graph, &oracle, &main_id, 10);
        assert!(
            fwd.contains(&sink_id),
            "forward slice from main should include sink, got {:?}",
            fwd,
        );
    }

    #[test]
    fn points_to_oracle_backward_slice_connected_chain() {
        // main calls source, passes result to sink.
        // Backward from sink should reach main (the caller that passes data).
        let mut source_ir = IrFunction::new("source");
        source_ir.params.push(Var::new("taint"));
        source_ir.push(IrOp::Return {
            value: Some(Operand::Var(Var::new("taint"))),
        });

        let mut main_ir = IrFunction::new("main");
        main_ir.push(IrOp::Call {
            dst: Some(Var::new("tmp")),
            callee: "source".into(),
            args: vec![],
        });
        main_ir.push(IrOp::Call {
            dst: None,
            callee: "sink".into(),
            args: vec![Operand::Var(Var::new("tmp"))],
        });

        let mut sink_ir = IrFunction::new("sink");
        sink_ir.params.push(Var::new("x"));

        let (mut graph, ids) = graph_with(&["source", "main", "sink"]);
        let (source_id, main_id, sink_id) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

        graph.add_edge(&main_id, &source_id, calls_edge()).unwrap();
        graph.add_edge(&main_id, &sink_id, calls_edge()).unwrap();

        let mut ir_map: HashMap<NodeId, IrFunction> = HashMap::new();
        ir_map.insert(source_id, source_ir);
        ir_map.insert(main_id.clone(), main_ir);
        ir_map.insert(sink_id.clone(), sink_ir);

        let oracle = super::PointsToOracle::build(&graph, &ir_map);

        // Backward slice from sink should reach main (def_uses).
        let bwd = backward_slice(&graph, &oracle, &sink_id, 10);
        assert!(
            bwd.contains(&main_id),
            "backward slice from sink should include main, got {:?}",
            bwd,
        );
    }
}
