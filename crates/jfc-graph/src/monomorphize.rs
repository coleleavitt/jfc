//! Generic monomorphization detection and annotation.
//!
//! Scans the code graph for generic function/struct nodes and callsites that
//! supply concrete type arguments, then annotates the graph with per-function
//! instantiation summaries.
//!
//! # Why monomorphize?
//!
//! The [`crate::possible_types`] analysis currently treats `Vec<T>` as `Vec`,
//! so `Vec<User>` and `Vec<Log>` are indistinguishable. By detecting which
//! concrete type arguments flow into generic callsites, downstream consumers
//! (type-sensitive taint, alias analysis) can reason about
//! `HashMap<String, SecretKey>` separately from `HashMap<String, usize>`.
//!
//! # Adapter contract
//!
//! Language adapters surface generic information by writing well-known keys
//! into [`crate::nodes::NodeData::metadata`]:
//!
//! | Key               | Who writes it          | Example value              |
//! |-------------------|------------------------|----------------------------|
//! | `generic_params`  | Adapter, on the *decl* | `["T", "U"]`               |
//! | `type_args`       | Adapter, on the *call* | `["String", "i32"]`        |
//!
//! Both are JSON-encoded string arrays. `generic_params` lives on the
//! declaring Function/Struct/Enum node; `type_args` lives on the calling
//! Function node's metadata under a callsite-keyed sub-object, OR on the
//! `Calls` edge's [`crate::edges::EdgeData::metadata`].
//!
//! # Precision
//!
//! Full type-arg recovery needs a type inferencer we don't have yet.
//! Adapters write metadata where they can (Rust turbofish `foo::<T>()`,
//! TypeScript `foo<T>()`, Go `foo[T]()`, Python type-comment generics).
//! This module is a best-effort surface: it reports what the adapters
//! provided and annotates the graph; it does NOT infer missing args.

use std::collections::{BTreeMap, BTreeSet};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};

/// A concrete instantiation of a generic function/struct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericInstantiation {
    /// The generic function/struct that was instantiated.
    pub generic_id: NodeId,
    /// Concrete type arguments at this callsite (in parameter order).
    pub type_args: Vec<String>,
    /// The callsite (caller function) that triggered this instantiation.
    pub callsite_id: NodeId,
}

/// Scan the graph for generic nodes and callsites that supply type args.
///
/// Returns one [`GenericInstantiation`] per unique (generic_id, callsite_id)
/// pair. If the same generic is called from 5 places with different type
/// args, there are 5 entries.
pub fn find_instantiations(graph: &CodeGraph) -> Vec<GenericInstantiation> {
    let generic_ids: BTreeSet<NodeId> = [NodeKind::Function, NodeKind::Struct, NodeKind::Enum]
        .iter()
        .flat_map(|k| graph.nodes_by_kind(*k))
        .filter(|n| is_generic_node(n))
        .map(|n| n.id.clone())
        .collect();

    if generic_ids.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    for generic_id in &generic_ids {
        collect_instantiations_for(graph, generic_id, &mut results);
    }
    results
}

fn is_generic_node(n: &crate::nodes::NodeData) -> bool {
    matches!(
        n.kind,
        NodeKind::Function | NodeKind::Struct | NodeKind::Enum
    ) && n
        .metadata
        .get("generic_params")
        .map(|v| v.starts_with('[') && v != "[]")
        .unwrap_or(false)
}

fn collect_instantiations_for(
    graph: &CodeGraph,
    generic_id: &NodeId,
    out: &mut Vec<GenericInstantiation>,
) {
    for (caller_id, edge) in graph.get_edges_to(generic_id) {
        if !matches!(edge.kind, EdgeKind::Calls) {
            continue;
        }
        try_caller_type_args(graph, generic_id, caller_id, out);
    }
}

fn try_caller_type_args(
    graph: &CodeGraph,
    generic_id: &NodeId,
    caller_id: &NodeId,
    out: &mut Vec<GenericInstantiation>,
) {
    let Some(caller_node) = graph.get_node(caller_id) else {
        return;
    };
    let Some(cta) = caller_node.metadata.get("callee_type_args") else {
        return;
    };
    let Ok(map) = serde_json::from_str::<BTreeMap<String, Vec<String>>>(cta) else {
        return;
    };
    let callee_name = graph
        .get_node(generic_id)
        .map(|n| n.name.as_str())
        .unwrap_or("");
    let Some(args) = map.get(callee_name) else {
        return;
    };
    if args.is_empty() {
        return;
    }
    out.push(GenericInstantiation {
        generic_id: generic_id.clone(),
        type_args: args.clone(),
        callsite_id: caller_id.clone(),
    });
}

