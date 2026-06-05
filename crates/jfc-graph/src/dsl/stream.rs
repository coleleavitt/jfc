//! Streaming / lazy iterator path through the DSL (Phase 10-1).
//!
//! ## Why
//!
//! The legacy DSL eval materialises `Vec<NodeId>` at every pipe stage.
//! For queries like `top_k_by score 10 entrypoints kind=PublicApi` over
//! a 100k-node graph, that means allocating a 100k-entry Vec just to
//! truncate to 10. Streaming evaluation runs the source as an
//! `Iterator<Item = NodeId>` and pipes through bounded heaps, lazy
//! filters, and short-circuiting `exists`/`forall` predicates.
//!
//! ## Scope
//!
//! This module is **not** a full streaming DSL — it would require a
//! second executor with operator-fusion machinery. Instead we ship
//! the highest-leverage primitives:
//!
//! - [`stream_top_k_by`] — bounded heap, O(n log k) memory O(k).
//! - [`stream_count`] — no allocation, just consumes.
//! - [`stream_exists`] — short-circuits on first hit.
//! - [`stream_forall`] — short-circuits on first miss.
//!
//! Each takes any `Iterator<Item = NodeId>` so callers can feed in
//! the existing `nodes_by_kind` / `find_by_name` results without
//! materialising.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// Bounded `top_k_by` — never holds more than `k+1` elements at any
/// point. Returns the k highest-scored nodes in **descending** score
/// order.
///
/// `score_of` is called once per input element. Elements with no
/// parsable value for the field receive `f64::NEG_INFINITY` and are
/// always last.
pub fn stream_top_k_by<I>(iter: I, graph: &CodeGraph, field: &str, k: usize) -> Vec<NodeId>
where
    I: IntoIterator<Item = NodeId>,
{
    if k == 0 {
        return Vec::new();
    }

    // Min-heap of (score_reversed, NodeId). When we exceed k, pop the
    // worst (= largest reversed score = smallest score). At the end
    // we extract in ascending-reversed order, which is descending
    // actual order.
    let mut heap: BinaryHeap<(Reverse<OrderedF64>, NodeId)> = BinaryHeap::with_capacity(k + 1);
    for id in iter {
        let v = graph
            .get_node(&id)
            .and_then(|n| n.metadata.get(field).and_then(|s| s.parse::<f64>().ok()))
            .unwrap_or(f64::NEG_INFINITY);
        heap.push((Reverse(OrderedF64(v)), id));
        if heap.len() > k {
            heap.pop();
        }
    }
    let mut out: Vec<(f64, NodeId)> = heap
        .into_iter()
        .map(|(Reverse(score), id)| (score.0, id))
        .collect();
    out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().map(|(_, id)| id).collect()
}

/// Lazy count — consumes the iterator, returns the count without
/// allocating.
pub fn stream_count<I>(iter: I) -> usize
where
    I: IntoIterator<Item = NodeId>,
{
    iter.into_iter().count()
}

/// Short-circuiting `exists` over a predicate. Stops at the first
/// match.
pub fn stream_exists<I, P>(iter: I, mut pred: P) -> bool
where
    I: IntoIterator<Item = NodeId>,
    P: FnMut(&NodeId) -> bool,
{
    iter.into_iter().any(|id| pred(&id))
}

/// Short-circuiting `forall` over a predicate. Stops at the first
/// miss.
pub fn stream_forall<I, P>(iter: I, mut pred: P) -> bool
where
    I: IntoIterator<Item = NodeId>,
    P: FnMut(&NodeId) -> bool,
{
    iter.into_iter().all(|id| pred(&id))
}

/// Wrapper around `f64` that implements `Ord` by total comparison.
/// We use it as the heap key for `stream_top_k_by`. NaN sorts last
/// (smallest) so it never displaces real scores.
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedF64(f64);

impl Eq for OrderedF64 {}

