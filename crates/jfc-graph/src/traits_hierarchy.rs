//! Trait- and type-centric analyses over the code graph.
//!
//! Roadmap idiom: "find trait implementation hierarchies, show all call
//! edges that dispatch via this trait, cluster functions by primary
//! types they manipulate."
//!
//! ## Mapping to the existing graph model
//!
//! The crate's [`crate::nodes::NodeKind`] is intentionally narrow — five
//! kinds: `Function`, `Struct`, `Enum`, `Module`, `Trait`. There is **no
//! separate `Impl` node kind**: an `impl Foo` block does not get its own
//! node, only the methods inside it (which become `Function` nodes
//! contained by the `Struct`/`Enum` they're implemented for) and the
//! `Implements` edge from the type to the trait.
//!
//! Concretely the analyses below rely on:
//!
//! - `Implements` edges (`Struct|Enum → Trait`) to build hierarchies.
//! - `Contains` edges from `Trait` nodes to their `Function` members
//!   to detect trait-method calls.
//! - `UsesType` edges from `Function` nodes to `Struct|Enum|Trait` to
//!   determine each function's primary type.
//!
//! Because there's no distinct impl-block node, "calls that dispatch via
//! a trait" are approximated by callees whose containing item is a
//! `Trait` (i.e., the callee is the trait's *declared* method, not the
//! concrete `impl Foo` override). This catches default-method calls and
//! generic trait-bounded calls; the adapter is responsible for whether
//! a static-resolved override gets edged to the trait function or the
//! concrete one.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::Direction;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};

/// All implementors of a trait, grouped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitHierarchy {
    pub trait_id: NodeId,
    /// Direct implementations (`Struct/Enum implements Trait` edges).
    pub direct_impls: BTreeSet<NodeId>,
}

/// Functions clustered by the type they primarily manipulate.
/// "Primary" = most frequently referenced via `UsesType` edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCluster {
    pub primary_type: NodeId,
    pub functions: BTreeSet<NodeId>,
}

/// A `Calls` edge that goes through a trait method (where target's
/// containing impl-or-trait is determined to be a trait method).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitDispatchEdge {
    pub caller: NodeId,
    pub callee: NodeId,
    pub trait_id: NodeId,
}

impl CodeGraph {
    /// Build hierarchies for every trait in the graph.
    ///
    /// O(V + E): one pass over `Trait` nodes to seed empty hierarchies,
    /// then one pass over edges filtering `Implements` to fill them in.
    /// Traits with no implementors still appear (with an empty
    /// `direct_impls` set) so callers can detect "declared but never
    /// implemented" cases.
    pub fn trait_hierarchies(&self) -> Vec<TraitHierarchy> {
        let mut hierarchies: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();

        // Seed every trait with an empty implementor set.
        for trait_node in self.nodes_by_kind(NodeKind::Trait) {
            hierarchies.insert(trait_node.id.clone(), BTreeSet::new());
        }

        // Fill in implementors.
        for edge in self.inner().edge_references() {
            if !matches!(edge.weight().kind, EdgeKind::Implements) {
                continue;
            }
            let Some(source_id) = self.node_id_for(edge.source()) else {
                continue;
            };
            let Some(target_id) = self.node_id_for(edge.target()) else {
                continue;
            };
            hierarchies
                .entry(target_id.clone())
                .or_default()
                .insert(source_id.clone());
        }

        hierarchies
            .into_iter()
            .map(|(trait_id, direct_impls)| TraitHierarchy {
                trait_id,
                direct_impls,
            })
            .collect()
    }

    /// Find all call edges where the callee is (a) a `Function` and
    /// (b) contained-by a `Trait` via a `Contains` edge from a `Trait`
    /// node. These represent calls that may dispatch dynamically
    /// or via trait methods.
    ///
    /// A function reached via `Trait → Contains → Function` is a trait
    /// method (declared on the trait itself). Inherent methods, by
    /// contrast, are `Struct|Enum → Contains → Function` and are
    /// excluded.
    pub fn trait_dispatch_calls(&self) -> Vec<TraitDispatchEdge> {
        let mut out = Vec::new();
        let inner = self.inner();

        for edge in inner.edge_references() {
            if !matches!(edge.weight().kind, EdgeKind::Calls) {
                continue;
            }

            // Look up the trait that contains this callee, if any.
            let callee_idx = edge.target();
            let trait_id_opt = inner
                .edges_directed(callee_idx, Direction::Incoming)
                .filter(|e| matches!(e.weight().kind, EdgeKind::Contains))
                .find_map(|e| {
                    let src = inner.node_weight(e.source())?;
                    if src.kind == NodeKind::Trait {
                        Some(src.id.clone())
                    } else {
                        None
                    }
                });

            let Some(trait_id) = trait_id_opt else {
                continue;
            };

            let Some(caller_id) = self.node_id_for(edge.source()).cloned() else {
                continue;
            };
            let Some(callee_id) = self.node_id_for(callee_idx).cloned() else {
                continue;
            };

            out.push(TraitDispatchEdge {
                caller: caller_id,
                callee: callee_id,
                trait_id,
            });
        }

        out
    }

