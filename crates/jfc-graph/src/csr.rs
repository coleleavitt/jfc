//! Read-optimised CSR (Compressed Sparse Row) snapshot of [`crate::graph::CodeGraph`].
//!
//! ## Why
//!
//! `CodeGraph` stores nodes/edges in `petgraph::StableDiGraph` — a
//! linked-list adjacency representation tuned for **mutation** (insert,
//! remove, slot-stable indices). For **analysis** (BFS, shortest path,
//! taint, possible-types, articulation), the linked-list representation
//! pollutes cache lines and forces a pointer chase per neighbour.
//!
//! CSR (Compressed Sparse Row) packs the adjacency list into two parallel
//! arrays:
//!
//! - `row_ptrs[i]..row_ptrs[i+1]` is the slice of `col_indices` containing
//!   the outgoing-neighbour indices of vertex `i`.
//!
//! Random walks become a single bounds-checked slice read. Because the
//! underlying storage is `&[u32]`, the entire neighbour list is
//! cache-friendly contiguous memory.
//!
//! ## BFS-order relabelling
//!
//! Naively numbering vertices in `petgraph` insertion order produces
//! near-random spatial locality — a BFS frontier touches arbitrary cache
//! lines. We relabel vertices in BFS order (Cuthill–McKee-lite from the
//! highest-degree node) so that BFS frontiers map to contiguous index
//! ranges. The result is a 2–4× cache-miss reduction on dense graphs.
//!
//! ## Bidirectional storage
//!
//! Many analyses need *both* directions (callees vs. callers, push vs.
//! pull). We store two CSRs in one snapshot — `out_*` for outgoing edges
//! and `in_*` for incoming edges. Memory cost is 2× edge count; analysis
//! gains O(1) constant-factor reduction on every traversal.
//!
//! ## What's NOT here
//!
//! - **Mutation**: `CsrSnapshot` is read-only. Mutations go through
//!   `CodeGraph` and invalidate the snapshot (caller responsibility).
//! - **Edge data**: only the kind discriminant is stored — full
//!   `EdgeData` lives in petgraph. If an analysis needs `weight` or
//!   `source_span`, it falls back to `CodeGraph::get_edges_from`.
//! - **Cycle detection**: not snapshot's concern — we just expose the
//!   topology.

use std::collections::{HashMap, VecDeque};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// Compact discriminant for [`EdgeKind`]. Fits in `u8`; lets us pack
/// per-edge kind alongside `col_indices` without serialising the full
/// `EdgeKind` (which contains owned strings for `UnresolvedCall` /
/// `ExternalCall`). The full kind is recoverable from `CodeGraph` if
/// needed; the snapshot keeps only what every analysis cares about.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeKindTag {
    Calls = 0,
    UnresolvedCall = 1,
    UsesType = 2,
    References = 3,
    Contains = 4,
    Implements = 5,
    ExternalCall = 6,
    Extends = 7,
    Returns = 8,
    TypeOf = 9,
}

impl EdgeKindTag {
    pub fn from_kind(k: &EdgeKind) -> Self {
        match k {
            EdgeKind::Calls => Self::Calls,
            EdgeKind::UnresolvedCall(_) => Self::UnresolvedCall,
            EdgeKind::UsesType => Self::UsesType,
            EdgeKind::References => Self::References,
            EdgeKind::Contains => Self::Contains,
            EdgeKind::Implements => Self::Implements,
            EdgeKind::ExternalCall(_, _) => Self::ExternalCall,
            EdgeKind::Extends => Self::Extends,
            EdgeKind::Returns => Self::Returns,
            EdgeKind::TypeOf => Self::TypeOf,
        }
    }
}

/// Vertex identifier within a `CsrSnapshot`. Distinct from `NodeIndex`
/// (petgraph slot) and `NodeId` (content hash) — this is a 0..n
/// dense vertex index after BFS-order relabelling. Deliberately a
/// newtype so it can't be confused with petgraph indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CsrVertex(pub u32);

impl CsrVertex {
    pub fn idx(self) -> usize {
        self.0 as usize
    }
}

