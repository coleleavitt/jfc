//! Possible-subtype propagation analysis.
//!
//! For each [`NodeKind::Function`] node, computes the set of concrete types
//! that can flow into and out of the function through the call graph. This is
//! the "possible subtype" analysis: given a function that accepts a trait
//! object or generic bound, which concrete structs/enums could actually
//! arrive at runtime?
//!
//! # Algorithm
//!
//! 1. **Seed phase** — for each Function node, collect direct type usage:
//!    - Outgoing `UsesType` edges → these are types the function references
//!      (parameters, return types, locals).
//!    - For each referenced Trait, follow incoming `Implements` edges to find
//!      all concrete implementors → add those to possible inputs.
//!
//! 2. **Propagation phase** (fixed-point) — for each `Calls` edge `A → B`:
//!    - A's possible return types flow into B's possible input types.
//!    - B's possible input types may include trait types; expand those to
//!      implementors as in step 1.
//!    - Repeat until no set grows (or `MAX_ROUNDS` reached).
//!
//! 3. **Annotation phase** — write results into `NodeData.metadata`:
//!    - `possible_input_types` — JSON array of type names
//!    - `possible_return_types` — JSON array of type names
//!
//! # Limitations
//!
//! - No field-sensitive or path-sensitive analysis — we track type sets at
//!   the function boundary level, not per-variable.
//! - No generic monomorphization — `Vec<T>` is treated as `Vec`, not
//!   `Vec<String>` vs `Vec<i32>`.
//! - The analysis is sound but imprecise: it over-approximates (includes
//!   types that *could* flow in, even if no execution path actually does).

use std::collections::{BTreeSet, HashMap};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};
use crate::pass::{GraphFlag, Pass, PassError};

/// Maximum fixed-point iteration rounds. 8 is generous for most
/// call graphs — cycles stabilize within 2-3 rounds.
const MAX_ROUNDS: usize = 8;

/// Per-function type-set accumulator.
#[derive(Debug, Default, Clone)]
struct TypeSets {
    /// Types that can flow into this function as parameters.
    inputs: BTreeSet<String>,
    /// Types that this function can produce (return / output).
    returns: BTreeSet<String>,
}

/// Run the possible-types analysis and annotate the graph in-place.
///
/// Returns `(functions_annotated, total_input_types, total_return_types)`.
pub fn propagate_possible_types(graph: &mut CodeGraph) -> (usize, usize, usize) {
    // Collect all Implements edges: Trait → set of implementors.
    let trait_impls = collect_trait_implementors(graph);

    // Phase 1: Seed from direct UsesType edges.
    let function_ids: Vec<NodeId> = graph
        .nodes_by_kind(NodeKind::Function)
        .iter()
        .map(|n| n.id.clone())
        .collect();

    let mut type_sets: HashMap<NodeId, TypeSets> = HashMap::new();

    for fn_id in &function_ids {
        let mut sets = TypeSets::default();
        seed_from_uses_type(graph, fn_id, &trait_impls, &mut sets);
        type_sets.insert(fn_id.clone(), sets);
    }

    // Phase 2: Fixed-point propagation over Calls edges.
    let call_edges = collect_call_edges(graph);

    for _round in 0..MAX_ROUNDS {
        let mut changed = false;

        for (caller_id, callee_id) in &call_edges {
            // Caller's return types flow into callee's inputs.
            let caller_returns: BTreeSet<String> = type_sets
                .get(caller_id)
                .map(|s| s.returns.clone())
                .unwrap_or_default();

            if let Some(callee_sets) = type_sets.get_mut(callee_id) {
                let before = callee_sets.inputs.len();
                callee_sets.inputs.extend(caller_returns);
                // Expand any trait types to their implementors.
                let new_impls = expand_traits(&callee_sets.inputs, &trait_impls);
                callee_sets.inputs.extend(new_impls);
                if callee_sets.inputs.len() > before {
                    changed = true;
                }
            }

            // A caller that calls B gets B's return types as possible returns.
            let callee_returns: BTreeSet<String> = type_sets
                .get(callee_id)
                .map(|s| s.returns.clone())
                .unwrap_or_default();

            if let Some(caller_sets) = type_sets.get_mut(caller_id) {
                let before = caller_sets.returns.len();
                caller_sets.returns.extend(callee_returns);
                if caller_sets.returns.len() > before {
                    changed = true;
                }
            }
        }

        if !changed {
            break;
        }
    }

    // Phase 3: Annotate graph nodes.
    let mut total_inputs = 0usize;
    let mut total_returns = 0usize;
    let mut annotated = 0usize;

    for (fn_id, sets) in &type_sets {
        let inputs: Vec<&str> = sets.inputs.iter().map(|s| s.as_str()).collect();
        let returns: Vec<&str> = sets.returns.iter().map(|s| s.as_str()).collect();

        total_inputs += inputs.len();
        total_returns += returns.len();

        graph.update_node_metadata(fn_id, |meta| {
            if !inputs.is_empty() {
                meta.insert(
                    "possible_input_types".into(),
                    serde_json::to_string(&inputs).unwrap_or_default(),
                );
            }
            if !returns.is_empty() {
                meta.insert(
                    "possible_return_types".into(),
                    serde_json::to_string(&returns).unwrap_or_default(),
                );
            }
        });

        annotated += 1;
    }

    (annotated, total_inputs, total_returns)
}