    /// Cluster functions by primary type. A function's "primary type"
    /// is whichever `Struct`/`Enum`/`Trait` it `UsesType`-edges to most
    /// often. Functions with no `UsesType` edges are skipped. Ties are
    /// broken by `NodeId` ordering for determinism. Returned clusters
    /// are sorted by size (largest first); ties between clusters are
    /// broken by `primary_type` for determinism.
    pub fn cluster_by_primary_type(&self) -> Vec<TypeCluster> {
        let inner = self.inner();
        let mut clusters: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();

        for func in self.nodes_by_kind(NodeKind::Function) {
            let Some(func_idx) = self.resolve(&func.id) else {
                continue;
            };

            // Count UsesType-edge targets for this function.
            let mut counts: BTreeMap<NodeId, usize> = BTreeMap::new();
            for e in inner.edges_directed(func_idx, Direction::Outgoing) {
                if !matches!(e.weight().kind, EdgeKind::UsesType) {
                    continue;
                }
                let Some(target_data) = inner.node_weight(e.target()) else {
                    continue;
                };
                if !matches!(
                    target_data.kind,
                    NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
                ) {
                    continue;
                }
                *counts.entry(target_data.id.clone()).or_insert(0) += 1;
            }

            // Pick the (count, NodeId) max — highest count wins, tie
            // broken by smallest NodeId. `BTreeMap` iteration is sorted
            // by key, so we walk it ourselves to make tie-break order
            // explicit.
            let mut best: Option<(usize, NodeId)> = None;
            for (target_id, count) in &counts {
                let candidate = (*count, target_id.clone());
                best = match best {
                    None => Some(candidate),
                    Some((best_count, ref best_id)) => {
                        if candidate.0 > best_count
                            || (candidate.0 == best_count && &candidate.1 < best_id)
                        {
                            Some(candidate)
                        } else {
                            best
                        }
                    }
                };
            }

            if let Some((_, primary)) = best {
                clusters.entry(primary).or_default().insert(func.id.clone());
            }
        }

        let mut out: Vec<TypeCluster> = clusters
            .into_iter()
            .map(|(primary_type, functions)| TypeCluster {
                primary_type,
                functions,
            })
            .collect();

        // Largest cluster first, ties broken by primary_type for determinism.
        out.sort_by(|a, b| {
            b.functions
                .len()
                .cmp(&a.functions.len())
                .then_with(|| a.primary_type.cmp(&b.primary_type))
        });

        out
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, Span, Visibility};

