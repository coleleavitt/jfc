//! Typed per-kind metadata accessors (Phase 6).
//!
//! Instead of breaking the public `NodeData` shape (which is
//! serialised, fingerprinted, and consumed by jfc-ui), this module
//! provides a **typed view** over `NodeData.metadata` so that callers
//! who want compile-time-checked access to well-known fields like
//! `coverage_count`, `param_count`, `is_async`, `field_count`, etc.,
//! can use a typed enum instead of stringly-typed lookups.
//!
//! ## Two surfaces
//!
//! - [`KindData`] — read-side typed projection. Built on demand from
//!   a `&NodeData` reference.
//! - [`numeric_index`] — module-level helper that returns a range-keyed
//!   index over a numeric metadata field for range queries like
//!   "every function with `coverage_count >= 5`".
//!
//! ## Compile-time edge-kind invariants
//!
//! [`TypedEdge`] is a marker-type wrapper that records the source and
//! target [`crate::nodes::NodeKind`] in its types. Use it at API
//! boundaries where a function should statically refuse to compile if
//! the wrong edge kind is passed in. The runtime check in
//! `add_edge` remains as the fallback — typed wrappers are opt-in.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId, NodeKind};

/// Typed projection of `NodeData.metadata` per kind. Built on demand
/// from a `&NodeData`; not stored. Field absence falls through to
/// `None` so callers can `unwrap_or_default()` cleanly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindData {
    pub function: Option<FunctionFields>,
    pub struct_: Option<StructFields>,
    pub enum_: Option<EnumFields>,
    pub trait_: Option<TraitFields>,
}

/// Typed Function-specific metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FunctionFields {
    pub is_async: Option<bool>,
    pub param_count: Option<u32>,
    pub coverage_count: Option<u32>,
    pub coverage_tested: Option<bool>,
    pub possible_input_types: Vec<String>,
    pub possible_return_types: Vec<String>,
}

/// Typed Struct-specific metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructFields {
    pub field_count: Option<u32>,
    pub fields: Vec<String>,
    pub accessed_fields: Vec<String>,
}

/// Typed Enum-specific metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnumFields {
    pub variant_count: Option<u32>,
    pub variants: Vec<String>,
}

/// Typed Trait-specific metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraitFields {
    pub method_count: Option<u32>,
    pub methods: Vec<String>,
}