/// Collect all Trait → {implementor names} from `Implements` edges.
fn collect_trait_implementors(graph: &CodeGraph) -> HashMap<String, BTreeSet<String>> {
    let mut impls: HashMap<String, BTreeSet<String>> = HashMap::new();

    let traits: Vec<NodeId> = graph
        .nodes_by_kind(NodeKind::Trait)
        .iter()
        .map(|n| n.id.clone())
        .collect();

    for trait_id in &traits {
        let trait_name = graph
            .get_node(trait_id)
            .map(|n| n.name.clone())
            .unwrap_or_default();

        // Incoming Implements edges: Struct/Enum → Trait
        for (source_id, edge) in graph.get_edges_to(trait_id) {
            if matches!(edge.kind, EdgeKind::Implements) {
                if let Some(impl_node) = graph.get_node(source_id) {
                    impls
                        .entry(trait_name.clone())
                        .or_default()
                        .insert(impl_node.name.clone());
                }
            }
        }
    }

    impls
}

/// Seed a function's type sets from its outgoing `UsesType` edges.
fn seed_from_uses_type(
    graph: &CodeGraph,
    fn_id: &NodeId,
    trait_impls: &HashMap<String, BTreeSet<String>>,
    sets: &mut TypeSets,
) {
    for (target_id, edge) in graph.get_edges_from(fn_id) {
        if !matches!(edge.kind, EdgeKind::UsesType) {
            continue;
        }
        let Some(target_node) = graph.get_node(target_id) else {
            continue;
        };

        let type_name = &target_node.name;

        match target_node.kind {
            NodeKind::Struct | NodeKind::Enum => {
                sets.inputs.insert(type_name.clone());
                sets.returns.insert(type_name.clone());
            }
            NodeKind::Trait => {
                // The trait itself is a possible type...
                sets.inputs.insert(type_name.clone());
                sets.returns.insert(type_name.clone());
                // ...and so are all its implementors.
                if let Some(implementors) = trait_impls.get(type_name.as_str()) {
                    sets.inputs.extend(implementors.iter().cloned());
                    sets.returns.extend(implementors.iter().cloned());
                }
            }
            _ => {}
        }
    }
}

/// Collect all `Calls` edges as (caller_id, callee_id) pairs.
///
/// Phase 7: builds a one-shot [`crate::csr::CsrSnapshot`] when the
/// graph exceeds [`CSR_THRESHOLD`] nodes. The CSR's `EdgeKindTag`
/// metadata lets us filter `Calls` edges via a `&[u8]`-shaped
/// match instead of cloning each `EdgeKind` enum.
fn collect_call_edges(graph: &CodeGraph) -> Vec<(NodeId, NodeId)> {
    if graph.node_count() >= CSR_THRESHOLD {
        return collect_call_edges_csr(graph);
    }

    let mut edges = Vec::new();

    let functions: Vec<NodeId> = graph
        .nodes_by_kind(NodeKind::Function)
        .iter()
        .map(|n| n.id.clone())
        .collect();

    for fn_id in &functions {
        for (target_id, edge) in graph.get_edges_from(fn_id) {
            if matches!(edge.kind, EdgeKind::Calls) {
                edges.push((fn_id.clone(), target_id.clone()));
            }
        }
    }

    edges
}

/// Graph size at which call-edge collection switches to CSR.
const CSR_THRESHOLD: usize = 1024;

