//! Fast-lookup indices over [`crate::graph::CodeGraph`].
//!
//! `CodeGraph`'s primary storage is a `StableDiGraph<NodeData, EdgeData>` plus
//! a `NodeId → NodeIndex` map. Anything that asks "give me every node where
//! kind=`Function` and name=`build`" or "every node in module `crate::ui`"
//! would otherwise walk every node — `O(n)` per query, multiplied by every
//! DSL traversal that filters by kind/name/module. As graphs grow into the
//! 10k+ node range that linear scan dominates traversal cost.
//!
//! [`Indices`] keeps three sorted maps that answer those questions in
//! `O(log n + k)` where `k` is the number of matches:
//!
//! - **`by_kind_name`**: `(NodeKind, String)` → `Vec<NodeId>`. The list
//!   form is intentional — overloaded names (e.g. trait-impl methods sharing
//!   a free function's identifier) are common, and we never want to silently
//!   drop one.
//! - **`by_metadata_key`**: metadata key → `BTreeSet<NodeId>`. Powers queries
//!   like "every node tagged `is_pub=true`" without forcing each filter to
//!   walk the graph and look at `metadata.contains_key`.
//! - **`by_module_path`**: exact module path → `BTreeSet<NodeId>`. Stored as
//!   the *exact* path (the qualified name minus its terminal segment); prefix
//!   queries are answered by a `BTreeMap::range` scan, which gives
//!   `O(log n + k)` for "all nodes under `crate::ui::*`".
//!
//! ## Invariant — index ↔ graph consistency
//!
//! The indices are derived state. Every mutation that touches the underlying
//! petgraph storage MUST also update each of the three indices, or queries
//! will silently desynchronise. Concretely:
//!
//! - [`crate::graph::CodeGraph::add_node`]: insert into all three indices.
//!   When `add_node` overwrites an existing slot (same `NodeId`, different
//!   payload), the old entries must be removed before re-inserting.
//! - [`crate::graph::CodeGraph::remove_node`]: remove from all three indices.
//! - Any future `update_node` API must do a remove-then-insert dance (the
//!   kind/name/metadata of an updated node may differ from the previous
//!   version, so the *old* index keys are no longer correct).
//!
//! The single source of truth is [`Indices::rebuild_from_graph`] — used at
//! `CodeGraph::new` and as a recovery hatch if a future deserialiser ever
//! bypasses `add_node`. If you ever suspect drift, calling that function
//! restores consistency in `O(n)`.
//!
//! `Indices` itself is `pub(crate)` — the only public surface is the methods
//! on `CodeGraph` that wrap it. Downstream crates should never see the
//! type itself.

use std::collections::{BTreeMap, BTreeSet};

use petgraph::stable_graph::StableDiGraph;

use crate::edges::EdgeData;
use crate::nodes::{NodeData, NodeId, NodeKind};

/// Derived indices over [`crate::graph::CodeGraph`]. See module docs for the
/// consistency invariant — every mutation of the underlying graph must flow
/// through one of the maintenance methods on this type.
#[derive(Default)]
pub(crate) struct Indices {
    /// `(NodeKind, name)` → every `NodeId` with that exact kind+name pair.
    /// `Vec` (not `Set`) because order is not meaningful but duplicates can
    /// never occur — `NodeId` includes the file path, so two Function nodes
    /// named `build` in different files are distinct `NodeId`s.
    by_kind_name: BTreeMap<(NodeKind, String), Vec<NodeId>>,

    /// metadata key → `NodeId`s with that key present in their metadata map.
    /// We index the *key*, not the value; "all nodes that record a `decl_id`"
    /// is the common shape of metadata queries.
    by_metadata_key: BTreeMap<String, BTreeSet<NodeId>>,

    /// Exact module path → `NodeId`s whose `qualified_name` lies in that
    /// module. The path is the qualified name minus its final segment
    /// (`crate::ui::App` lives in module `crate::ui`). Stored exact rather
    /// than prefix-decomposed so memory is `O(n)`; prefix queries use
    /// `BTreeMap::range`.
    by_module_path: BTreeMap<String, BTreeSet<NodeId>>,
}

impl Indices {
    /// Insert a node's contributions into all three indices. Idempotent for
    /// repeated inserts of the *same* node — but if the caller wants to
    /// overwrite (kind/name/metadata changed), they must call
    /// [`Self::remove_node`] first.
    pub(crate) fn insert_node(&mut self, data: &NodeData) {
        self.by_kind_name
            .entry((data.kind, data.name.clone()))
            .or_default()
            .push(data.id.clone());

        for key in data.metadata.keys() {
            self.by_metadata_key
                .entry(key.clone())
                .or_default()
                .insert(data.id.clone());
        }

        let module = module_path_of(&data.qualified_name);
        self.by_module_path
            .entry(module.to_string())
            .or_default()
            .insert(data.id.clone());
    }

