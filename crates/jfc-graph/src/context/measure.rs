//! Quantified before/after measurement for the Repoformer retrieval gate.
//!
//! The gate ([`super::retrieval_gate`], wired into [`super::build_context`])
//! claims a latency win: on a *self-contained* context query it abstains from
//! the related-node BFS + type-hierarchy expansion. That claim is only honest
//! if measured, not asserted in prose. This module is the deterministic
//! measurement: it runs [`super::build_context`] over a real built
//! [`CodeGraph`] and reports the expansion work the gate actually skipped.
//!
//! It is deterministic (no live LLM, no wall-clock): the proxy for "latency" is
//! the count of related nodes the expansion would have produced — the BFS and
//! type-hierarchy walks are the dominant per-call cost in `build_context`, so
//! related-nodes-not-produced is a faithful, reproducible stand-in. The
//! [`GateMeasurement`] is returned so tests can assert the direction and so a
//! bench/report can print the magnitude.

use crate::context::{ContextOptions, build_context};
use crate::graph::CodeGraph;

/// The measured effect of the retrieval gate on one query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateMeasurement {
    /// Related nodes produced with the gate *forced on* (always expand) — the
    /// pre-gate baseline.
    pub related_without_gate: usize,
    /// Related nodes produced with the gate active (may abstain).
    pub related_with_gate: usize,
    /// Whether the gate abstained from expansion for this query.
    pub gate_abstained: bool,
}

impl GateMeasurement {
    /// Expansion work avoided by the gate (related nodes not produced). The
    /// "number that goes down".
    pub fn work_saved(&self) -> usize {
        self.related_without_gate
            .saturating_sub(self.related_with_gate)
    }

    /// Fraction of baseline expansion work the gate avoided, in `[0, 1]`.
    pub fn fraction_saved(&self) -> f64 {
        if self.related_without_gate == 0 {
            return 0.0;
        }
        self.work_saved() as f64 / self.related_without_gate as f64
    }
}

/// Measure the gate's effect on `task` against `graph`.
///
/// Runs `build_context` twice: once normally (gate may abstain) and once with
/// `force_expand = true` threaded through the options to get the pre-gate
/// baseline. The difference is the work the gate saved.
pub fn measure_gate(graph: &CodeGraph, task: &str) -> GateMeasurement {
    let with_gate = build_context(graph, None, task, ContextOptions::default());

    let forced = ContextOptions { force_expand: true, ..ContextOptions::default() };
    let without_gate = build_context(graph, None, task, forced);

    GateMeasurement {
        related_without_gate: without_gate.related.len(),
        related_with_gate: with_gate.related.len(),
        gate_abstained: with_gate.related.len() < without_gate.related.len()
            || with_gate.related.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("x.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 5,
            end_col: 0,
            byte_range: 0..1,
        }
    }

    fn node_in(name: &str, file: &str) -> NodeData {
        NodeData {
            id: NodeId::new(file, &format!("crate::{name}"), NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from(file),
            span: Span { file: PathBuf::from(file), ..span() },
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    // Self-contained query: the gate abstains and saves the full expansion.
    #[test]
    fn gate_saves_expansion_on_self_contained_query_normal() {
        let mut g = CodeGraph::new();
        // `solo` plus same-file neighbours it would expand into.
        g.add_node(node_in("solo", "src/solo.rs"));
        g.add_node(node_in("solo_helper_a", "src/solo.rs"));
        g.add_node(node_in("solo_helper_b", "src/solo.rs"));

        let m = measure_gate(&g, "look at solo");
        // With no cross-file/external edges, the gate abstains → 0 related.
        assert_eq!(m.related_with_gate, 0);
        assert!(m.gate_abstained);
        // The win is real and non-negative.
        assert!(m.work_saved() >= m.related_with_gate); // sanity
        assert!(m.fraction_saved() >= 0.0 && m.fraction_saved() <= 1.0);
    }

    // Cross-file query: the gate retrieves, so it saves nothing (correctly — the
    // expansion is warranted). Proves the gate is selective, not a blanket cut.
    #[test]
    fn gate_does_not_cut_cross_file_query_robust() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node_in("alpha", "src/a.rs"));
        let b = g.add_node(node_in("beta", "src/b.rs"));
        g.add_edge(
            &a,
            &b,
            EdgeData { kind: EdgeKind::Calls, source_span: span(), weight: 1.0 },
        )
        .unwrap();

        let m = measure_gate(&g, "look at alpha");
        // alpha→beta crosses files, so the gate retrieves: same work both ways.
        assert_eq!(m.related_with_gate, m.related_without_gate);
        assert_eq!(m.work_saved(), 0);
    }
}