    fn span() -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 2,
            end_col: 0,
            byte_range: 0..10,
        }
    }

    fn node(name: &str, kind: NodeKind) -> NodeData {
        let qn = format!("crate::{name}");
        NodeData {
            id: NodeId::new("src/lib.rs", &qn, kind),
            kind,
            name: name.to_string(),
            qualified_name: qn,
            file_path: PathBuf::from("src/lib.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn edge(kind: EdgeKind) -> EdgeData {
        EdgeData {
            kind,
            source_span: span(),
            weight: 1.0,
        }
    }

    // ─── trait_hierarchies ──────────────────────────────────────────

    #[test]
    fn trait_hierarchy_groups_implementors_normal() {
        let mut g = CodeGraph::new();
        let t = node("MyTrait", NodeKind::Trait);
        let s1 = node("Foo", NodeKind::Struct);
        let s2 = node("Bar", NodeKind::Struct);
        let t_id = g.add_node(t);
        let s1_id = g.add_node(s1);
        let s2_id = g.add_node(s2);

        g.add_edge(&s1_id, &t_id, edge(EdgeKind::Implements))
            .unwrap();
        g.add_edge(&s2_id, &t_id, edge(EdgeKind::Implements))
            .unwrap();

        let hierarchies = g.trait_hierarchies();
        let h = hierarchies
            .iter()
            .find(|h| h.trait_id == t_id)
            .expect("trait should appear in hierarchies");
        assert_eq!(h.direct_impls.len(), 2);
        assert!(h.direct_impls.contains(&s1_id));
        assert!(h.direct_impls.contains(&s2_id));
    }

    #[test]
    fn trait_hierarchy_empty_when_trait_has_no_impls_robust() {
        let mut g = CodeGraph::new();
        let t = node("Lonely", NodeKind::Trait);
        let t_id = g.add_node(t);

        let hierarchies = g.trait_hierarchies();
        let h = hierarchies
            .iter()
            .find(|h| h.trait_id == t_id)
            .expect("orphan trait still appears");
        assert!(h.direct_impls.is_empty());
    }

    // ─── trait_dispatch_calls ────────────────────────────────────────

    #[test]
    fn trait_dispatch_calls_finds_trait_methods_normal() {
        // Trait T contains method `tm`. Function `caller` calls `tm`.
        let mut g = CodeGraph::new();
        let t = node("T", NodeKind::Trait);
        let tm = node("T_tm", NodeKind::Function);
        let caller = node("caller", NodeKind::Function);
        let t_id = g.add_node(t);
        let tm_id = g.add_node(tm);
        let caller_id = g.add_node(caller);

        g.add_edge(&t_id, &tm_id, edge(EdgeKind::Contains)).unwrap();
        g.add_edge(&caller_id, &tm_id, edge(EdgeKind::Calls))
            .unwrap();

        let dispatched = g.trait_dispatch_calls();
        assert_eq!(dispatched.len(), 1);
        assert_eq!(dispatched[0].caller, caller_id);
        assert_eq!(dispatched[0].callee, tm_id);
        assert_eq!(dispatched[0].trait_id, t_id);
    }

    #[test]
    fn trait_dispatch_calls_excludes_inherent_methods_robust() {
        // Struct S contains inherent method `m`. Function `caller` calls `m`.
        // Should NOT be in the dispatched-calls list (Contains is from a
        // Struct, not a Trait).
        let mut g = CodeGraph::new();
        let s = node("S", NodeKind::Struct);
        let m = node("S_m", NodeKind::Function);
        let caller = node("caller", NodeKind::Function);
        let s_id = g.add_node(s);
        let m_id = g.add_node(m);
        let caller_id = g.add_node(caller);

        g.add_edge(&s_id, &m_id, edge(EdgeKind::Contains)).unwrap();
        g.add_edge(&caller_id, &m_id, edge(EdgeKind::Calls))
            .unwrap();

        let dispatched = g.trait_dispatch_calls();
        assert!(
            dispatched.is_empty(),
            "inherent-method calls must not appear in trait_dispatch_calls"
        );
    }

    // ─── cluster_by_primary_type ─────────────────────────────────────

    #[test]
    fn cluster_by_primary_type_picks_most_used_normal() {
        // f uses Foo 3x and Bar 1x → primary = Foo.
        let mut g = CodeGraph::new();
        let f = node("f", NodeKind::Function);
        let foo = node("Foo", NodeKind::Struct);
        let bar = node("Bar", NodeKind::Struct);
        let f_id = g.add_node(f);
        let foo_id = g.add_node(foo);
        let bar_id = g.add_node(bar);

        // Three UsesType edges to Foo, one to Bar. (Multi-edges between
        // the same pair are allowed by petgraph; they count as separate
        // UsesType references.)
        for _ in 0..3 {
            g.add_edge(&f_id, &foo_id, edge(EdgeKind::UsesType))
                .unwrap();
        }
        g.add_edge(&f_id, &bar_id, edge(EdgeKind::UsesType))
            .unwrap();

        let clusters = g.cluster_by_primary_type();
        let fc = clusters
            .iter()
            .find(|c| c.functions.contains(&f_id))
            .expect("f should appear in some cluster");
        assert_eq!(fc.primary_type, foo_id);
        assert!(!clusters.iter().any(|c| c.primary_type == bar_id));
    }

    #[test]
    fn cluster_by_primary_type_breaks_ties_by_node_id_robust() {
        // f uses Foo 1x and Bar 1x → tie, broken by smallest NodeId.
        let mut g = CodeGraph::new();
        let f = node("f", NodeKind::Function);
        let foo = node("Foo", NodeKind::Struct);
        let bar = node("Bar", NodeKind::Struct);
        let f_id = g.add_node(f);
        let foo_id = g.add_node(foo);
        let bar_id = g.add_node(bar);

        g.add_edge(&f_id, &foo_id, edge(EdgeKind::UsesType))
            .unwrap();
        g.add_edge(&f_id, &bar_id, edge(EdgeKind::UsesType))
            .unwrap();

        let clusters = g.cluster_by_primary_type();
        let fc = clusters
            .iter()
            .find(|c| c.functions.contains(&f_id))
            .expect("f should appear in some cluster");

        // Tie-break: smaller NodeId wins.
        let expected = std::cmp::min(foo_id.clone(), bar_id.clone());
        assert_eq!(
            fc.primary_type, expected,
            "tie should be broken by smallest NodeId for determinism"
        );
    }
}