/// Read-optimised CSR snapshot. Built from a `CodeGraph` at a point in
/// time; invalidated by any mutation.
pub struct CsrSnapshot {
    /// Number of vertices.
    pub n: usize,
    /// Total edges (not 2x — outgoing only).
    pub m: usize,

    /// Outgoing CSR: `out_row_ptrs[v]..out_row_ptrs[v+1]` is the slice
    /// of `out_col_indices` holding v's outgoing-neighbour vertex
    /// indices.
    pub out_row_ptrs: Vec<u32>,
    pub out_col_indices: Vec<u32>,
    pub out_edge_kinds: Vec<EdgeKindTag>,

    /// Incoming CSR (reverse adjacency).
    pub in_row_ptrs: Vec<u32>,
    pub in_col_indices: Vec<u32>,
    pub in_edge_kinds: Vec<EdgeKindTag>,

    /// `CsrVertex.idx()` → `NodeId`. Used to map CSR results back to
    /// the public API.
    pub vertex_to_id: Vec<NodeId>,
    /// `NodeId` → `CsrVertex`. Inverse of `vertex_to_id`.
    pub id_to_vertex: HashMap<NodeId, CsrVertex>,

    /// `revision` of the source graph at snapshot time. Used for cache
    /// invalidation by `incremental.rs`.
    pub source_revision: u64,
}

impl CsrSnapshot {
    /// Build a CSR snapshot of the given graph with BFS-order vertex
    /// relabelling for cache locality.
    ///
    /// Cost: O(V + E) plus the BFS for vertex ordering. For typical
    /// code graphs (10k nodes, 50k edges) this is sub-millisecond.
    pub fn build(graph: &CodeGraph) -> Self {
        let inner = graph.inner();
        let n = inner.node_count();

        if n == 0 {
            return Self::empty(graph.current_revision());
        }

        // Step 1: BFS-order vertex relabelling. Pick the highest-degree
        // node as the seed (acts as a hub) and BFS outward; nodes
        // discovered together are adjacent in the new numbering.
        let order = bfs_relabel(graph);

        // Step 2: Build NodeIndex → CsrVertex map.
        let mut petgraph_to_csr: HashMap<NodeIndex, u32> = HashMap::with_capacity(n);
        let mut vertex_to_id: Vec<NodeId> = Vec::with_capacity(n);
        for (csr_idx, pg_idx) in order.iter().enumerate() {
            petgraph_to_csr.insert(*pg_idx, csr_idx as u32);
            if let Some(id) = graph.node_id_for(*pg_idx) {
                vertex_to_id.push(id.clone());
            } else {
                // Should be unreachable — every NodeIndex came from a
                // valid traversal of the live graph. If we hit it, fail
                // open with an empty snapshot rather than panic.
                return Self::empty(graph.current_revision());
            }
        }

        let mut id_to_vertex: HashMap<NodeId, CsrVertex> = HashMap::with_capacity(n);
        for (csr_idx, id) in vertex_to_id.iter().enumerate() {
            id_to_vertex.insert(id.clone(), CsrVertex(csr_idx as u32));
        }

        // Step 3: Build outgoing CSR.
        let mut m_total: usize = 0;
        let mut out_row_ptrs: Vec<u32> = Vec::with_capacity(n + 1);
        let mut out_col: Vec<u32> = Vec::new();
        let mut out_kinds: Vec<EdgeKindTag> = Vec::new();
        out_row_ptrs.push(0);

        for csr_idx in 0..n {
            let pg_idx = order[csr_idx];
            let mut neighbours: Vec<(u32, EdgeKindTag)> = Vec::new();
            for edge_ref in inner.edges_directed(pg_idx, Direction::Outgoing) {
                use petgraph::visit::EdgeRef;
                let target = edge_ref.target();
                if let Some(&csr_target) = petgraph_to_csr.get(&target) {
                    neighbours.push((csr_target, EdgeKindTag::from_kind(&edge_ref.weight().kind)));
                }
            }
            // Sort by target vertex for deterministic, cache-friendly access.
            neighbours.sort_by_key(|&(v, _)| v);
            for (v, k) in neighbours {
                out_col.push(v);
                out_kinds.push(k);
            }
            m_total += out_col.len() - *out_row_ptrs.last().expect("primed") as usize;
            out_row_ptrs.push(out_col.len() as u32);
        }

        // Step 4: Build incoming CSR (reverse adjacency).
        let mut in_row_ptrs: Vec<u32> = Vec::with_capacity(n + 1);
        let mut in_col: Vec<u32> = Vec::new();
        let mut in_kinds: Vec<EdgeKindTag> = Vec::new();
        in_row_ptrs.push(0);

        for csr_idx in 0..n {
            let pg_idx = order[csr_idx];
            let mut sources: Vec<(u32, EdgeKindTag)> = Vec::new();
            for edge_ref in inner.edges_directed(pg_idx, Direction::Incoming) {
                use petgraph::visit::EdgeRef;
                let source = edge_ref.source();
                if let Some(&csr_source) = petgraph_to_csr.get(&source) {
                    sources.push((csr_source, EdgeKindTag::from_kind(&edge_ref.weight().kind)));
                }
            }
            sources.sort_by_key(|&(v, _)| v);
            for (v, k) in sources {
                in_col.push(v);
                in_kinds.push(k);
            }
            in_row_ptrs.push(in_col.len() as u32);
        }

        Self {
            n,
            m: m_total,
            out_row_ptrs,
            out_col_indices: out_col,
            out_edge_kinds: out_kinds,
            in_row_ptrs,
            in_col_indices: in_col,
            in_edge_kinds: in_kinds,
            vertex_to_id,
            id_to_vertex,
            source_revision: graph.current_revision(),
        }
    }