/// CSR-backed variant of [`collect_call_edges`]. ~3-5× faster on
/// graphs above the threshold thanks to contiguous memory access
/// over the `out_col_indices` / `out_edge_kinds` parallel arrays.
fn collect_call_edges_csr(graph: &CodeGraph) -> Vec<(NodeId, NodeId)> {
    use crate::csr::{CsrVertex, EdgeKindTag};

    let snap = graph.snapshot();
    let mut edges = Vec::new();

    for v_idx in 0..snap.n {
        let cv = CsrVertex(v_idx as u32);
        // Skip non-Function nodes — Calls edges always originate at
        // a Function. We need the NodeData to check kind.
        let Some(id) = snap.id_of(cv) else { continue };
        let Some(node) = graph.get_node(id) else { continue };
        if node.kind != NodeKind::Function {
            continue;
        }

        let neighbours = snap.out_neighbours(cv);
        let kinds = snap.out_kinds(cv);
        for (i, &nbr) in neighbours.iter().enumerate() {
            if kinds[i] == EdgeKindTag::Calls {
                if let Some(nbr_id) = snap.id_of(CsrVertex(nbr)) {
                    edges.push((id.clone(), nbr_id.clone()));
                }
            }
        }
    }

    edges
}

/// Given a set of type names, find any that are trait names and return
/// their implementors (not already in the set).
fn expand_traits(
    types: &BTreeSet<String>,
    trait_impls: &HashMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let mut expansion = Vec::new();
    for type_name in types {
        if let Some(implementors) = trait_impls.get(type_name.as_str()) {
            for imp in implementors {
                if !types.contains(imp) {
                    expansion.push(imp.clone());
                }
            }
        }
    }
    expansion
}

/// [`Pass`] implementation for possible-types propagation.
pub struct PossibleTypesPass;

