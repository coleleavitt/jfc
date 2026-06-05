//! Interprocedural taint analysis (v2). Tracks tainted values from
//! configured **sources** through function-call boundaries to **sinks**,
//! optionally noting whether the path passed through a sanitizer.
//!
//! Roadmap idiom: *"build a coarse interprocedural dataflow graph
//! (parameters, return values, well-known sources/sinks), then run a proper
//! taint analysis on top of it."* This module is the analysis half — the
//! "coarse interprocedural dataflow graph" is provided externally via a
//! [`crate::slicing::DataflowOracle`].
//!
//! # Forward-infrastructure caveat
//!
//! The "v2" name distinguishes this from any earlier ad-hoc taint logic
//! that operated directly on the call graph. Real precision requires:
//!
//! 1. Parameter-level dataflow (which argument flows where),
//! 2. Return-value tracking (does the callee leak the tainted value back?),
//! 3. Field-sensitive aliasing (does writing `s.x` taint `s`?).
//!
//! None of those exist in `jfc-graph` yet. The algorithm below is correct
//! given a precise oracle and degrades to "any reachable sink is tainted"
//! given only call-graph reachability. As the dataflow IR becomes more
//! precise, the same `analyze` function gives better results without code
//! changes — the precision lives in the oracle.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::graph::CodeGraph;
use crate::nodes::NodeId;
use crate::slicing::DataflowOracle;

/// One source-to-sink taint flow.
///
/// `path` walks from the source to the sink along the dataflow oracle's
/// `def_uses` edges (i.e. forward dataflow). `passed_through_sanitizer`
/// records the **first** sanitizer encountered on the path; downstream
/// reporters treat sanitized flows as informational rather than
/// vulnerabilities.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaintFlow {
    pub source: NodeId,
    pub sink: NodeId,
    /// Path from source to sink (inclusive endpoints). Constructed by
    /// reconstructing the BFS predecessor chain so callers can render
    /// "tainted value flows from `source` through A, B, C to `sink`".
    pub path: Vec<NodeId>,
    /// First sanitizer encountered on `path`, if any. `None` means the
    /// path reaches the sink without sanitization (i.e. a real flow).
    pub passed_through_sanitizer: Option<NodeId>,
}

/// Configuration: which nodes are sources, sinks, sanitizers.
///
/// Borrowed slices instead of owned `Vec`s — callers typically build these
/// from rule registries or LSP queries and reuse them across many `analyze`
/// invocations.
pub struct TaintConfig<'a> {
    pub sources: &'a [NodeId],
    pub sinks: &'a [NodeId],
    pub sanitizers: &'a [NodeId],
}