    /// Drop every reference to a node from all three indices. The caller
    /// passes the `NodeData` that's about to disappear (or just disappeared);
    /// we need its kind/name/metadata/qualified_name to know which buckets
    /// to clean.
    pub(crate) fn remove_node(&mut self, data: &NodeData) {
        if let Some(bucket) = self.by_kind_name.get_mut(&(data.kind, data.name.clone())) {
            bucket.retain(|id| id != &data.id);
            if bucket.is_empty() {
                self.by_kind_name.remove(&(data.kind, data.name.clone()));
            }
        }

        for key in data.metadata.keys() {
            if let Some(bucket) = self.by_metadata_key.get_mut(key) {
                bucket.remove(&data.id);
                if bucket.is_empty() {
                    self.by_metadata_key.remove(key);
                }
            }
        }

        let module = module_path_of(&data.qualified_name);
        if let Some(bucket) = self.by_module_path.get_mut(module) {
            bucket.remove(&data.id);
            if bucket.is_empty() {
                self.by_module_path.remove(module);
            }
        }
    }

    /// Drop every entry and rebuild from the graph's current contents. Used
    /// by `CodeGraph::new` (cheap — empty graph) and as the recovery hatch
    /// if any future code path inserts nodes without going through the
    /// indexed API.
    pub(crate) fn rebuild_from_graph(&mut self, graph: &StableDiGraph<NodeData, EdgeData>) {
        self.by_kind_name.clear();
        self.by_metadata_key.clear();
        self.by_module_path.clear();

        for data in graph.node_weights() {
            self.insert_node(data);
        }
    }

    /// Lookup by `(kind, name)`. Returns the empty slice if no such pair
    /// exists — the empty-vec branch never allocates.
    pub(crate) fn nodes_by_kind_name(&self, kind: NodeKind, name: &str) -> &[NodeId] {
        // Avoid allocating a `String` for the lookup key. `BTreeMap::get`
        // accepts any `Q: Ord` that the stored key borrows as, but tuple
        // borrows are awkward, so the cheap path is one allocation per
        // query. Names are short (identifiers), so the cost is bounded.
        match self.by_kind_name.get(&(kind, name.to_string())) {
            Some(v) => v.as_slice(),
            None => &[],
        }
    }

    /// Iterator over node ids that record metadata `key`.
    pub(crate) fn nodes_with_metadata_key(
        &self,
        key: &str,
    ) -> impl Iterator<Item = &NodeId> + '_ {
        self.by_metadata_key
            .get(key)
            .into_iter()
            .flat_map(|set| set.iter())
    }

    /// All nodes whose module path matches `prefix` exactly OR is a child
    /// module of `prefix`. Implemented as a `BTreeMap::range` scan so the
    /// cost is `O(log n + k)` where `k` is the number of matching modules
    /// (each itself a small bucket).
    pub(crate) fn nodes_in_module(&self, prefix: &str) -> Vec<NodeId> {
        let mut out = Vec::new();

        // The matching set of *module paths* is:
        //   { p : p == prefix } ∪ { p : p starts with prefix + "::" }
        //
        // We get those via two range scans rather than iterating every
        // module path. The submodule range is `[prefix::, prefix:;)` —
        // `:;` is just `::` with the second `:` bumped by one, the
        // standard trick for half-open lex range queries.
        if let Some(bucket) = self.by_module_path.get(prefix) {
            out.extend(bucket.iter().cloned());
        }

        let lower = format!("{prefix}::");
        // Bumping the last byte of `::` (`:` = 0x3A) to `;` (0x3B) gives the
        // smallest string strictly greater than every `prefix::*` string.
        let mut upper_bytes = lower.as_bytes().to_vec();
        if let Some(last) = upper_bytes.last_mut() {
            *last += 1;
        }
        let upper = String::from_utf8(upper_bytes).expect("ASCII bump preserves UTF-8");

        for (_, bucket) in self.by_module_path.range(lower..upper) {
            out.extend(bucket.iter().cloned());
        }
        out
    }
}

/// Extract the module path from a qualified name. `crate::ui::App` →
/// `crate::ui`; `App` (top-level, no `::`) → `""`. The empty-string bucket
/// is meaningful — it groups every top-level item, so `nodes_in_module("")`
/// would conceptually return all nodes.
fn module_path_of(qualified_name: &str) -> &str {
    match qualified_name.rfind("::") {
        Some(idx) => &qualified_name[..idx],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_path_of_strips_terminal_segment_normal() {
        assert_eq!(module_path_of("crate::ui::App"), "crate::ui");
        assert_eq!(module_path_of("crate::App"), "crate");
        assert_eq!(module_path_of("App"), "");
    }
}
