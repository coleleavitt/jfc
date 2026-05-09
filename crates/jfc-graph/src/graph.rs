//! Core graph data structure and operations.
//!
//! Uses `StableGraph` instead of `DiGraph` so that `NodeIndex` values remain
//! stable across removals — no more swap-back fixup.
//!
//! ## NodeIndex stability invariant
//!
//! `petgraph::stable_graph::NodeIndex` is an **internal** identifier used to
//! address slots inside the underlying `StableDiGraph`. It is NEVER exposed
//! through the public API of this crate.
//!
//! The user-facing identifier is [`crate::nodes::NodeId`] — content-addressed,
//! stable for the lifetime of a node, and durable across the graph's
//! insertion/removal sequence. Public APIs accept and return `NodeId` only.
//!
//! ### Why this matters
//!
//! `StableDiGraph` keeps an existing index live across removals (no
//! swap-back), but a **removed** slot may be **re-used** by the next call to
//! `add_node`. A caller that cached a `NodeIndex` across a remove/add cycle
//! could silently address a different node. By contrast, `NodeId` is derived
//! from node-identifying content (file path + qualified name + kind), so the
//! same `NodeId` always refers to the same logical entity.
//!
//! ### Enforcement
//!
//! - `index_map`, `inner`, `resolve`, `node_id_for`, and `node_indices` are
//!   all `pub(crate)` — visible to sibling modules (`analysis`, `traversal`,
//!   `formatting`, …) but invisible to downstream crates.
//! - Any new `pub fn` in this crate that returns or accepts a `NodeIndex`
//!   should be considered a regression. Use `NodeId` at the boundary; do the
//!   `NodeId → NodeIndex` translation inside the function via `resolve()`.
//! - See `tests/node_id_stability.rs` for a regression test demonstrating the
//!   slot-reuse hazard and the `NodeId`-based contract that prevents it.

use std::collections::HashMap;
use std::path::Path;

use petgraph::Direction;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::EdgeRef;
use thiserror::Error;

use crate::adapter::LanguageAdapter;
use crate::edges::{EdgeData, EdgeKind};
use crate::index::Indices;
use crate::nodes::{NodeData, NodeId, NodeKind};
use crate::persistence::GraphEvent;

/// Edge-invariant violations detected at insertion time.
///
/// Each `EdgeKind` has implicit constraints on the [`NodeKind`] of its source
/// and target (e.g. `Calls` requires Function→Function, `Implements` requires
/// Struct/Enum→Trait). Inserting a tuple that violates these constraints
/// silently corrupts downstream traversal — `Contains`-walks expecting
/// containment semantics, `Calls`-walks expecting reachability, etc. — so we
/// reject at the boundary instead of patching later.
#[derive(Debug, Error)]
pub enum EdgeInvariantError {
    #[error(
        "edge {edge:?} requires source kind matching its constraint, got {got:?} for node {id:?}"
    )]
    WrongSourceKind {
        edge: EdgeKind,
        got: NodeKind,
        id: NodeId,
    },

    #[error(
        "edge {edge:?} requires target kind matching its constraint, got {got:?} for node {id:?}"
    )]
    WrongTargetKind {
        edge: EdgeKind,
        got: NodeKind,
        id: NodeId,
    },

    /// Edge weight was NaN or +/-infinity. Downstream shortest-path
    /// algorithms (`k_shortest_paths`, `dijkstra`) panic on non-finite
    /// `f32` weights, so we reject them at insertion time.
    #[error("edge {edge:?} has non-finite weight {weight} (must be finite f32)")]
    NonFiniteWeight { edge: EdgeKind, weight: f32 },
}

/// Errors from graph operations.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("node not found: {0:?}")]
    NodeNotFound(NodeId),

    #[error("edge already exists between {from:?} and {to:?}")]
    EdgeExists { from: NodeId, to: NodeId },

    #[error(transparent)]
    InvariantViolation(#[from] EdgeInvariantError),
}