/// Find all source-to-sink taint flows in `graph` according to `oracle`.
///
/// For each configured source, runs a BFS along `oracle.use_defs` (forward
/// dataflow). When the BFS reaches a sink node, emits one [`TaintFlow`]
/// recording the path and whether a sanitizer appeared on it.
///
/// Multiple flows from the same `(source, sink)` pair are deduplicated —
/// the first path discovered (shortest, by BFS layer order) wins. Callers
/// who need *all* paths should run `forward_slice` directly and inspect
/// the slice instead.
///
/// ## Phase 7 — frontier-aware BFS
///
/// The visited set uses a [`crate::frontier::Frontier`]-style hybrid
/// (`HashSet<NodeId>` for sparse traversals, no auto-promotion since
/// we work in `NodeId` space rather than dense `u32` indices). The
/// underlying graph is the dataflow oracle's view, not the call
/// graph, so push/pull direction-optimisation doesn't apply
/// directly — we'd have to ask the oracle to enumerate predecessors.
/// Future work: extend `DataflowOracle` with a `predecessors` method
/// to enable pull-mode BFS for dense oracles.
pub fn analyze(
    graph: &CodeGraph,
    oracle: &dyn DataflowOracle,
    config: &TaintConfig,
) -> Vec<TaintFlow> {
    // Lookup tables: O(1) "is this a sink/sanitizer?" during BFS.
    let sinks: HashSet<&NodeId> = config.sinks.iter().collect();
    let sanitizers: HashSet<&NodeId> = config.sanitizers.iter().collect();

    let mut flows: Vec<TaintFlow> = Vec::new();
    // Dedup: at most one flow per (source, sink) pair. BFS guarantees the
    // first one is the shortest.
    let mut seen_pairs: HashSet<(NodeId, NodeId)> = HashSet::new();

    for source in config.sources {
        if !graph.contains_node(source) {
            continue;
        }

        // BFS state. `predecessor` lets us reconstruct paths once we hit a
        // sink. `sanitizer_on_path` carries the first sanitizer seen on the
        // shortest path to each node — sufficient because BFS visits each
        // node via its shortest-path predecessor.
        let mut predecessor: HashMap<NodeId, NodeId> = HashMap::new();
        let mut sanitizer_on_path: HashMap<NodeId, NodeId> = HashMap::new();
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut frontier: VecDeque<NodeId> = VecDeque::new();

        visited.insert(source.clone());
        frontier.push_back(source.clone());
        // The source itself may or may not be a sanitizer (almost never in
        // practice, but the bookkeeping is symmetric).
        if sanitizers.contains(source) {
            sanitizer_on_path.insert(source.clone(), source.clone());
        }

        while let Some(current) = frontier.pop_front() {
            // Sink check happens *after* dequeue and *before* expansion so
            // the source itself can be a sink (degenerate case).
            if sinks.contains(&current) && &current != source {
                let pair = (source.clone(), current.clone());
                if seen_pairs.insert(pair) {
                    flows.push(reconstruct_flow(
                        source,
                        &current,
                        &predecessor,
                        sanitizer_on_path.get(&current).cloned(),
                    ));
                }
                // Continue BFS — other sinks may be reachable past this one.
            }

            for next in oracle.use_defs(&current) {
                if !graph.contains_node(&next) {
                    continue;
                }
                if !visited.insert(next.clone()) {
                    continue;
                }
                predecessor.insert(next.clone(), current.clone());
                // Carry forward an existing sanitizer marker, or set one if
                // the new node is itself a sanitizer.
                if let Some(s) = sanitizer_on_path.get(&current).cloned() {
                    sanitizer_on_path.insert(next.clone(), s);
                } else if sanitizers.contains(&next) {
                    sanitizer_on_path.insert(next.clone(), next.clone());
                }
                frontier.push_back(next);
            }
        }
    }

    flows
}