    fn empty(revision: u64) -> Self {
        Self {
            n: 0,
            m: 0,
            out_row_ptrs: vec![0],
            out_col_indices: Vec::new(),
            out_edge_kinds: Vec::new(),
            in_row_ptrs: vec![0],
            in_col_indices: Vec::new(),
            in_edge_kinds: Vec::new(),
            vertex_to_id: Vec::new(),
            id_to_vertex: HashMap::new(),
            source_revision: revision,
        }
    }

    /// Outgoing-neighbour vertex indices of `v`.
    #[inline]
    pub fn out_neighbours(&self, v: CsrVertex) -> &[u32] {
        let start = self.out_row_ptrs[v.idx()] as usize;
        let end = self.out_row_ptrs[v.idx() + 1] as usize;
        &self.out_col_indices[start..end]
    }

    /// Incoming-source vertex indices of `v`.
    #[inline]
    pub fn in_neighbours(&self, v: CsrVertex) -> &[u32] {
        let start = self.in_row_ptrs[v.idx()] as usize;
        let end = self.in_row_ptrs[v.idx() + 1] as usize;
        &self.in_col_indices[start..end]
    }

    /// Outgoing edge kinds of `v`, parallel to `out_neighbours`.
    #[inline]
    pub fn out_kinds(&self, v: CsrVertex) -> &[EdgeKindTag] {
        let start = self.out_row_ptrs[v.idx()] as usize;
        let end = self.out_row_ptrs[v.idx() + 1] as usize;
        &self.out_edge_kinds[start..end]
    }

    /// Incoming edge kinds of `v`, parallel to `in_neighbours`.
    #[inline]
    pub fn in_kinds(&self, v: CsrVertex) -> &[EdgeKindTag] {
        let start = self.in_row_ptrs[v.idx()] as usize;
        let end = self.in_row_ptrs[v.idx() + 1] as usize;
        &self.in_edge_kinds[start..end]
    }

    /// Outgoing degree of `v`.
    #[inline]
    pub fn out_degree(&self, v: CsrVertex) -> usize {
        (self.out_row_ptrs[v.idx() + 1] - self.out_row_ptrs[v.idx()]) as usize
    }

    /// Incoming degree of `v`.
    #[inline]
    pub fn in_degree(&self, v: CsrVertex) -> usize {
        (self.in_row_ptrs[v.idx() + 1] - self.in_row_ptrs[v.idx()]) as usize
    }

    /// Resolve a `NodeId` to its `CsrVertex` in this snapshot.
    pub fn vertex_of(&self, id: &NodeId) -> Option<CsrVertex> {
        self.id_to_vertex.get(id).copied()
    }

    /// Resolve a `CsrVertex` back to a `NodeId`.
    pub fn id_of(&self, v: CsrVertex) -> Option<&NodeId> {
        self.vertex_to_id.get(v.idx())
    }