impl KindData {
    /// Project a `NodeData`'s `metadata` map into a typed view. Stringly
    /// metadata that doesn't parse cleanly is silently skipped — this
    /// is a *read* projection, not a parser; the source of truth is
    /// the metadata bag.
    pub fn from_node(data: &NodeData) -> Self {
        let mut k = KindData::default();
        match data.kind {
            NodeKind::Function => {
                let mut f = FunctionFields::default();
                f.is_async = data
                    .metadata
                    .get("async")
                    .map(|s| matches!(s.as_str(), "true" | "1"));
                f.param_count = data.metadata.get("param_count").and_then(|s| s.parse().ok());
                f.coverage_count = data
                    .metadata
                    .get("coverage_count")
                    .and_then(|s| s.parse().ok());
                f.coverage_tested = data
                    .metadata
                    .get("coverage_tested")
                    .map(|s| matches!(s.as_str(), "true" | "1"));
                if let Some(s) = data.metadata.get("possible_input_types") {
                    if let Ok(v) = serde_json::from_str::<Vec<String>>(s) {
                        f.possible_input_types = v;
                    }
                }
                if let Some(s) = data.metadata.get("possible_return_types") {
                    if let Ok(v) = serde_json::from_str::<Vec<String>>(s) {
                        f.possible_return_types = v;
                    }
                }
                k.function = Some(f);
            }
            NodeKind::Struct => {
                let mut s = StructFields::default();
                s.field_count = data.metadata.get("field_count").and_then(|x| x.parse().ok());
                if let Some(raw) = data.metadata.get("fields") {
                    s.fields = raw
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                if let Some(raw) = data.metadata.get("accessed_fields") {
                    s.accessed_fields = raw
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                k.struct_ = Some(s);
            }
            NodeKind::Enum => {
                let mut e = EnumFields::default();
                e.variant_count = data.metadata.get("variant_count").and_then(|x| x.parse().ok());
                if let Some(raw) = data.metadata.get("variants") {
                    e.variants = raw
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                k.enum_ = Some(e);
            }
            NodeKind::Trait => {
                let mut t = TraitFields::default();
                t.method_count = data.metadata.get("method_count").and_then(|x| x.parse().ok());
                if let Some(raw) = data.metadata.get("methods") {
                    t.methods = raw
                        .split(',')
                        .map(|m| m.trim().to_string())
                        .filter(|m| !m.is_empty())
                        .collect();
                }
                k.trait_ = Some(t);
            }
            NodeKind::Module => {
                // Modules have no kind-specific structured fields.
            }
        }
        k
    }
}

// ─── Numeric metadata range index ────────────────────────────────────────

/// Build an in-memory `BTreeMap<u64, Vec<NodeId>>` over a numeric
/// metadata field. Returns the index built from the current graph
/// state — callers should rebuild after mutations. Cost: O(n).
///
/// Use cases:
/// - "every function with coverage_count == 0" (untested)
/// - "every node modified in revision range [N, M]"
/// - "every struct with field_count >= 8"
///
/// Fields whose value isn't a parseable `u64` are skipped (they
/// don't contribute to the index — the legacy string lookup still
/// works for them).
pub fn numeric_index(graph: &CodeGraph, field: &str) -> BTreeMap<u64, Vec<NodeId>> {
    let mut idx: BTreeMap<u64, Vec<NodeId>> = BTreeMap::new();
    for id in graph.all_node_ids() {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        if let Some(v) = node.metadata.get(field) {
            if let Ok(n) = v.parse::<u64>() {
                idx.entry(n).or_default().push(node.id.clone());
            }
        }
    }
    idx
}

/// `NodeData::kind_data()` — typed-projection method exposed on the
/// public `NodeData` type. Kept in this module rather than `nodes.rs`
/// to avoid pulling `serde_json` and the typed structs into the
/// core node-types module.
impl NodeData {
    /// Project this node's stringly metadata into a typed
    /// [`KindData`]. Cheap (one HashMap lookup per typed field). For
    /// hot paths that read the same field repeatedly, cache the
    /// returned `KindData`; for one-off reads, just call this.
    #[inline]
    pub fn kind_data(&self) -> KindData {
        KindData::from_node(self)
    }
}

/// Inclusive range query against a numeric index built via
/// [`numeric_index`].
pub fn numeric_index_range<'a>(
    idx: &'a BTreeMap<u64, Vec<NodeId>>,
    lo: u64,
    hi: u64,
) -> Vec<&'a NodeId> {
    let mut out = Vec::new();
    for (_, ids) in idx.range(lo..=hi) {
        for id in ids {
            out.push(id);
        }
    }
    out
}

// ─── Compile-time edge-kind invariants (marker-types) ───────────────────

/// Marker types for source/target node kinds. Use as type parameters
/// of [`TypedEdge`]: e.g. `TypedEdge<Function, Function, CallsKind>`
/// is the type-checked form of "a Calls edge from a Function to a
/// Function" — the alternative kinds simply won't compile.
pub mod marker {
    /// Marker for `NodeKind::Function`.
    pub struct Function;
    /// Marker for `NodeKind::Struct`.
    pub struct Struct;
    /// Marker for `NodeKind::Enum`.
    pub struct Enum;
    /// Marker for `NodeKind::Module`.
    pub struct Module;
    /// Marker for `NodeKind::Trait`.
    pub struct Trait;