/// Annotate generic nodes with a `mono_instances` metadata key (JSON array).
///
/// Each entry in the array is `{"callsite": "<id>", "type_args": ["T1", ...]}`.
/// Returns the number of generic nodes annotated.
pub fn annotate(graph: &mut CodeGraph) -> usize {
    let instances = find_instantiations(graph);
    if instances.is_empty() {
        return 0;
    }

    // Group by generic_id.
    let mut grouped: BTreeMap<NodeId, Vec<serde_json::Value>> = BTreeMap::new();
    for inst in &instances {
        grouped
            .entry(inst.generic_id.clone())
            .or_default()
            .push(serde_json::json!({
                "callsite": inst.callsite_id.0,
                "type_args": inst.type_args,
            }));
    }

    let mut annotated = 0usize;
    for (gid, entries) in &grouped {
        graph.update_node_metadata(gid, |meta| {
            meta.insert(
                "mono_instances".into(),
                serde_json::to_string(entries).unwrap_or_default(),
            );
        });
        annotated += 1;
    }

    annotated
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::graph::CodeGraph;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 10,
            byte_range: 0..10,
        }
    }

    fn make_node(name: &str, kind: NodeKind, meta: HashMap<String, String>) -> NodeData {
        NodeData {
            id: NodeId::new("test.rs", name, kind),
            name: name.into(),
            qualified_name: name.into(),
            kind,
            file_path: PathBuf::from("test.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: meta,
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn no_generics_yields_empty() {
        let mut g = CodeGraph::new();
        g.add_node(make_node("foo", NodeKind::Function, HashMap::new()));
        let insts = find_instantiations(&g);
        assert!(insts.is_empty());
    }

    fn edge(kind: EdgeKind) -> EdgeData {
        EdgeData {
            kind,
            source_span: span(),
            weight: 1.0,
        }
    }

    #[test]
    fn finds_instantiation_from_caller_metadata() {
        let mut g = CodeGraph::new();
        let mut gp: HashMap<String, String> = HashMap::new();
        gp.insert("generic_params".into(), r#"["T"]"#.into());
        let vec_new_id = NodeId::new("test.rs", "new", NodeKind::Function);
        let caller_id = NodeId::new("test.rs", "main", NodeKind::Function);
        g.add_node(make_node("new", NodeKind::Function, gp));

        // Caller node carries callee_type_args.
        let mut cm: HashMap<String, String> = HashMap::new();
        cm.insert("callee_type_args".into(), r#"{"new": ["String"]}"#.into());
        g.add_node(make_node("main", NodeKind::Function, cm));
        g.add_edge(&caller_id, &vec_new_id, edge(EdgeKind::Calls))
            .unwrap();

        let insts = find_instantiations(&g);
        assert_eq!(insts.len(), 1);
        assert_eq!(insts[0].type_args, vec!["String"]);
        assert_eq!(insts[0].generic_id, vec_new_id);
    }

    #[test]
    fn annotate_writes_mono_instances() {
        let mut g = CodeGraph::new();
        let mut gp: HashMap<String, String> = HashMap::new();
        gp.insert("generic_params".into(), r#"["T"]"#.into());
        let gfn_id = NodeId::new("test.rs", "identity", NodeKind::Function);
        let caller_id = NodeId::new("test.rs", "user_code", NodeKind::Function);
        g.add_node(make_node("identity", NodeKind::Function, gp));

        let mut cm: HashMap<String, String> = HashMap::new();
        cm.insert("callee_type_args".into(), r#"{"identity": ["i32"]}"#.into());
        g.add_node(make_node("user_code", NodeKind::Function, cm));
        g.add_edge(&caller_id, &gfn_id, edge(EdgeKind::Calls))
            .unwrap();

        let count = annotate(&mut g);
        assert_eq!(count, 1);
        let node = g.get_node(&gfn_id).unwrap();
        let mi = node.metadata.get("mono_instances").unwrap();
        assert!(mi.contains("i32"), "expected 'i32' in {mi}");
    }

    #[test]
    fn no_callee_type_args_yields_empty() {
        let mut g = CodeGraph::new();
        let mut gp: HashMap<String, String> = HashMap::new();
        gp.insert("generic_params".into(), r#"["T"]"#.into());
        let gfn_id = NodeId::new("test.rs", "identity", NodeKind::Function);
        let caller_id = NodeId::new("test.rs", "caller", NodeKind::Function);
        g.add_node(make_node("identity", NodeKind::Function, gp));
        // Caller has NO callee_type_args.
        g.add_node(make_node("caller", NodeKind::Function, HashMap::new()));
        g.add_edge(&caller_id, &gfn_id, edge(EdgeKind::Calls))
            .unwrap();

        let insts = find_instantiations(&g);
        assert!(insts.is_empty());
    }

    /// Integration test: parse Rust source with turbofish, build the graph
    /// via the adapter + resolver, then run `find_instantiations`.
    #[test]
    fn integration_rust_adapter_turbofish() {
        use crate::adapter::LanguageAdapter;
        use crate::adapter::rust::RustAdapter;
        use crate::resolver::ReferenceResolver;
        use std::path::PathBuf;

        let adapter = RustAdapter::new();
        let path = PathBuf::from("turbofish.rs");
        let src = r#"
fn identity<T>(x: T) -> T { x }

fn caller() {
    identity::<String>("hello");
}
"#;
        let parsed = adapter.parse_file(&path, src).expect("parse");
        let nodes = adapter.extract_nodes(&parsed);

        let mut graph = CodeGraph::new();
        for node in &nodes {
            graph.add_node(node.clone());
        }

        // Resolve call edges the same way the builder does.
        let sites = adapter.extract_call_sites(&parsed, &nodes);
        if !sites.is_empty() {
            let mut resolver = ReferenceResolver::new(&mut graph);
            resolver.resolve_all(&sites);
        }

        let insts = find_instantiations(&graph);
        assert_eq!(insts.len(), 1, "expected one instantiation, got: {insts:?}");
        assert_eq!(insts[0].type_args, vec!["String"]);

        // Also verify the generic_id points to the identity function.
        let identity_node = nodes
            .iter()
            .find(|n| n.name == "identity")
            .expect("identity");
        assert_eq!(insts[0].generic_id, identity_node.id);
    }
}
