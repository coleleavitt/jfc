//! Regression tests pinning the `NodeId` ↔ `NodeIndex` separation invariant
//! documented at the top of `crates/jfc-graph/src/graph.rs`.
//!
//! These tests exercise two complementary guarantees:
//!
//! 1. **Public API hygiene** — only `NodeId` (and aggregates of `NodeId`)
//!    appears on `CodeGraph`'s public surface. Any new method that returned
//!    `NodeIndex` would either fail to compile downstream (because the type
//!    isn't re-exported) or — if exported — break this test's assumption that
//!    public callers never see one. The test below uses ordinary `NodeId`
//!    round-trips; the assertion is structural.
//!
//! 2. **Slot-reuse safety** — `petgraph::stable_graph::StableDiGraph` may
//!    re-use the slot vacated by a removed node when the next `add_node` is
//!    issued. A caller that cached a `NodeIndex` across the cycle would point
//!    at the wrong node, silently. `NodeId` is content-addressed, so the same
//!    logical entity resolves consistently before and after the cycle, while
//!    a *different* logical entity inheriting the same slot is correctly
//!    distinguished.

use std::collections::HashMap;
use std::path::PathBuf;

use jfc_graph::edges::{EdgeData, EdgeKind};
use jfc_graph::graph::CodeGraph;
use jfc_graph::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

fn sample_span() -> Span {
    Span {
        file: PathBuf::from("src/lib.rs"),
        start_line: 1,
        start_col: 0,
        end_line: 10,
        end_col: 1,
        byte_range: 0..100,
    }
}

fn make_node(name: &str, kind: NodeKind) -> NodeData {
    let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
    NodeData {
        id,
        kind,
        name: name.to_string(),
        qualified_name: format!("crate::{name}"),
        file_path: PathBuf::from("src/lib.rs"),
        span: sample_span(),
        visibility: Visibility::Public,
        metadata: HashMap::new(),
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

fn make_edge(kind: EdgeKind) -> EdgeData {
    EdgeData {
        kind,
        source_span: sample_span(),
        weight: 1.0,
    }
}

/// Removing a node and adding a different one frees the slot for re-use.
/// The cached `NodeId` for the original node MUST NOT resolve to the new
/// node's data — a cached `NodeIndex` very well might.
#[test]
fn node_id_survives_remove_then_add_with_slot_reuse() {
    let mut graph = CodeGraph::new();

    let alpha_id = graph.add_node(make_node("alpha", NodeKind::Function));
    let beta_id = graph.add_node(make_node("beta", NodeKind::Function));

    // Sanity: distinct entities, distinct ids.
    assert_ne!(alpha_id, beta_id);

    // Drop alpha. Its underlying `NodeIndex` slot is now free for re-use.
    let removed = graph.remove_node(&alpha_id);
    assert!(removed.is_some());
    assert!(!graph.contains_node(&alpha_id));

    // Beta is unaffected — `NodeId`-keyed access remains correct across the
    // mutation, regardless of any internal slot bookkeeping.
    assert!(graph.contains_node(&beta_id));
    assert_eq!(graph.get_node(&beta_id).map(|n| n.name.as_str()), Some("beta"));

    // Insert a brand-new node. `StableDiGraph` is free to reclaim alpha's
    // vacated slot for `gamma`.
    let gamma_id = graph.add_node(make_node("gamma", NodeKind::Function));
    assert_ne!(alpha_id, gamma_id);
    assert_ne!(beta_id, gamma_id);

    // The OLD `NodeId` for alpha must still report "missing" — even if a
    // `NodeIndex` was cached and silently aliased to gamma's slot.
    assert!(!graph.contains_node(&alpha_id));
    assert!(graph.get_node(&alpha_id).is_none());

    // The new `NodeId` for gamma resolves to gamma, and only gamma.
    assert_eq!(
        graph.get_node(&gamma_id).map(|n| n.name.as_str()),
        Some("gamma"),
    );

    // Beta is still beta.
    assert_eq!(graph.get_node(&beta_id).map(|n| n.name.as_str()), Some("beta"));
}

/// Re-adding the *same* logical node after removal is consistent: the same
/// `NodeId` resolves through the lifecycle, whatever internal slot petgraph
/// hands out.
#[test]
fn node_id_round_trips_through_remove_readd_cycle() {
    let mut graph = CodeGraph::new();

    let original = make_node("foo", NodeKind::Function);
    let id_first = graph.add_node(original.clone());

    graph.remove_node(&id_first).expect("node was added");
    assert!(!graph.contains_node(&id_first));

    // Re-add the *same* logical node. `NodeId` is content-addressed, so the
    // returned id MUST equal the original — even though the internal
    // `NodeIndex` may differ.
    let id_second = graph.add_node(original);
    assert_eq!(id_first, id_second);
    assert!(graph.contains_node(&id_first));
    assert_eq!(graph.get_node(&id_first).map(|n| n.name.as_str()), Some("foo"));
}

/// Edges referencing a node by `NodeId` survive a churn cycle on a *different*
/// node. The only public identifier the caller has to keep track of is
/// `NodeId`; nothing about the internal index layout leaks.
#[test]
fn edges_remain_addressable_by_node_id_after_churn() {
    let mut graph = CodeGraph::new();

    let a_id = graph.add_node(make_node("a", NodeKind::Function));
    let b_id = graph.add_node(make_node("b", NodeKind::Function));
    let c_id = graph.add_node(make_node("c", NodeKind::Function));

    graph
        .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
        .expect("a -> b");
    graph
        .add_edge(&b_id, &c_id, make_edge(EdgeKind::Calls))
        .expect("b -> c");

    // Churn: remove c, add a fresh node `d` that may reclaim c's slot.
    graph.remove_node(&c_id);
    let d_id = graph.add_node(make_node("d", NodeKind::Function));
    assert_ne!(c_id, d_id);

    // The a -> b edge is unaffected; its endpoints are still addressable
    // *by NodeId*, which is the only identifier the public API exposes.
    let outgoing_from_a: Vec<&NodeId> =
        graph.get_edges_from(&a_id).into_iter().map(|(id, _)| id).collect();
    assert_eq!(outgoing_from_a, vec![&b_id]);

    // The b -> c edge was severed by remove_node (StableDiGraph drops incident
    // edges); it must NOT have been silently re-targeted at d.
    let outgoing_from_b: Vec<&NodeId> =
        graph.get_edges_from(&b_id).into_iter().map(|(id, _)| id).collect();
    assert!(
        outgoing_from_b.is_empty(),
        "b -> c edge must be gone — never silently re-pointed at d"
    );
}

/// Compile-time pin on the public surface. If any of these calls started
/// returning `NodeIndex` (or anything other than the documented `NodeId`-shaped
/// type) this file would fail to type-check — flagging the regression at
/// build time rather than after a runtime corruption.
#[test]
fn public_api_returns_node_id_shaped_values() {
    let mut graph = CodeGraph::new();

    // Add returns a `NodeId`, not a `NodeIndex`.
    let id: NodeId = graph.add_node(make_node("x", NodeKind::Function));
    let _: bool = graph.contains_node(&id);
    let _: Option<&NodeData> = graph.get_node(&id);
    let _: Option<NodeData> = graph.remove_node(&id);

    // `all_node_ids` yields `&NodeId`, never `NodeIndex`.
    let _: Vec<&NodeId> = graph.all_node_ids();
}