/// The core code graph — wraps petgraph's `StableDiGraph` with typed nodes and O(1) ID lookup.
///
/// `StableDiGraph` keeps indices stable across removals, eliminating the swap-back
/// fixup that was necessary with plain `DiGraph`.
///
/// ## Index consistency invariant
///
/// Beyond the petgraph store and the `NodeId → NodeIndex` map, `CodeGraph`
/// also maintains an [`Indices`] table for fast `(kind, name)`,
/// metadata-key, and module-prefix queries. **Every mutation that touches
/// the underlying graph must also update `indices`.** The methods on
/// `CodeGraph` enforce this — if you ever add a new mutation API, route it
/// through one of the `insert_node` / `remove_node` helpers on `Indices`,
/// or call [`Self::rebuild_indices`] before exposing a query result.
pub struct CodeGraph {
    /// The wrapped petgraph instance. Module-private — see the
    /// [crate-level invariant](self). Sibling modules go through
    /// [`Self::inner`] (`pub(crate)`) instead of accessing this field.
    graph: StableDiGraph<NodeData, EdgeData>,
    /// `NodeId → NodeIndex` translation table. Module-private to lock down
    /// the slot identifier — see the [crate-level invariant](self). All
    /// translations route through [`Self::resolve`] / [`Self::node_id_for`].
    index_map: HashMap<NodeId, NodeIndex>,
    /// Derived fast-lookup indices. Module-private; kept in lockstep with
    /// `graph` by every mutation method. See [`Indices`] docs for the
    /// invariant.
    indices: Indices,
    /// Monotonically-increasing graph revision. Bumped before every mutation
    /// (`add_node`, `add_edge`, `remove_node`) and stamped onto the affected
    /// node(s)' `birth_revision` / `last_modified_revision`. Surfaced via
    /// [`Self::current_revision`] and used by [`Self::nodes_changed_since`].
    ///
    /// Starts at `0`; the first mutation bumps to `1`. The `0` value is
    /// reserved for "pre-history" — a node with `birth_revision == 0`
    /// either came from a pre-revision-tracking serialized graph or was
    /// constructed as a `NodeData` literal but never inserted via
    /// `add_node`.
    revision: u64,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: StableDiGraph::new(),
            index_map: HashMap::new(),
            indices: Indices::default(),
            revision: 0,
        }
    }

    /// Current graph revision. Bumped on every mutation
    /// (`add_node` / `add_edge` / `remove_node`). Pair with
    /// [`Self::nodes_changed_since`] to query the temporal delta.
    pub fn current_revision(&self) -> u64 {
        self.revision
    }

    /// Increment and return the new revision. Every mutation method calls
    /// this *before* stamping the affected node(s) — that way the first
    /// mutation produces revision `1`, leaving `0` reserved for "pre-history"
    /// (see [`crate::nodes::NodeData`] docs for the wire-format meaning).
    fn bump_revision(&mut self) -> u64 {
        self.revision = self.revision.saturating_add(1);
        self.revision
    }

    /// Stamp `last_modified_revision` on a node by `NodeId`.
    /// No-op if the node is absent. Used by edge-mutation paths so both
    /// endpoints learn that something changed about them.
    fn touch_node(&mut self, id: &NodeId, rev: u64) {
        if let Some(&idx) = self.index_map.get(id) {
            self.graph[idx].last_modified_revision = rev;
        }
    }

    /// Direct read access to the inner petgraph. Enables all petgraph
    /// algorithms (SCC, dominators, toposort, page_rank, etc.) to operate
    /// without copying.
    ///
    /// `pub(crate)`: this exposes `NodeIndex` indirectly via the petgraph API,
    /// which is the kind of leak the [crate-level invariant](self) forbids in
    /// the public API. Sibling modules (`analysis`, `traversal`, `formatting`)
    /// need it for petgraph's bundled algorithms; downstream crates do not.
    pub(crate) fn inner(&self) -> &StableDiGraph<NodeData, EdgeData> {
        &self.graph
    }

    /// Resolve a `NodeId` to a petgraph `NodeIndex`.
    ///
    /// `pub(crate)`: returns the internal slot identifier — see the
    /// [crate-level invariant](self). Always perform the translation at the
    /// public-API boundary; never let the `NodeIndex` escape.
    pub(crate) fn resolve(&self, id: &NodeId) -> Option<NodeIndex> {
        self.index_map.get(id).copied()
    }

    /// Reverse lookup: `NodeIndex` → `NodeId`.
    ///
    /// `pub(crate)`: takes the internal slot identifier — see the
    /// [crate-level invariant](self).
    pub(crate) fn node_id_for(&self, idx: NodeIndex) -> Option<&NodeId> {
        self.graph.node_weight(idx).map(|n| &n.id)
    }

    /// Add a node. Returns the NodeId. If node with same ID exists, updates it.
    ///
    /// Also updates the fast-lookup indices. On the *overwrite* path
    /// (existing `NodeId`, possibly different name/kind/metadata/module
    /// path) the old index entries must be evicted before the new ones go
    /// in — otherwise stale `(kind, name)` or metadata-key buckets would
    /// continue to point at a node that no longer matches the query.
    ///
    /// Stamps revision metadata: bumps [`Self::current_revision`] and writes
    /// the new revision into `last_modified_revision` (and `birth_revision`
    /// on the *insert* path). Any value the caller put in those two fields
    /// of the incoming [`NodeData`] is overwritten — the graph is the
    /// source of truth for revision identity.
    pub fn add_node(&mut self, data: NodeData) -> NodeId {
        let id = data.id.clone();
        let rev = self.bump_revision();
        let mut data = data;
        data.last_modified_revision = rev;
        if let Some(&idx) = self.index_map.get(&id) {
            // Overwrite path: drop the *previous* node's contributions to
            // the indices first. We snapshot the old data before swapping
            // the petgraph slot so `Indices::remove_node` sees the keys
            // that were originally inserted. Preserve the original
            // `birth_revision` (the node was born when it first appeared,
            // not at this overwrite).
            let old = self.graph[idx].clone();
            data.birth_revision = old.birth_revision;
            self.indices.remove_node(&old);
            self.graph[idx] = data.clone();
            self.indices.insert_node(&data);
        } else {
            // Insert path: this revision IS the birth revision.
            data.birth_revision = rev;
            let idx = self.graph.add_node(data.clone());
            self.index_map.insert(id.clone(), idx);
            self.indices.insert_node(&data);
        }
        id
    }

    /// Add an edge between two nodes.
    ///
    /// Returns [`GraphError::NodeNotFound`] if either endpoint is absent and
    /// [`GraphError::InvariantViolation`] if the edge kind would violate the
    /// per-kind source/target [`NodeKind`] constraints encoded in
    /// [`EdgeKind::valid_for`]. The validation runs *before* the edge is
    /// inserted, so a rejected edge leaves the graph untouched.
    ///
    /// `UnresolvedCall` and `ExternalCall` edges relax the target-kind check
    /// since they may point at placeholder nodes that haven't been resolved
    /// yet — only the source-kind constraint is enforced for those.
    pub fn add_edge(
        &mut self,
        from: &NodeId,
        to: &NodeId,
        data: EdgeData,
    ) -> Result<(), GraphError> {
        let &from_idx = self
            .index_map
            .get(from)
            .ok_or_else(|| GraphError::NodeNotFound(from.clone()))?;
        let &to_idx = self
            .index_map
            .get(to)
            .ok_or_else(|| GraphError::NodeNotFound(to.clone()))?;

        let from_kind = self.graph[from_idx].kind;
        let to_kind = self.graph[to_idx].kind;

        if !data.kind.valid_for(from_kind, to_kind) {
            // Determine whether the source or target is the offender so we
            // can emit a precise error. For edges whose source check passes,
            // the violation must be on the target side.
            let source_only_kind = matches!(
                data.kind,
                EdgeKind::UnresolvedCall(_) | EdgeKind::ExternalCall(_, _)
            );

            // Build a "source-only" probe: if the source is wrong, fail on it;
            // otherwise the target must be the offender.
            let source_ok = match &data.kind {
                EdgeKind::Calls => from_kind == NodeKind::Function,
                EdgeKind::UnresolvedCall(_) => from_kind == NodeKind::Function,
                EdgeKind::UsesType => from_kind == NodeKind::Function,
                EdgeKind::References => true,
                EdgeKind::Contains => matches!(
                    from_kind,
                    NodeKind::Module | NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
                ),
                EdgeKind::Implements => {
                    matches!(from_kind, NodeKind::Struct | NodeKind::Enum)
                }
                EdgeKind::ExternalCall(_, _) => from_kind == NodeKind::Function,
            };

            if !source_ok {
                return Err(GraphError::InvariantViolation(
                    EdgeInvariantError::WrongSourceKind {
                        edge: data.kind,
                        got: from_kind,
                        id: from.clone(),
                    },
                ));
            }

            // Source is fine — must be the target. (For source-only kinds we
            // would have already returned valid_for == true.)
            debug_assert!(!source_only_kind, "source-only edges accept any target");
            return Err(GraphError::InvariantViolation(
                EdgeInvariantError::WrongTargetKind {
                    edge: data.kind,
                    got: to_kind,
                    id: to.clone(),
                },
            ));
        }

        // Reject NaN / infinite weights at insertion. petgraph's
        // `k_shortest_paths` / `dijkstra` panic on non-finite floats; catch
        // the bad input here so the panic site never sees it.
        if !data.weight.is_finite() {
            return Err(GraphError::InvariantViolation(
                EdgeInvariantError::NonFiniteWeight {
                    edge: data.kind,
                    weight: data.weight,
                },
            ));
        }

        self.graph.add_edge(from_idx, to_idx, data);
        // Edge insertion changes the connectivity of both endpoints, so
        // both have logically been "modified" at this revision. Bump the
        // graph revision once and stamp it onto both nodes.
        let rev = self.bump_revision();
        self.graph[from_idx].last_modified_revision = rev;
        self.graph[to_idx].last_modified_revision = rev;
        Ok(())
    }

    /// Get node data by ID.
    pub fn get_node(&self, id: &NodeId) -> Option<&NodeData> {
        self.index_map.get(id).map(|&idx| &self.graph[idx])
    }

    /// Get all outgoing edges from a node: (target_id, edge_data)
    pub fn get_edges_from(&self, id: &NodeId) -> Vec<(&NodeId, &EdgeData)> {
        let Some(&idx) = self.index_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|edge| {
                let target_data = &self.graph[edge.target()];
                (&target_data.id, edge.weight())
            })
            .collect()
    }

    /// Get all incoming edges to a node: (source_id, edge_data)
    pub fn get_edges_to(&self, id: &NodeId) -> Vec<(&NodeId, &EdgeData)> {
        let Some(&idx) = self.index_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|edge| {
                let source_data = &self.graph[edge.source()];
                (&source_data.id, edge.weight())
            })
            .collect()
    }

    /// Remove a node and all its connected edges.
    ///
    /// With `StableDiGraph`, indices remain stable after removal — no swap-back fixup needed.
    /// Also evicts the node from the fast-lookup [`Indices`] table.
    ///
    /// Bumps [`Self::current_revision`] and stamps every *surviving* neighbor
    /// of the removed node with the new revision — those neighbors lost an
    /// edge, so their `last_modified_revision` should reflect that.
    pub fn remove_node(&mut self, id: &NodeId) -> Option<NodeData> {
        let idx = self.index_map.remove(id)?;
        // Snapshot every neighbor (both directions) BEFORE the petgraph
        // removal — the edges are gone afterwards and we wouldn't be able
        // to enumerate them. Use `NodeId` rather than `NodeIndex` so the
        // touch_node lookup stays slot-stable across the removal.
        let neighbors: Vec<NodeId> = self
            .graph
            .neighbors_directed(idx, Direction::Outgoing)
            .chain(self.graph.neighbors_directed(idx, Direction::Incoming))
            .filter_map(|n| self.graph.node_weight(n).map(|d| d.id.clone()))
            .collect();
        let removed = self.graph.remove_node(idx)?;
        self.indices.remove_node(&removed);
        let rev = self.bump_revision();
        for n in &neighbors {
            self.touch_node(n, rev);
        }
        Some(removed)
    }

    /// Find nodes by kind.
    pub fn nodes_by_kind(&self, kind: NodeKind) -> Vec<&NodeData> {
        self.graph
            .node_weights()
            .filter(|data| data.kind == kind)
            .collect()
    }

    /// Find nodes by name (substring match, case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Vec<&NodeData> {
        let lower = name.to_lowercase();
        self.graph
            .node_weights()
            .filter(|data| data.name.to_lowercase().contains(&lower))
            .collect()
    }

    /// Lookup nodes by exact `(kind, name)`. Returns `&[NodeId]` rather than
    /// `&[NodeData]` because the indexed lookup can short-circuit at the
    /// `(kind, name)` bucket without touching the petgraph store; downstream
    /// callers can decide whether they actually need the full payload.
    ///
    /// Cost: `O(log n)` for the map lookup, plus the cost of borrowing the
    /// pre-built bucket. Use this in preference to filtering [`find_by_name`]
    /// when you know both the kind and the exact name — DSL `fn` / `type`
    /// selectors are the canonical case.
    pub fn nodes_by_kind_name(&self, kind: NodeKind, name: &str) -> &[NodeId] {
        self.indices.nodes_by_kind_name(kind, name)
    }

    /// Iterate every `NodeId` whose metadata map records the given key.
    /// Powers queries like "all nodes that carry an `is_pub` annotation"
    /// without scanning the graph. The iterator yields nothing if no node
    /// has the key, so callers can use it directly in `for` loops.
    pub fn nodes_with_metadata_key(&self, key: &str) -> impl Iterator<Item = &NodeId> + '_ {
        self.indices.nodes_with_metadata_key(key)
    }

    /// All nodes whose qualified-name module path matches `module_prefix`
    /// exactly OR is a child module of it. `nodes_in_module("crate::ui")`
    /// returns nodes in `crate::ui` *and* every `crate::ui::*` descendant.
    ///
    /// Cost: `O(log n + k)` where `k` is the number of matching modules,
    /// thanks to a `BTreeMap::range` scan over the indexed module-path map.
    pub fn nodes_in_module(&self, module_prefix: &str) -> Vec<NodeId> {
        self.indices.nodes_in_module(module_prefix)
    }

    /// Recompute the fast-lookup indices from the current node store.
    ///
    /// `add_node` / `remove_node` keep the indices in lockstep automatically,
    /// so this is only needed if a future code path inserts nodes by some
    /// other route (e.g. a deserialiser that pokes `pub(crate)` internals
    /// directly). Cost: `O(n)` plus per-node index work.
    pub fn rebuild_indices(&mut self) {
        self.indices.rebuild_from_graph(&self.graph);
    }

    /// Mutate a node's metadata in-place without a full `add_node` round-trip.
    ///
    /// The closure receives `&mut HashMap<String, String>` — the node's
    /// metadata map. The node's `last_modified_revision` is bumped to the
    /// current revision after the closure runs so that `since N` queries
    /// see the change. The fast-lookup indices are updated for any keys
    /// the closure added or removed.
    ///
    /// Returns `false` if the node doesn't exist.
    pub(crate) fn update_node_metadata<F>(&mut self, id: &NodeId, f: F) -> bool
    where
        F: FnOnce(&mut std::collections::HashMap<String, String>),
    {
        let Some(&idx) = self.index_map.get(id) else {
            return false;
        };
        // Snapshot old metadata keys for index diff.
        let old_keys: Vec<String> = self.graph[idx].metadata.keys().cloned().collect();
        f(&mut self.graph[idx].metadata);
        let rev = self.bump_revision();
        self.graph[idx].last_modified_revision = rev;
        // Rebuild index entries for this node (cheap: one node).
        self.indices.remove_node(&self.graph[idx]);
        self.indices.insert_node(&self.graph[idx]);
        let _ = old_keys; // suppress unused warning
        true
    }

    /// Total node count.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Total edge count.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Get all node IDs.
    pub fn all_node_ids(&self) -> Vec<&NodeId> {
        self.index_map.keys().collect()
    }

    /// Check if a node exists.
    pub fn contains_node(&self, id: &NodeId) -> bool {
        self.index_map.contains_key(id)
    }

    /// Every node whose `last_modified_revision >= since_rev`. Pair with
    /// [`Self::current_revision`] taken before a batch of mutations to
    /// answer "what changed in this batch?". Cost: `O(n)` — walks the
    /// node store. No allocations beyond the result `Vec`.
    ///
    /// The comparison is `>=` because `since_rev` is inclusive: passing the
    /// revision from *before* a mutation captures every node that mutation
    /// touched.
    pub fn nodes_changed_since(&self, since_rev: u64) -> Vec<NodeId> {
        self.graph
            .node_weights()
            .filter(|n| n.last_modified_revision >= since_rev)
            .map(|n| n.id.clone())
            .collect()
    }

    /// Every node within `depth` undirected hops of any node modified at or
    /// after `since_rev`. The seed set comes from
    /// [`Self::nodes_changed_since`]; the expansion is a simple breadth-first
    /// walk that ignores edge direction (we want "what's near the change",
    /// not "what's reachable from the change").
    ///
    /// `depth == 0` returns just the changed-since seed set (equivalent to
    /// [`Self::nodes_changed_since`]). The result is deduplicated.
    pub fn nodes_changed_within_depth(&self, since_rev: u64, depth: usize) -> Vec<NodeId> {
        use std::collections::{HashSet, VecDeque};

        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut frontier: VecDeque<(NodeId, usize)> = VecDeque::new();

        for id in self.nodes_changed_since(since_rev) {
            if visited.insert(id.clone()) {
                frontier.push_back((id, 0));
            }
        }

        while let Some((id, d)) = frontier.pop_front() {
            if d >= depth {
                continue;
            }
            let Some(&idx) = self.index_map.get(&id) else {
                continue;
            };
            let next: Vec<NodeId> = self
                .graph
                .neighbors_directed(idx, Direction::Outgoing)
                .chain(self.graph.neighbors_directed(idx, Direction::Incoming))
                .filter_map(|n| self.graph.node_weight(n).map(|w| w.id.clone()))
                .collect();
            for nb in next {
                if visited.insert(nb.clone()) {
                    frontier.push_back((nb, d + 1));
                }
            }
        }

        visited.into_iter().collect()
    }

    /// Incrementally update the graph for a single changed file.
    /// Returns the persistence events generated.
    pub fn update_file(
        &mut self,
        path: &Path,
        new_content: &str,
        adapter: &dyn LanguageAdapter,
    ) -> Vec<GraphEvent> {
        let mut events = Vec::new();

        let to_remove: Vec<NodeId> = self
            .all_node_ids()
            .into_iter()
            .filter(|id| {
                self.get_node(id)
                    .map(|n| n.file_path == path)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        for id in &to_remove {
            if self.remove_node(id).is_some() {
                events.push(GraphEvent::NodeRemoved(id.clone()));
            }
        }

        if let Ok(parsed) = adapter.parse_file(path, new_content) {
            let nodes = adapter.extract_nodes(&parsed);
            for node in &nodes {
                self.add_node(node.clone());
                events.push(GraphEvent::NodeAdded(node.clone()));
            }
            let edges = adapter.extract_edges(&parsed, &nodes);
            for (from, to, data) in edges {
                if self.contains_node(&from) && self.contains_node(&to) {
                    let _ = self.add_edge(&from, &to, data.clone());
                    events.push(GraphEvent::EdgeAdded { from, to, data });
                }
            }
        }

        events.push(GraphEvent::FileReindexed(path.to_path_buf()));
        events
    }
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGraph {
    /// Build a read-optimised CSR snapshot of the current graph state.
    /// See [`crate::csr`] for the rationale.
    pub fn snapshot(&self) -> crate::csr::CsrSnapshot {
        crate::csr::CsrSnapshot::build(self)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::adapter::rust::RustAdapter;
    use crate::builder::GraphBuilder;
    use crate::edges::EdgeKind;
    use crate::nodes::{Span, Visibility};

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

    #[test]
    fn test_add_and_get_node() {
        let mut graph = CodeGraph::new();
        let node = make_node("foo", NodeKind::Function);
        let id = graph.add_node(node.clone());

        assert!(graph.contains_node(&id));
        let retrieved = graph.get_node(&id).unwrap();
        assert_eq!(retrieved.name, "foo");
    }

    #[test]
    fn test_inner_access() {
        let mut graph = CodeGraph::new();
        let node = make_node("bar", NodeKind::Function);
        graph.add_node(node);
        assert_eq!(graph.inner().node_count(), 1);
    }

    #[test]
    fn test_resolve_and_node_id_for() {
        let mut graph = CodeGraph::new();
        let node = make_node("baz", NodeKind::Struct);
        let id = graph.add_node(node);

        let idx = graph.resolve(&id).unwrap();
        let round_trip = graph.node_id_for(idx).unwrap();
        assert_eq!(&id, round_trip);
    }

    #[test]
    fn test_add_edge_and_retrieve() {
        let mut graph = CodeGraph::new();
        let a = make_node("a", NodeKind::Function);
        let b = make_node("b", NodeKind::Function);
        let a_id = graph.add_node(a);
        let b_id = graph.add_node(b);

        graph
            .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
            .unwrap();

        let edges_from_a = graph.get_edges_from(&a_id);
        assert_eq!(edges_from_a.len(), 1);
        assert_eq!(edges_from_a[0].0, &b_id);

        let edges_to_b = graph.get_edges_to(&b_id);
        assert_eq!(edges_to_b.len(), 1);
        assert_eq!(edges_to_b[0].0, &a_id);
    }

    #[test]
    fn test_remove_node() {
        let mut graph = CodeGraph::new();
        let node = make_node("remove_me", NodeKind::Function);
        let id = graph.add_node(node);

        assert!(graph.contains_node(&id));
        graph.remove_node(&id);
        assert!(!graph.contains_node(&id));
    }

    #[test]
    fn test_add_edge_rejects_calls_with_module_source_robust() {
        let mut graph = CodeGraph::new();
        let module = make_node("m", NodeKind::Module);
        let function = make_node("f", NodeKind::Function);
        let module_id = graph.add_node(module);
        let function_id = graph.add_node(function);

        let result = graph.add_edge(&module_id, &function_id, make_edge(EdgeKind::Calls));
        assert!(matches!(
            result,
            Err(GraphError::InvariantViolation(
                EdgeInvariantError::WrongSourceKind { .. }
            ))
        ));
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_add_edge_rejects_implements_with_function_target_robust() {
        let mut graph = CodeGraph::new();
        let s = make_node("S", NodeKind::Struct);
        let f = make_node("f", NodeKind::Function);
        let s_id = graph.add_node(s);
        let f_id = graph.add_node(f);

        let result = graph.add_edge(&s_id, &f_id, make_edge(EdgeKind::Implements));
        assert!(matches!(
            result,
            Err(GraphError::InvariantViolation(
                EdgeInvariantError::WrongTargetKind { .. }
            ))
        ));
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_add_edge_accepts_valid_contains_normal() {
        let mut graph = CodeGraph::new();
        let m = make_node("m", NodeKind::Module);
        let f = make_node("f", NodeKind::Function);
        let m_id = graph.add_node(m);
        let f_id = graph.add_node(f);

        graph
            .add_edge(&m_id, &f_id, make_edge(EdgeKind::Contains))
            .expect("Module → Function Contains is valid");
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_add_edge_returns_missing_node_when_target_absent_robust() {
        let mut graph = CodeGraph::new();
        let a = make_node("a", NodeKind::Function);
        let a_id = graph.add_node(a);

        let phantom = NodeId::new("nowhere.rs", "crate::phantom", NodeKind::Function);
        let result = graph.add_edge(&a_id, &phantom, make_edge(EdgeKind::Calls));
        match result {
            Err(GraphError::NodeNotFound(id)) => assert_eq!(id, phantom),
            other => panic!("expected NodeNotFound, got {other:?}"),
        }
    }

    #[test]
    fn test_add_edge_rejects_uses_type_with_function_target() {
        let mut graph = CodeGraph::new();
        let f = make_node("f", NodeKind::Function);
        let g = make_node("g", NodeKind::Function);
        let f_id = graph.add_node(f);
        let g_id = graph.add_node(g);

        let result = graph.add_edge(&f_id, &g_id, make_edge(EdgeKind::UsesType));
        assert!(matches!(
            result,
            Err(GraphError::InvariantViolation(
                EdgeInvariantError::WrongTargetKind { .. }
            ))
        ));
    }

    // Robust: NaN and infinite f32 weights are rejected at insertion so
    // downstream petgraph algorithms (`k_shortest_paths`, `dijkstra`) never
    // panic on a non-finite weight.
    #[test]
    fn add_edge_rejects_non_finite_weight_robust() {
        let mut graph = CodeGraph::new();
        let a_id = graph.add_node(make_node("a", NodeKind::Function));
        let b_id = graph.add_node(make_node("b", NodeKind::Function));

        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let edge = EdgeData {
                kind: EdgeKind::Calls,
                source_span: sample_span(),
                weight: bad,
            };
            let err = graph
                .add_edge(&a_id, &b_id, edge)
                .expect_err("non-finite weight must be rejected");
            assert!(matches!(
                err,
                GraphError::InvariantViolation(EdgeInvariantError::NonFiniteWeight { .. })
            ));
        }

        // Sanity: a finite weight inserts cleanly afterwards.
        graph
            .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
            .expect("finite weight inserts");
        assert_eq!(graph.get_edges_from(&a_id).len(), 1);
    }

    #[test]
    fn test_stable_indices_after_removal() {
        let mut graph = CodeGraph::new();
        let a_id = graph.add_node(make_node("a", NodeKind::Function));
        let b_id = graph.add_node(make_node("b", NodeKind::Function));
        let c_id = graph.add_node(make_node("c", NodeKind::Function));

        // Remove middle node
        graph.remove_node(&b_id);

        // Other indices still resolve correctly
        assert!(graph.resolve(&a_id).is_some());
        assert!(graph.resolve(&c_id).is_some());
        assert_eq!(graph.get_node(&a_id).unwrap().name, "a");
        assert_eq!(graph.get_node(&c_id).unwrap().name, "c");
    }

    /// Build a node with a fully-customised qualified name, file path, and
    /// metadata bag. Tests for the fast-lookup indices need finer-grained
    /// control than `make_node` offers — overloaded names across files,
    /// nested module paths, and metadata-keyed nodes all require it.
    fn make_node_full(
        name: &str,
        kind: NodeKind,
        qualified_name: &str,
        file_path: &str,
        metadata: HashMap<String, String>,
    ) -> NodeData {
        let id = NodeId::new(file_path, qualified_name, kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: qualified_name.to_string(),
            file_path: PathBuf::from(file_path),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata,
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    #[test]
    fn indices_kind_name_lookup_normal() {
        let mut graph = CodeGraph::new();
        let id = graph.add_node(make_node("compute", NodeKind::Function));

        let hits = graph.nodes_by_kind_name(NodeKind::Function, "compute");
        assert_eq!(hits, &[id.clone()]);

        // Same name, different kind → empty.
        assert!(
            graph
                .nodes_by_kind_name(NodeKind::Struct, "compute")
                .is_empty()
        );
    }

    #[test]
    fn indices_kind_name_returns_empty_for_missing_robust() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node("real", NodeKind::Function));

        // Unknown name → empty slice (must not panic, must not allocate
        // a sentinel).
        assert!(
            graph
                .nodes_by_kind_name(NodeKind::Function, "phantom")
                .is_empty()
        );
        // Empty graph for a different kind → also empty.
        let fresh = CodeGraph::new();
        assert!(
            fresh
                .nodes_by_kind_name(NodeKind::Trait, "anything")
                .is_empty()
        );
    }

    #[test]
    fn indices_kind_name_handles_multiple_robust() {
        // Two functions named `build` in different files / qualified names.
        // `NodeId` includes the file path, so these are distinct nodes; the
        // index must surface *both* of them under the same `(kind, name)`
        // key.
        let mut graph = CodeGraph::new();
        let a = graph.add_node(make_node_full(
            "build",
            NodeKind::Function,
            "crate::a::build",
            "src/a.rs",
            HashMap::new(),
        ));
        let b = graph.add_node(make_node_full(
            "build",
            NodeKind::Function,
            "crate::b::build",
            "src/b.rs",
            HashMap::new(),
        ));

        let hits: Vec<NodeId> = graph
            .nodes_by_kind_name(NodeKind::Function, "build")
            .to_vec();
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&a));
        assert!(hits.contains(&b));
    }

    #[test]
    fn indices_metadata_key_lookup_normal() {
        let mut graph = CodeGraph::new();

        let mut md_async = HashMap::new();
        md_async.insert("async".to_string(), "true".to_string());
        let async_id = graph.add_node(make_node_full(
            "fetch",
            NodeKind::Function,
            "crate::fetch",
            "src/fetch.rs",
            md_async,
        ));

        // A second node whose metadata also carries `async`.
        let mut md_async2 = HashMap::new();
        md_async2.insert("async".to_string(), "false".to_string());
        let async_id2 = graph.add_node(make_node_full(
            "block",
            NodeKind::Function,
            "crate::block",
            "src/block.rs",
            md_async2,
        ));

        // A third node *without* the `async` key.
        graph.add_node(make_node_full(
            "plain",
            NodeKind::Function,
            "crate::plain",
            "src/plain.rs",
            HashMap::new(),
        ));

        let hits: Vec<&NodeId> = graph.nodes_with_metadata_key("async").collect();
        assert_eq!(hits.len(), 2);
        assert!(hits.contains(&&async_id));
        assert!(hits.contains(&&async_id2));

        // Unknown key → empty iterator.
        assert_eq!(graph.nodes_with_metadata_key("unknown_key").count(), 0);
    }

    #[test]
    fn indices_module_prefix_matches_submodules_normal() {
        let mut graph = CodeGraph::new();

        let ui_app = graph.add_node(make_node_full(
            "App",
            NodeKind::Struct,
            "crate::ui::App",
            "src/ui.rs",
            HashMap::new(),
        ));
        let ui_render_draw = graph.add_node(make_node_full(
            "draw",
            NodeKind::Function,
            "crate::ui::render::draw",
            "src/ui/render.rs",
            HashMap::new(),
        ));
        let core_run = graph.add_node(make_node_full(
            "run",
            NodeKind::Function,
            "crate::core::run",
            "src/core.rs",
            HashMap::new(),
        ));

        // `crate::ui` should pull both the direct child and the deeper
        // submodule, but not the unrelated `crate::core` node.
        let ui_hits = graph.nodes_in_module("crate::ui");
        assert_eq!(ui_hits.len(), 2);
        assert!(ui_hits.contains(&ui_app));
        assert!(ui_hits.contains(&ui_render_draw));
        assert!(!ui_hits.contains(&core_run));

        // Deeper prefix narrows the result.
        let render_hits = graph.nodes_in_module("crate::ui::render");
        assert_eq!(render_hits, vec![ui_render_draw]);

        // Prefix that doesn't match any module returns nothing.
        assert!(graph.nodes_in_module("crate::missing").is_empty());
    }

    #[test]
    fn indices_remain_consistent_after_remove_robust() {
        let mut graph = CodeGraph::new();

        let mut md = HashMap::new();
        md.insert("flag".to_string(), "1".to_string());
        let n1 = graph.add_node(make_node_full(
            "alpha",
            NodeKind::Function,
            "crate::svc::alpha",
            "src/svc.rs",
            md.clone(),
        ));
        let n2 = graph.add_node(make_node_full(
            "beta",
            NodeKind::Function,
            "crate::svc::beta",
            "src/svc.rs",
            md.clone(),
        ));
        let n3 = graph.add_node(make_node_full(
            "gamma",
            NodeKind::Function,
            "crate::svc::gamma",
            "src/svc.rs",
            md,
        ));

        // Sanity: all three indexed everywhere.
        assert_eq!(graph.nodes_in_module("crate::svc").len(), 3);
        assert_eq!(graph.nodes_with_metadata_key("flag").count(), 3);

        graph.remove_node(&n2).expect("beta must exist");

        // `(Function, "beta")` bucket must be gone — index lookup returns
        // empty, no stale `NodeId` left behind.
        assert!(
            graph
                .nodes_by_kind_name(NodeKind::Function, "beta")
                .is_empty()
        );

        // The other two buckets still resolve.
        assert_eq!(
            graph
                .nodes_by_kind_name(NodeKind::Function, "alpha")
                .to_vec(),
            vec![n1.clone()]
        );
        assert_eq!(
            graph
                .nodes_by_kind_name(NodeKind::Function, "gamma")
                .to_vec(),
            vec![n3.clone()]
        );

        // Module / metadata indices reflect the deletion.
        let module_hits = graph.nodes_in_module("crate::svc");
        assert_eq!(module_hits.len(), 2);
        assert!(!module_hits.contains(&n2));
        let meta_hits: Vec<&NodeId> = graph.nodes_with_metadata_key("flag").collect();
        assert_eq!(meta_hits.len(), 2);
        assert!(!meta_hits.contains(&&n2));
    }

    #[test]
    fn rebuild_indices_from_scratch_normal() {
        // After populating the graph, calling `rebuild_indices` should be
        // a no-op observable through the public query methods — it's the
        // recovery hatch for hypothetical future code that bypasses the
        // mutation API. The contract: post-rebuild query results must
        // match the steady state.
        let mut graph = CodeGraph::new();

        let mut md = HashMap::new();
        md.insert("kind_tag".to_string(), "x".to_string());
        let s = graph.add_node(make_node_full(
            "Widget",
            NodeKind::Struct,
            "crate::widgets::Widget",
            "src/widgets.rs",
            md,
        ));
        let f = graph.add_node(make_node_full(
            "spawn",
            NodeKind::Function,
            "crate::widgets::spawn",
            "src/widgets.rs",
            HashMap::new(),
        ));

        let before_kind_name = graph
            .nodes_by_kind_name(NodeKind::Struct, "Widget")
            .to_vec();
        let before_module = graph.nodes_in_module("crate::widgets");
        let before_meta: Vec<NodeId> = graph
            .nodes_with_metadata_key("kind_tag")
            .cloned()
            .collect();

        graph.rebuild_indices();

        assert_eq!(
            graph
                .nodes_by_kind_name(NodeKind::Struct, "Widget")
                .to_vec(),
            before_kind_name
        );
        assert_eq!(graph.nodes_in_module("crate::widgets"), before_module);
        let after_meta: Vec<NodeId> = graph
            .nodes_with_metadata_key("kind_tag")
            .cloned()
            .collect();
        assert_eq!(after_meta, before_meta);

        // And the rebuilt indices still see both nodes through the
        // module-prefix query.
        let module_hits = graph.nodes_in_module("crate::widgets");
        assert!(module_hits.contains(&s));
        assert!(module_hits.contains(&f));
    }
}