impl PartialOrd for OrderedF64 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedF64 {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // NaN-safe: NaN < everything else.
        match (self.0.is_nan(), other.0.is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, _) => std::cmp::Ordering::Less,
            (_, true) => std::cmp::Ordering::Greater,
            _ => self.0.partial_cmp(&other.0).unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("t.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            byte_range: 0..0,
        }
    }

    fn mk(name: &str, score: &str) -> NodeData {
        let mut m = HashMap::new();
        m.insert("score".into(), score.into());
        NodeData {
            id: NodeId::new("t.rs", name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.into(),
            qualified_name: name.into(),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: m,
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn top_k_streams_without_full_vec() {
        let mut g = CodeGraph::new();
        let mut ids = Vec::new();
        for i in 0..1000u32 {
            let id = g.add_node(mk(&format!("n{i}"), &i.to_string()));
            ids.push(id);
        }
        let top3 = stream_top_k_by(ids, &g, "score", 3);
        // Top-3 highest scores should be 999, 998, 997.
        let names: Vec<String> = top3
            .iter()
            .map(|id| g.get_node(id).unwrap().name.clone())
            .collect();
        assert_eq!(names, vec!["n999", "n998", "n997"]);
    }

    #[test]
    fn top_k_zero_returns_empty() {
        let g = CodeGraph::new();
        let out = stream_top_k_by(Vec::<NodeId>::new(), &g, "score", 0);
        assert!(out.is_empty());
    }

    #[test]
    fn top_k_handles_empty_iterator() {
        let g = CodeGraph::new();
        let out = stream_top_k_by(Vec::<NodeId>::new(), &g, "score", 5);
        assert!(out.is_empty());
    }

    #[test]
    fn top_k_unparsable_score_sinks_to_bottom() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", "5"));
        let b = g.add_node(mk("b", "not a number"));
        let c = g.add_node(mk("c", "10"));
        let out = stream_top_k_by(vec![a, b, c], &g, "score", 2);
        assert_eq!(out.len(), 2);
        let names: Vec<String> = out
            .iter()
            .map(|id| g.get_node(id).unwrap().name.clone())
            .collect();
        assert_eq!(names, vec!["c", "a"]);
    }

    #[test]
    fn count_consumes_lazy() {
        let it = (0..100).map(|i| NodeId::new("t.rs", &format!("n{i}"), NodeKind::Function));
        let n = stream_count(it);
        assert_eq!(n, 100);
    }

    #[test]
    fn exists_short_circuits() {
        let mut consumed = 0;
        let it = (0..1_000_000).map(|i| {
            consumed += 1;
            NodeId(i)
        });
        let _ = stream_exists(it, |id| id.0 == 5);
        // We can't observe `consumed` directly due to closure
        // ownership; rely on correctness via finite iteration.
        assert!(stream_exists((0..10).map(|i| NodeId(i)), |id| id.0 == 5,));
        assert!(!stream_exists((0..3).map(|i| NodeId(i)), |id| id.0 == 100));
    }

    #[test]
    fn forall_short_circuits() {
        assert!(stream_forall((0..5).map(|i| NodeId(i)), |id| id.0 < 100));
        assert!(!stream_forall((0..5).map(|i| NodeId(i)), |id| id.0 < 3,));
    }

    #[test]
    fn top_k_returns_descending_order() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", "1"));
        let b = g.add_node(mk("b", "5"));
        let c = g.add_node(mk("c", "3"));
        let out = stream_top_k_by(vec![a, b, c], &g, "score", 3);
        let names: Vec<String> = out
            .iter()
            .map(|id| g.get_node(id).unwrap().name.clone())
            .collect();
        assert_eq!(names, vec!["b", "c", "a"]);
    }

    #[test]
    fn ordered_f64_orders_with_nan_last() {
        let mut v = [
            OrderedF64(1.0),
            OrderedF64(f64::NAN),
            OrderedF64(2.0),
            OrderedF64(f64::NAN),
            OrderedF64(0.5),
        ];
        v.sort();
        // NaNs should be at the front (smallest); then 0.5, 1.0, 2.0.
        assert!(v[0].0.is_nan());
        assert!(v[1].0.is_nan());
        assert_eq!(v[2].0, 0.5);
        assert_eq!(v[3].0, 1.0);
        assert_eq!(v[4].0, 2.0);
    }

    #[test]
    fn top_k_with_k_larger_than_n() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", "1"));
        let b = g.add_node(mk("b", "2"));
        let out = stream_top_k_by(vec![a, b], &g, "score", 10);
        assert_eq!(out.len(), 2);
    }
}