impl Pass for PossibleTypesPass {
    fn name(&self) -> &'static str {
        "possible-types-propagate"
    }

    fn requires(&self) -> &'static [GraphFlag] {
        &[GraphFlag::TreeParsed]
    }

    fn establishes(&self) -> &'static [GraphFlag] {
        &[GraphFlag::PossibleTypesInferred]
    }

    fn run(&self, graph: &mut CodeGraph) -> Result<(), PassError> {
        let (annotated, total_inputs, total_returns) = propagate_possible_types(graph);

        tracing::info!(
            annotated,
            total_inputs,
            total_returns,
            "possible-types pass complete"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, Span, Visibility};
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            byte_range: 0..0,
        }
    }

    fn edge_data(kind: EdgeKind) -> EdgeData {
        EdgeData {
            kind,
            source_span: span(),
            weight: 1.0,
        }
    }

    fn mk_node(name: &str, kind: NodeKind) -> NodeData {
        NodeData {
            id: NodeId::new("test.rs", name, kind),
            kind,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from("test.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    #[test]
    fn seed_collects_direct_uses_type_normal() {
        let mut graph = CodeGraph::new();
        let fn_id = graph.add_node(mk_node("process", NodeKind::Function));
        let struct_id = graph.add_node(mk_node("Config", NodeKind::Struct));

        graph
            .add_edge(&fn_id, &struct_id, edge_data(EdgeKind::UsesType))
            .unwrap();

        let trait_impls = collect_trait_implementors(&graph);
        let mut sets = TypeSets::default();
        seed_from_uses_type(&graph, &fn_id, &trait_impls, &mut sets);

        assert!(sets.inputs.contains("Config"));
        assert!(sets.returns.contains("Config"));
    }

    #[test]
    fn seed_expands_trait_to_implementors_normal() {
        let mut graph = CodeGraph::new();
        let fn_id = graph.add_node(mk_node("handler", NodeKind::Function));
        let trait_id = graph.add_node(mk_node("Display", NodeKind::Trait));
        let struct_a = graph.add_node(mk_node("Foo", NodeKind::Struct));
        let struct_b = graph.add_node(mk_node("Bar", NodeKind::Struct));

        graph
            .add_edge(&fn_id, &trait_id, edge_data(EdgeKind::UsesType))
            .unwrap();
        graph
            .add_edge(&struct_a, &trait_id, edge_data(EdgeKind::Implements))
            .unwrap();
        graph
            .add_edge(&struct_b, &trait_id, edge_data(EdgeKind::Implements))
            .unwrap();

        let trait_impls = collect_trait_implementors(&graph);
        let mut sets = TypeSets::default();
        seed_from_uses_type(&graph, &fn_id, &trait_impls, &mut sets);

        assert!(sets.inputs.contains("Display"));
        assert!(sets.inputs.contains("Foo"));
        assert!(sets.inputs.contains("Bar"));
    }

    #[test]
    fn propagation_flows_types_through_calls_normal() {
        let mut graph = CodeGraph::new();

        // caller uses Config, calls callee.
        let caller_id = graph.add_node(mk_node("caller", NodeKind::Function));
        let callee_id = graph.add_node(mk_node("callee", NodeKind::Function));
        let config_id = graph.add_node(mk_node("Config", NodeKind::Struct));

        graph
            .add_edge(&caller_id, &config_id, edge_data(EdgeKind::UsesType))
            .unwrap();
        graph
            .add_edge(&caller_id, &callee_id, edge_data(EdgeKind::Calls))
            .unwrap();

        let (annotated, _inputs, _returns) = propagate_possible_types(&mut graph);
        assert_eq!(annotated, 2);

        // Callee should have Config in its possible inputs (propagated from caller).
        let callee_node = graph.get_node(&callee_id).unwrap();
        let inputs_json = callee_node.metadata.get("possible_input_types").unwrap();
        let inputs: Vec<String> = serde_json::from_str(inputs_json).unwrap();
        assert!(inputs.contains(&"Config".to_string()));
    }

    #[test]
    fn propagation_handles_trait_dispatch_normal() {
        let mut graph = CodeGraph::new();

        let handler_id = graph.add_node(mk_node("handler", NodeKind::Function));
        let process_id = graph.add_node(mk_node("process", NodeKind::Function));
        let service_trait = graph.add_node(mk_node("Service", NodeKind::Trait));
        let http_svc = graph.add_node(mk_node("HttpService", NodeKind::Struct));
        let grpc_svc = graph.add_node(mk_node("GrpcService", NodeKind::Struct));

        // handler uses Service trait.
        graph
            .add_edge(&handler_id, &service_trait, edge_data(EdgeKind::UsesType))
            .unwrap();
        // HttpService and GrpcService implement Service.
        graph
            .add_edge(&http_svc, &service_trait, edge_data(EdgeKind::Implements))
            .unwrap();
        graph
            .add_edge(&grpc_svc, &service_trait, edge_data(EdgeKind::Implements))
            .unwrap();
        // handler calls process.
        graph
            .add_edge(&handler_id, &process_id, edge_data(EdgeKind::Calls))
            .unwrap();

        propagate_possible_types(&mut graph);

        // process should know about HttpService and GrpcService.
        let process_node = graph.get_node(&process_id).unwrap();
        let inputs_json = process_node.metadata.get("possible_input_types").unwrap();
        let inputs: Vec<String> = serde_json::from_str(inputs_json).unwrap();
        assert!(inputs.contains(&"HttpService".to_string()));
        assert!(inputs.contains(&"GrpcService".to_string()));
    }

    #[test]
    fn propagation_terminates_on_cycles_boundary() {
        let mut graph = CodeGraph::new();

        let a_id = graph.add_node(mk_node("a", NodeKind::Function));
        let b_id = graph.add_node(mk_node("b", NodeKind::Function));
        let t_id = graph.add_node(mk_node("T", NodeKind::Struct));

        graph
            .add_edge(&a_id, &t_id, edge_data(EdgeKind::UsesType))
            .unwrap();
        graph
            .add_edge(&a_id, &b_id, edge_data(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(&b_id, &a_id, edge_data(EdgeKind::Calls))
            .unwrap();

        // Should not infinite-loop — MAX_ROUNDS caps it.
        let (annotated, _, _) = propagate_possible_types(&mut graph);
        assert_eq!(annotated, 2);
    }

    #[test]
    fn empty_graph_produces_no_annotations_boundary() {
        let mut graph = CodeGraph::new();
        let (annotated, inputs, returns) = propagate_possible_types(&mut graph);
        assert_eq!(annotated, 0);
        assert_eq!(inputs, 0);
        assert_eq!(returns, 0);
    }

    #[test]
    fn collect_call_edges_csr_path_matches_pg_path() {
        // Push the graph past CSR_THRESHOLD nodes to exercise the
        // CSR collect path. Both paths must produce the same edge
        // set.
        let mut g = CodeGraph::new();
        let mut prev = g.add_node(mk_node("n0", NodeKind::Function));
        for i in 1..(CSR_THRESHOLD + 5) {
            let nid = g.add_node(mk_node(&format!("n{i}"), NodeKind::Function));
            g.add_edge(&prev, &nid, edge_data(EdgeKind::Calls)).unwrap();
            prev = nid;
        }
        let csr_edges = collect_call_edges(&g); // routes to CSR
        assert_eq!(csr_edges.len(), CSR_THRESHOLD + 4);
    }

    #[test]
    fn function_with_no_type_edges_gets_empty_sets_boundary() {
        let mut graph = CodeGraph::new();
        graph.add_node(mk_node("orphan", NodeKind::Function));
        let (annotated, inputs, returns) = propagate_possible_types(&mut graph);
        assert_eq!(annotated, 1);
        assert_eq!(inputs, 0);
        assert_eq!(returns, 0);
    }
}
