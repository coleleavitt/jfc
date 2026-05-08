//! Regression tests for the iteration-order-independence of [`Fingerprint`].
//!
//! See `crates/jfc-graph/src/fingerprint.rs` for the contract being pinned
//! here. The bug class these tests prevent: a graph fingerprint that varies
//! between runs because some intermediate `HashMap` was iterated directly,
//! exposing the per-process random seed.
//!
//! - `fingerprint_is_stable_across_insert_orders` — same nodes/edges,
//!   different insertion order → identical fingerprint.
//! - `fingerprint_changes_on_node_addition_robust` — adding a real node
//!   shifts the digest (sanity check that we're not all-zero).
//! - `fingerprint_changes_on_metadata_change_robust` — the
//!   `NodeData::metadata` `HashMap` participates in the digest (this is the
//!   field most prone to iteration-order leakage).
//! - `fingerprint_ignores_metadata_insertion_order` — the canonicalization
//!   inside `update_unordered_map` actually erases insertion order.

use std::collections::HashMap;
use std::path::PathBuf;

use jfc_graph::edges::{EdgeData, EdgeKind};
use jfc_graph::fingerprint::Fingerprintable;
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

fn make_node_with_metadata(
    name: &str,
    kind: NodeKind,
    metadata: HashMap<String, String>,
) -> NodeData {
    let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
    NodeData {
        id,
        kind,
        name: name.to_string(),
        qualified_name: format!("crate::{name}"),
        file_path: PathBuf::from("src/lib.rs"),
        span: sample_span(),
        visibility: Visibility::Public,
        metadata,
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

fn make_node(name: &str, kind: NodeKind) -> NodeData {
    make_node_with_metadata(name, kind, HashMap::new())
}

fn make_edge(kind: EdgeKind) -> EdgeData {
    EdgeData {
        kind,
        source_span: sample_span(),
        weight: 1.0,
    }
}

#[test]
fn fingerprint_is_stable_across_insert_orders() {
    // Build the same logical graph in two different insertion orders.
    let mut g1 = CodeGraph::new();
    let alpha_id = g1.add_node(make_node("alpha", NodeKind::Function));
    let beta_id = g1.add_node(make_node("beta", NodeKind::Function));
    let gamma_id = g1.add_node(make_node("gamma", NodeKind::Function));
    g1.add_edge(&alpha_id, &beta_id, make_edge(EdgeKind::Calls))
        .unwrap();
    g1.add_edge(&beta_id, &gamma_id, make_edge(EdgeKind::Calls))
        .unwrap();
    g1.add_edge(&alpha_id, &gamma_id, make_edge(EdgeKind::Calls))
        .unwrap();

    let mut g2 = CodeGraph::new();
    // Reverse node order; reverse edge order; verify fingerprint matches.
    let gamma_id_2 = g2.add_node(make_node("gamma", NodeKind::Function));
    let beta_id_2 = g2.add_node(make_node("beta", NodeKind::Function));
    let alpha_id_2 = g2.add_node(make_node("alpha", NodeKind::Function));
    g2.add_edge(&alpha_id_2, &gamma_id_2, make_edge(EdgeKind::Calls))
        .unwrap();
    g2.add_edge(&beta_id_2, &gamma_id_2, make_edge(EdgeKind::Calls))
        .unwrap();
    g2.add_edge(&alpha_id_2, &beta_id_2, make_edge(EdgeKind::Calls))
        .unwrap();

    // The NodeIds derive deterministically from (path, qualified_name, kind),
    // so they should match across the two graphs.
    assert_eq!(alpha_id, alpha_id_2);
    assert_eq!(beta_id, beta_id_2);
    assert_eq!(gamma_id, gamma_id_2);

    assert_eq!(
        g1.fingerprint(),
        g2.fingerprint(),
        "fingerprint must be insert-order-independent for identical graphs"
    );
}

#[test]
fn fingerprint_changes_on_node_addition_robust() {
    let mut g = CodeGraph::new();
    g.add_node(make_node("alpha", NodeKind::Function));
    let one_node = g.fingerprint();

    g.add_node(make_node("beta", NodeKind::Function));
    let two_nodes = g.fingerprint();

    assert_ne!(
        one_node, two_nodes,
        "adding a real node must change the fingerprint"
    );
}

#[test]
fn fingerprint_changes_on_edge_addition_robust() {
    let mut g = CodeGraph::new();
    let a = g.add_node(make_node("alpha", NodeKind::Function));
    let b = g.add_node(make_node("beta", NodeKind::Function));
    let no_edges = g.fingerprint();

    g.add_edge(&a, &b, make_edge(EdgeKind::Calls)).unwrap();
    let with_edge = g.fingerprint();

    assert_ne!(
        no_edges, with_edge,
        "adding an edge must change the fingerprint"
    );
}

#[test]
fn fingerprint_ignores_metadata_insertion_order() {
    // The metadata HashMap is exactly the kind of unordered container that
    // would leak iteration order into the digest if hashed naively. Build
    // the same metadata map in two different insertion orders and verify
    // the fingerprint matches.
    let mut meta_a = HashMap::new();
    meta_a.insert("alpha".to_string(), "1".to_string());
    meta_a.insert("beta".to_string(), "2".to_string());
    meta_a.insert("gamma".to_string(), "3".to_string());

    let mut meta_b = HashMap::new();
    meta_b.insert("gamma".to_string(), "3".to_string());
    meta_b.insert("alpha".to_string(), "1".to_string());
    meta_b.insert("beta".to_string(), "2".to_string());

    let mut g1 = CodeGraph::new();
    g1.add_node(make_node_with_metadata("foo", NodeKind::Function, meta_a));

    let mut g2 = CodeGraph::new();
    g2.add_node(make_node_with_metadata("foo", NodeKind::Function, meta_b));

    assert_eq!(
        g1.fingerprint(),
        g2.fingerprint(),
        "metadata HashMap insertion order must not influence the fingerprint"
    );
}

#[test]
fn fingerprint_changes_on_metadata_change_robust() {
    // Sanity: the metadata map *does* participate in the digest. If we
    // accidentally skipped it, fingerprints would falsely collide for nodes
    // that differ only in metadata.
    let mut meta_v1 = HashMap::new();
    meta_v1.insert("async".to_string(), "false".to_string());

    let mut meta_v2 = HashMap::new();
    meta_v2.insert("async".to_string(), "true".to_string());

    let mut g1 = CodeGraph::new();
    g1.add_node(make_node_with_metadata("foo", NodeKind::Function, meta_v1));

    let mut g2 = CodeGraph::new();
    g2.add_node(make_node_with_metadata("foo", NodeKind::Function, meta_v2));

    assert_ne!(
        g1.fingerprint(),
        g2.fingerprint(),
        "metadata-only changes must change the fingerprint"
    );
}

#[test]
fn fingerprint_distinguishes_different_edges_normal() {
    // Two graphs with the same nodes but different edges must fingerprint
    // differently. This pins the edge half of the digest.
    let mut g_calls = CodeGraph::new();
    let a = g_calls.add_node(make_node("alpha", NodeKind::Function));
    let b = g_calls.add_node(make_node("beta", NodeKind::Function));
    g_calls
        .add_edge(&a, &b, make_edge(EdgeKind::Calls))
        .unwrap();

    let mut g_references = CodeGraph::new();
    let a2 = g_references.add_node(make_node("alpha", NodeKind::Function));
    let b2 = g_references.add_node(make_node("beta", NodeKind::Function));
    g_references
        .add_edge(&a2, &b2, make_edge(EdgeKind::References))
        .unwrap();

    assert_ne!(
        g_calls.fingerprint(),
        g_references.fingerprint(),
        "different edge kinds between the same nodes must fingerprint differently"
    );
}