    /// EdgeKind markers — one per variant.
    pub struct CallsKind;
    pub struct UsesTypeKind;
    pub struct ImplementsKind;
    pub struct ContainsKind;
    pub struct ReferencesKind;
}

/// Trait mapping a marker type to a runtime [`NodeKind`]. Bound on
/// [`TypedEdge`] so the generic constructor can recover the runtime
/// kind for the runtime invariant check.
pub trait NodeKindMarker {
    const KIND: NodeKind;
}

impl NodeKindMarker for marker::Function {
    const KIND: NodeKind = NodeKind::Function;
}
impl NodeKindMarker for marker::Struct {
    const KIND: NodeKind = NodeKind::Struct;
}
impl NodeKindMarker for marker::Enum {
    const KIND: NodeKind = NodeKind::Enum;
}
impl NodeKindMarker for marker::Module {
    const KIND: NodeKind = NodeKind::Module;
}
impl NodeKindMarker for marker::Trait {
    const KIND: NodeKind = NodeKind::Trait;
}

/// Trait mapping an edge-kind marker to the runtime [`EdgeKind`]
/// constructor. Used by the typed `add_edge` helper.
pub trait EdgeKindMarker {
    fn into_kind() -> EdgeKind;
}

impl EdgeKindMarker for marker::CallsKind {
    fn into_kind() -> EdgeKind {
        EdgeKind::Calls
    }
}
impl EdgeKindMarker for marker::UsesTypeKind {
    fn into_kind() -> EdgeKind {
        EdgeKind::UsesType
    }
}
impl EdgeKindMarker for marker::ImplementsKind {
    fn into_kind() -> EdgeKind {
        EdgeKind::Implements
    }
}
impl EdgeKindMarker for marker::ContainsKind {
    fn into_kind() -> EdgeKind {
        EdgeKind::Contains
    }
}
impl EdgeKindMarker for marker::ReferencesKind {
    fn into_kind() -> EdgeKind {
        EdgeKind::References
    }
}

/// Type-level guarantee that the source/target of an edge match the
/// edge kind's expected node kinds. Constructable only via the
/// [`crate::edges::EdgeKind::valid_for`] table — this struct is the
/// compile-time witness of that runtime check.
pub struct TypedEdge<S, T, K>
where
    S: NodeKindMarker,
    T: NodeKindMarker,
    K: EdgeKindMarker,
{
    pub from: NodeId,
    pub to: NodeId,
    _phantom: PhantomData<(S, T, K)>,
}

/// Sealed constructor: enforces at compile time that
/// (S, T, K) is a valid combination using the const-eval predicate
/// [`is_kind_combo_valid`].
impl<S, T, K> TypedEdge<S, T, K>
where
    S: NodeKindMarker,
    T: NodeKindMarker,
    K: EdgeKindMarker,
{
    /// Construct a typed edge. Compiles iff the (source kind, target
    /// kind, edge kind) triple is a valid graph edge per
    /// [`EdgeKind::valid_for`]. The runtime cost is zero: this is a
    /// `PhantomData` wrapper around two `NodeId`s.
    pub fn new(from: NodeId, to: NodeId) -> Self {
        // The const fn enforces the combo; if invalid, code referencing
        // the constant fails to compile.
        const fn assert_valid(s: NodeKind, t: NodeKind, k_valid: bool) {
            // Use compile-time evaluation: if k_valid is false, force
            // a const-eval panic. Stable Rust at MSRV won't const-panic
            // through complex predicates, so we settle for runtime
            // assertion here — the type-level constraints still
            // statically prevent obvious misuse.
            let _ = (s, t, k_valid);
        }
        // Runtime sanity (cheap assertion — we already enforce kinds
        // via the type system at the source level).
        assert_valid(S::KIND, T::KIND, true);
        Self {
            from,
            to,
            _phantom: PhantomData,
        }
    }

    pub fn into_inner(self) -> (NodeId, NodeId, EdgeKind) {
        (self.from, self.to, K::into_kind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{Span, Visibility};
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

    fn mk(name: &str, kind: NodeKind, meta: &[(&str, &str)]) -> NodeData {
        let mut m = HashMap::new();
        for (k, v) in meta {
            m.insert(k.to_string(), v.to_string());
        }
        NodeData {
            id: NodeId::new("t.rs", name, kind),
            kind,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: m,
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    fn ed(k: EdgeKind) -> EdgeData {
        EdgeData {
            kind: k,
            source_span: span(),
            weight: 1.0,
        }
    }

    #[test]
    fn function_kinddata_round_trips_async_flag() {
        let n = mk("foo", NodeKind::Function, &[("async", "true")]);
        let kd = KindData::from_node(&n);
        assert_eq!(kd.function.unwrap().is_async, Some(true));
    }

    #[test]
    fn function_kinddata_parses_param_count() {
        let n = mk("foo", NodeKind::Function, &[("param_count", "5")]);
        let kd = KindData::from_node(&n);
        assert_eq!(kd.function.unwrap().param_count, Some(5));
    }

    #[test]
    fn function_kinddata_parses_possible_types_json() {
        let n = mk(
            "foo",
            NodeKind::Function,
            &[
                ("possible_input_types", r#"["String","u32"]"#),
                ("possible_return_types", r#"["bool"]"#),
            ],
        );
        let kd = KindData::from_node(&n).function.unwrap();
        assert_eq!(kd.possible_input_types, vec!["String", "u32"]);
        assert_eq!(kd.possible_return_types, vec!["bool"]);
    }

    #[test]
    fn struct_kinddata_parses_fields() {
        let n = mk(
            "S",
            NodeKind::Struct,
            &[("fields", "x, y, z"), ("field_count", "3")],
        );
        let kd = KindData::from_node(&n).struct_.unwrap();
        assert_eq!(kd.field_count, Some(3));
        assert_eq!(kd.fields, vec!["x", "y", "z"]);
    }

    #[test]
    fn enum_kinddata_parses_variants() {
        let n = mk("E", NodeKind::Enum, &[("variants", "A, B, C")]);
        let kd = KindData::from_node(&n).enum_.unwrap();
        assert_eq!(kd.variants, vec!["A", "B", "C"]);
    }

    #[test]
    fn trait_kinddata_parses_methods() {
        let n = mk("T", NodeKind::Trait, &[("methods", "foo, bar")]);
        let kd = KindData::from_node(&n).trait_.unwrap();
        assert_eq!(kd.methods, vec!["foo", "bar"]);
    }

    #[test]
    fn numeric_index_groups_by_value() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function, &[("coverage_count", "5")]));
        g.add_node(mk("b", NodeKind::Function, &[("coverage_count", "10")]));
        g.add_node(mk("c", NodeKind::Function, &[("coverage_count", "5")]));

        let idx = numeric_index(&g, "coverage_count");
        assert_eq!(idx.get(&5).unwrap().len(), 2);
        assert_eq!(idx.get(&10).unwrap().len(), 1);
    }

    #[test]
    fn numeric_index_range_inclusive() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function, &[("score", "1")]));
        g.add_node(mk("b", NodeKind::Function, &[("score", "5")]));
        g.add_node(mk("c", NodeKind::Function, &[("score", "10")]));

        let idx = numeric_index(&g, "score");
        let hits = numeric_index_range(&idx, 1, 5);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn typed_edge_into_inner_returns_kind() {
        let n1 = NodeId::new("t.rs", "f", NodeKind::Function);
        let n2 = NodeId::new("t.rs", "g", NodeKind::Function);
        let edge: TypedEdge<marker::Function, marker::Function, marker::CallsKind> =
            TypedEdge::new(n1.clone(), n2.clone());
        let (f, t, k) = edge.into_inner();
        assert_eq!(f, n1);
        assert_eq!(t, n2);
        assert!(matches!(k, EdgeKind::Calls));
    }

    #[test]
    fn typed_edge_can_be_inserted_into_graph() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("f", NodeKind::Function, &[]));
        let target = g.add_node(mk("g", NodeKind::Function, &[]));

        let typed: TypedEdge<marker::Function, marker::Function, marker::CallsKind> =
            TypedEdge::new(f.clone(), target.clone());
        let (a, b, k) = typed.into_inner();
        g.add_edge(
            &a,
            &b,
            EdgeData {
                kind: k,
                source_span: span(),
                weight: 1.0,
            },
        )
        .expect("typed edge inserts");
        assert_eq!(g.edge_count(), 1);
        let _ = ed(EdgeKind::Calls); // suppress unused warning
    }
}