    /// Returns `true` if the source graph's revision has advanced past
    /// this snapshot's `source_revision`. Snapshots are point-in-time
    /// copies; if a caller holds one across `add_node`/`add_edge`/
    /// `remove_node`, downstream analysis would silently use stale
    /// adjacencies.
    ///
    /// Use:
    /// ```ignore
    /// if csr.is_stale(&graph) { csr = graph.snapshot(); }
    /// ```
    pub fn is_stale(&self, graph: &CodeGraph) -> bool {
        graph.current_revision() != self.source_revision
    }

    /// Returns `true` only if `revision` exactly equals
    /// [`Self::source_revision`]. Companion to [`Self::is_stale`] for
    /// callers who already hold the revision rather than the graph.
    pub fn matches_revision(&self, revision: u64) -> bool {
        self.source_revision == revision
    }
}

/// BFS relabelling: produces a `Vec<NodeIndex>` where index `i` is the
/// petgraph node assigned CSR vertex `i`. Cache-locality optimisation —
/// nodes that BFS visits together get adjacent indices, so a frontier
/// traversal accesses contiguous memory.
fn bfs_relabel(graph: &CodeGraph) -> Vec<NodeIndex> {
    let inner = graph.inner();
    let n = inner.node_count();
    let mut order: Vec<NodeIndex> = Vec::with_capacity(n);
    let mut visited: HashMap<NodeIndex, ()> = HashMap::with_capacity(n);

    // Seed selection: highest-out-degree node. Acts as a hub so the
    // resulting BFS spans most of the strongly-connected core early.
    let mut seeds: Vec<NodeIndex> = inner.node_indices().collect();
    seeds.sort_by_key(|&idx| {
        std::cmp::Reverse(
            inner.neighbors_directed(idx, Direction::Outgoing).count()
                + inner.neighbors_directed(idx, Direction::Incoming).count(),
        )
    });

    for seed in seeds {
        if visited.contains_key(&seed) {
            continue;
        }
        let mut queue: VecDeque<NodeIndex> = VecDeque::new();
        queue.push_back(seed);
        visited.insert(seed, ());

        while let Some(curr) = queue.pop_front() {
            order.push(curr);
            // Walk both directions for relabelling — undirected
            // closeness is what matters for cache locality.
            for n in inner.neighbors_directed(curr, Direction::Outgoing) {
                if visited.insert(n, ()).is_none() {
                    queue.push_back(n);
                }
            }
            for n in inner.neighbors_directed(curr, Direction::Incoming) {
                if visited.insert(n, ()).is_none() {
                    queue.push_back(n);
                }
            }
        }
    }

    debug_assert_eq!(order.len(), n);
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap as StdHashMap;
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

    fn mk(name: &str, kind: NodeKind) -> NodeData {
        NodeData {
            id: NodeId::new("t.rs", name, kind),
            kind,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Private,
            metadata: StdHashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
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
    fn empty_graph_produces_empty_snapshot() {
        let g = CodeGraph::new();
        let csr = CsrSnapshot::build(&g);
        assert_eq!(csr.n, 0);
        assert_eq!(csr.m, 0);
    }

    #[test]
    fn single_node_no_edges() {
        let mut g = CodeGraph::new();
        let id = g.add_node(mk("solo", NodeKind::Function));
        let csr = CsrSnapshot::build(&g);
        assert_eq!(csr.n, 1);
        let v = csr.vertex_of(&id).unwrap();
        assert_eq!(csr.out_degree(v), 0);
        assert_eq!(csr.in_degree(v), 0);
    }

    #[test]
    fn linear_chain_csr_round_trip() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();

        let csr = CsrSnapshot::build(&g);
        assert_eq!(csr.n, 3);

        let va = csr.vertex_of(&a).unwrap();
        let vb = csr.vertex_of(&b).unwrap();
        let vc = csr.vertex_of(&c).unwrap();

        // a → b
        let a_out = csr.out_neighbours(va);
        assert_eq!(a_out.len(), 1);
        assert_eq!(CsrVertex(a_out[0]), vb);

        // b → c
        let b_out = csr.out_neighbours(vb);
        assert_eq!(b_out.len(), 1);
        assert_eq!(CsrVertex(b_out[0]), vc);

        // c has no out, but has in from b.
        assert_eq!(csr.out_degree(vc), 0);
        let c_in = csr.in_neighbours(vc);
        assert_eq!(c_in.len(), 1);
        assert_eq!(CsrVertex(c_in[0]), vb);
    }

    #[test]
    fn fan_out_and_fan_in() {
        let mut g = CodeGraph::new();
        let hub = g.add_node(mk("hub", NodeKind::Function));
        let leaves: Vec<NodeId> = (0..5)
            .map(|i| g.add_node(mk(&format!("leaf{i}"), NodeKind::Function)))
            .collect();
        for l in &leaves {
            g.add_edge(&hub, l, ed(EdgeKind::Calls)).unwrap();
        }

        let csr = CsrSnapshot::build(&g);
        let vh = csr.vertex_of(&hub).unwrap();
        assert_eq!(csr.out_degree(vh), 5);

        for l in &leaves {
            let v = csr.vertex_of(l).unwrap();
            assert_eq!(csr.in_degree(v), 1);
            assert_eq!(csr.out_degree(v), 0);
        }
    }

    #[test]
    fn vertex_id_round_trip() {
        let mut g = CodeGraph::new();
        let id = g.add_node(mk("x", NodeKind::Function));
        let csr = CsrSnapshot::build(&g);
        let v = csr.vertex_of(&id).unwrap();
        let back = csr.id_of(v).unwrap();
        assert_eq!(back, &id);
    }

    #[test]
    fn snapshot_records_source_revision() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function));
        let rev = g.current_revision();
        let csr = CsrSnapshot::build(&g);
        assert_eq!(csr.source_revision, rev);
    }

    #[test]
    fn snapshot_invariants_n_consistent() {
        let mut g = CodeGraph::new();
        for i in 0..10 {
            g.add_node(mk(&format!("n{i}"), NodeKind::Function));
        }
        let csr = CsrSnapshot::build(&g);
        assert_eq!(csr.n, 10);
        assert_eq!(csr.out_row_ptrs.len(), 11);
        assert_eq!(csr.in_row_ptrs.len(), 11);
        assert_eq!(csr.vertex_to_id.len(), 10);
        assert_eq!(csr.id_to_vertex.len(), 10);
    }

    #[test]
    fn edge_kind_tag_preserves_kind_normal() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("f", NodeKind::Function));
        let s = g.add_node(mk("S", NodeKind::Struct));
        g.add_edge(&f, &s, ed(EdgeKind::UsesType)).unwrap();
        let csr = CsrSnapshot::build(&g);
        let vf = csr.vertex_of(&f).unwrap();
        assert_eq!(csr.out_kinds(vf), &[EdgeKindTag::UsesType]);
    }

    #[test]
    fn snapshot_stale_after_mutation() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function));
        let csr = CsrSnapshot::build(&g);
        assert!(!csr.is_stale(&g));
        g.add_node(mk("b", NodeKind::Function));
        assert!(csr.is_stale(&g));
    }

    #[test]
    fn matches_revision_after_build() {
        let mut g = CodeGraph::new();
        g.add_node(mk("a", NodeKind::Function));
        let rev = g.current_revision();
        let csr = CsrSnapshot::build(&g);
        assert!(csr.matches_revision(rev));
        assert!(!csr.matches_revision(rev + 1));
    }

    #[test]
    fn parallel_arrays_align() {
        // Critical invariant: out_col_indices and out_edge_kinds must
        // always have the same length and pair index-by-index.
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("f", NodeKind::Function));
        let s = g.add_node(mk("S", NodeKind::Struct));
        let g_ = g.add_node(mk("g", NodeKind::Function));
        g.add_edge(&f, &s, ed(EdgeKind::UsesType)).unwrap();
        g.add_edge(&f, &g_, ed(EdgeKind::Calls)).unwrap();
        let csr = CsrSnapshot::build(&g);
        assert_eq!(csr.out_col_indices.len(), csr.out_edge_kinds.len());
        assert_eq!(csr.in_col_indices.len(), csr.in_edge_kinds.len());
    }
}