/// Walk the BFS predecessor map back from `sink` to `source` to assemble
/// the path. Returns `[source, …, sink]` in source-to-sink order.
fn reconstruct_flow(
    source: &NodeId,
    sink: &NodeId,
    predecessor: &HashMap<NodeId, NodeId>,
    sanitizer: Option<NodeId>,
) -> TaintFlow {
    let mut path = vec![sink.clone()];
    let mut cursor = sink.clone();
    while let Some(prev) = predecessor.get(&cursor) {
        path.push(prev.clone());
        if prev == source {
            break;
        }
        cursor = prev.clone();
    }
    path.reverse();
    TaintFlow {
        source: source.clone(),
        sink: sink.clone(),
        path,
        passed_through_sanitizer: sanitizer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::path::PathBuf;

    /// MockOracle: hard-coded use-def edges keyed by NodeId.
    #[derive(Default)]
    struct MockOracle {
        uses: HashMap<NodeId, Vec<NodeId>>,
    }

    impl DataflowOracle for MockOracle {
        fn def_uses(&self, _: &NodeId) -> Vec<NodeId> {
            Vec::new()
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

    fn graph_with(names: &[&str]) -> (CodeGraph, Vec<NodeId>) {
        let mut g = CodeGraph::new();
        let ids = names.iter().map(|n| g.add_node(mk_node_data(n))).collect();
        (g, ids)
    }

    #[test]
    fn taint_source_to_sink_direct_normal() {
        // source -> sink with no sanitizer.
        let (graph, ids) = graph_with(&["source", "sink"]);
        let (source, sink) = (ids[0].clone(), ids[1].clone());

        let mut oracle = MockOracle::default();
        oracle.uses.insert(source.clone(), vec![sink.clone()]);

        let sources = [source.clone()];
        let sinks = [sink.clone()];
        let sanitizers: [NodeId; 0] = [];
        let cfg = TaintConfig {
            sources: &sources,
            sinks: &sinks,
            sanitizers: &sanitizers,
        };

        let flows = analyze(&graph, &oracle, &cfg);

        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.source, source);
        assert_eq!(f.sink, sink);
        assert_eq!(f.path, vec![source, sink]);
        assert_eq!(f.passed_through_sanitizer, None);
    }

    #[test]
    fn taint_source_through_sanitizer_to_sink_flagged_normal() {
        // source -> sanitizer -> sink: the flow is reported with the
        // sanitizer recorded.
        let (graph, ids) = graph_with(&["source", "san", "sink"]);
        let (source, san, sink) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

        let mut oracle = MockOracle::default();
        oracle.uses.insert(source.clone(), vec![san.clone()]);
        oracle.uses.insert(san.clone(), vec![sink.clone()]);

        let sources = [source.clone()];
        let sinks = [sink.clone()];
        let sanitizers = [san.clone()];
        let cfg = TaintConfig {
            sources: &sources,
            sinks: &sinks,
            sanitizers: &sanitizers,
        };

        let flows = analyze(&graph, &oracle, &cfg);

        assert_eq!(flows.len(), 1);
        let f = &flows[0];
        assert_eq!(f.source, source);
        assert_eq!(f.sink, sink);
        assert_eq!(f.passed_through_sanitizer, Some(san.clone()));
        assert_eq!(f.path, vec![source, san, sink]);
    }

    #[test]
    fn taint_source_unreachable_to_sink_empty_boundary() {
        // source and sink exist, but the oracle reports no path between them.
        let (graph, ids) = graph_with(&["source", "sink"]);
        let (source, sink) = (ids[0].clone(), ids[1].clone());

        let oracle = MockOracle::default();

        let sources = [source];
        let sinks = [sink];
        let sanitizers: [NodeId; 0] = [];
        let cfg = TaintConfig {
            sources: &sources,
            sinks: &sinks,
            sanitizers: &sanitizers,
        };

        let flows = analyze(&graph, &oracle, &cfg);

        assert!(flows.is_empty());
    }

    #[test]
    fn taint_multiple_sources_each_yield_flow_normal() {
        // Two sources, both reach the same sink. Expect two flows
        // (one per source, dedup is per (source, sink) pair).
        let (graph, ids) = graph_with(&["s1", "s2", "sink"]);
        let (s1, s2, sink) = (ids[0].clone(), ids[1].clone(), ids[2].clone());

        let mut oracle = MockOracle::default();
        oracle.uses.insert(s1.clone(), vec![sink.clone()]);
        oracle.uses.insert(s2.clone(), vec![sink.clone()]);

        let sources = [s1.clone(), s2.clone()];
        let sinks = [sink];
        let sanitizers: [NodeId; 0] = [];
        let cfg = TaintConfig {
            sources: &sources,
            sinks: &sinks,
            sanitizers: &sanitizers,
        };

        let flows = analyze(&graph, &oracle, &cfg);

        assert_eq!(flows.len(), 2);
        let sources_seen: HashSet<&NodeId> = flows.iter().map(|f| &f.source).collect();
        assert!(sources_seen.contains(&s1));
        assert!(sources_seen.contains(&s2));
    }

    #[test]
    fn taint_dedup_per_source_sink_pair_boundary() {
        // Two paths source -> a -> sink and source -> b -> sink. Only the
        // shortest (BFS-first) is reported; both have length 2 here, so
        // exactly one flow emerges.
        let (graph, ids) = graph_with(&["source", "a", "b", "sink"]);
        let (source, a, b, sink) = (
            ids[0].clone(),
            ids[1].clone(),
            ids[2].clone(),
            ids[3].clone(),
        );

        let mut oracle = MockOracle::default();
        oracle
            .uses
            .insert(source.clone(), vec![a.clone(), b.clone()]);
        oracle.uses.insert(a, vec![sink.clone()]);
        oracle.uses.insert(b, vec![sink.clone()]);

        let sources = [source];
        let sinks = [sink];
        let sanitizers: [NodeId; 0] = [];
        let cfg = TaintConfig {
            sources: &sources,
            sinks: &sinks,
            sanitizers: &sanitizers,
        };

        let flows = analyze(&graph, &oracle, &cfg);

        assert_eq!(
            flows.len(),
            1,
            "dedup keeps only one flow per (source, sink)"
        );
    }
}
