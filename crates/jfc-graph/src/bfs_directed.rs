//! Direction-optimised BFS (Yang et al. 2018, "Implementing Push-Pull
//! Efficiently in GraphBLAS").
//!
//! Runs BFS over a [`crate::csr::CsrSnapshot`] with **dynamic
//! push/pull switching**. Push expansion (iterate frontier, add
//! successors) is cheap when the frontier is small; pull expansion
//! (iterate every unvisited vertex, scan its predecessors for any
//! already-visited) is cheap when the frontier dominates the graph.
//!
//! ## Switch heuristic
//!
//! Yang's three knobs (alpha, beta, gamma) factor the decision:
//!
//! - `alpha`: switch push → pull when frontier-edges > m / alpha.
//! - `beta`:  switch pull → push when next-frontier < n / beta.
//! - `gamma`: bound on frontier growth rate; we omit (defaults safe).
//!
//! Defaults `alpha=14`, `beta=24` are the values reported optimal in
//! Yang's evaluation across the 14 benchmark graphs in the paper.
//!
//! ## When to use
//!
//! Anything that visits a large fraction of the graph by reachability:
//! `callers`/`callees` traversal at large depth, taint analysis,
//! possible-types propagation, dominators. For tiny BFS (depth=1,
//! single source) the legacy hashset BFS in `traversal.rs` is fine —
//! the fixed cost of building a `CsrSnapshot` (~1ms for 10k nodes)
//! dominates.

use crate::csr::{CsrSnapshot, CsrVertex, EdgeKindTag};
use crate::frontier::Frontier;

/// Yang 2018 push→pull switch parameter. Switch when frontier edges
/// exceed `m / ALPHA`. Larger ALPHA = stay in push longer.
pub const ALPHA: usize = 14;
/// Yang 2018 pull→push switch parameter. Switch back when next
/// frontier shrinks below `n / BETA`.
pub const BETA: usize = 24;

/// Result of a direction-optimised BFS.
pub struct DirectedBfsResult {
    /// Vertex → distance from source. `None` for unreachable.
    pub depth: Vec<Option<u32>>,
    /// Vertex → predecessor in BFS tree. `None` for source / unreachable.
    pub parent: Vec<Option<CsrVertex>>,
    /// Final frontier exhaustion depth.
    pub max_depth_reached: u32,
}

/// Direction the BFS is currently executing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Iterate frontier, expand to outgoing neighbours.
    Push,
    /// Iterate every unvisited vertex, scan incoming neighbours for
    /// any in-frontier predecessor.
    Pull,
}

/// Edge filter — restrict BFS to specific edge kinds (e.g. only `Calls`).
pub struct BfsConfig<'a> {
    pub max_depth: u32,
    /// If `Some`, only edges whose kind matches one of these tags are
    /// followed. `None` = follow every edge.
    pub edge_filter: Option<&'a [EdgeKindTag]>,
    /// Reverse the BFS (walk incoming edges as outgoing). Used for
    /// "callers" semantics: the BFS algorithm itself is unchanged, we
    /// just swap which array is the "out" adjacency.
    pub reverse: bool,
}

impl<'a> BfsConfig<'a> {
    pub fn new(max_depth: u32) -> Self {
        Self {
            max_depth,
            edge_filter: None,
            reverse: false,
        }
    }
    pub fn reverse(mut self) -> Self {
        self.reverse = true;
        self
    }
    pub fn with_edge_filter(mut self, filter: &'a [EdgeKindTag]) -> Self {
        self.edge_filter = Some(filter);
        self
    }
}

/// Direction-optimised BFS from a single source vertex.
pub fn bfs(snapshot: &CsrSnapshot, source: CsrVertex, cfg: &BfsConfig<'_>) -> DirectedBfsResult {
    let n = snapshot.n;
    let mut depth: Vec<Option<u32>> = vec![None; n];
    let mut parent: Vec<Option<CsrVertex>> = vec![None; n];
    let mut max_depth_reached: u32 = 0;

    if n == 0 || source.idx() >= n {
        return DirectedBfsResult {
            depth,
            parent,
            max_depth_reached,
        };
    }

    depth[source.idx()] = Some(0);

    let mut current: Frontier = Frontier::singleton(n, source.0);
    let mut next: Frontier = Frontier::new(n);
    let mut current_depth: u32 = 0;
    let mut direction = Direction::Push;

    while !current.is_empty() && current_depth < cfg.max_depth {
        // Pick direction for this layer.
        direction = pick_direction(direction, &current, snapshot, cfg);

        match direction {
            Direction::Push => push_step(snapshot, &current, &mut next, &mut depth, &mut parent, current_depth + 1, cfg),
            Direction::Pull => pull_step(snapshot, &current, &mut next, &mut depth, &mut parent, current_depth + 1, cfg),
        }

        if !next.is_empty() {
            current_depth += 1;
            max_depth_reached = current_depth;
        }
        std::mem::swap(&mut current, &mut next);
        next.clear();
    }

    DirectedBfsResult {
        depth,
        parent,
        max_depth_reached,
    }
}

/// Yang's push-vs-pull selection heuristic. See module docs.
fn pick_direction(
    current: Direction,
    frontier: &Frontier,
    snapshot: &CsrSnapshot,
    cfg: &BfsConfig<'_>,
) -> Direction {
    let n = snapshot.n;
    let m = snapshot.m.max(1);
    if n == 0 {
        return Direction::Push;
    }

    let frontier_edges = frontier.push_workload(|v| {
        let cv = CsrVertex(v);
        if cfg.reverse {
            snapshot.in_degree(cv)
        } else {
            snapshot.out_degree(cv)
        }
    });

    match current {
        Direction::Push => {
            // Switch to pull when frontier edges exceed m / ALPHA.
            if frontier_edges > m / ALPHA {
                Direction::Pull
            } else {
                Direction::Push
            }
        }
        Direction::Pull => {
            // Switch back to push when frontier shrinks under n / BETA.
            if frontier.len() < n / BETA {
                Direction::Push
            } else {
                Direction::Pull
            }
        }
    }
}

fn push_step(
    snapshot: &CsrSnapshot,
    current: &Frontier,
    next: &mut Frontier,
    depth: &mut [Option<u32>],
    parent: &mut [Option<CsrVertex>],
    new_depth: u32,
    cfg: &BfsConfig<'_>,
) {
    for v in current.iter() {
        let cv = CsrVertex(v);
        let neighbours = if cfg.reverse {
            snapshot.in_neighbours(cv)
        } else {
            snapshot.out_neighbours(cv)
        };
        let kinds = if cfg.reverse {
            snapshot.in_kinds(cv)
        } else {
            snapshot.out_kinds(cv)
        };

        for (i, &nbr) in neighbours.iter().enumerate() {
            if !edge_matches(cfg.edge_filter, kinds[i]) {
                continue;
            }
            let nbr_u = nbr as usize;
            if depth[nbr_u].is_none() {
                depth[nbr_u] = Some(new_depth);
                parent[nbr_u] = Some(cv);
                next.insert(nbr);
            }
        }
    }
}

fn pull_step(
    snapshot: &CsrSnapshot,
    current: &Frontier,
    next: &mut Frontier,
    depth: &mut [Option<u32>],
    parent: &mut [Option<CsrVertex>],
    new_depth: u32,
    cfg: &BfsConfig<'_>,
) {
    for v in 0..(snapshot.n as u32) {
        let vu = v as usize;
        if depth[vu].is_some() {
            continue;
        }
        // Look at predecessors (or successors if reverse) and check if any are in current.
        let preds = if cfg.reverse {
            snapshot.out_neighbours(CsrVertex(v))
        } else {
            snapshot.in_neighbours(CsrVertex(v))
        };
        let kinds = if cfg.reverse {
            snapshot.out_kinds(CsrVertex(v))
        } else {
            snapshot.in_kinds(CsrVertex(v))
        };

        for (i, &p) in preds.iter().enumerate() {
            if !edge_matches(cfg.edge_filter, kinds[i]) {
                continue;
            }
            if current.contains(p) {
                depth[vu] = Some(new_depth);
                parent[vu] = Some(CsrVertex(p));
                next.insert(v);
                break;
            }
        }
    }
}

#[inline]
fn edge_matches(filter: Option<&[EdgeKindTag]>, k: EdgeKindTag) -> bool {
    match filter {
        None => true,
        Some(allowed) => allowed.contains(&k),
    }
}

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
            metadata: HashMap::new(),
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
    fn bfs_finds_all_reachable() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        let d = g.add_node(mk("d", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&a, &d, ed(EdgeKind::Calls)).unwrap();

        let csr = g.snapshot();
        let v_a = csr.vertex_of(&a).unwrap();
        let r = bfs(&csr, v_a, &BfsConfig::new(10));
        let v_b = csr.vertex_of(&b).unwrap();
        let v_c = csr.vertex_of(&c).unwrap();
        let v_d = csr.vertex_of(&d).unwrap();

        assert_eq!(r.depth[v_a.idx()], Some(0));
        assert_eq!(r.depth[v_b.idx()], Some(1));
        assert_eq!(r.depth[v_c.idx()], Some(2));
        assert_eq!(r.depth[v_d.idx()], Some(1));
    }

    #[test]
    fn bfs_respects_max_depth() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();

        let csr = g.snapshot();
        let r = bfs(&csr, csr.vertex_of(&a).unwrap(), &BfsConfig::new(1));
        assert_eq!(r.depth[csr.vertex_of(&a).unwrap().idx()], Some(0));
        assert_eq!(r.depth[csr.vertex_of(&b).unwrap().idx()], Some(1));
        assert_eq!(r.depth[csr.vertex_of(&c).unwrap().idx()], None);
    }

    #[test]
    fn bfs_reverse_walks_callers() {
        let mut g = CodeGraph::new();
        let caller = g.add_node(mk("caller", NodeKind::Function));
        let callee = g.add_node(mk("callee", NodeKind::Function));
        g.add_edge(&caller, &callee, ed(EdgeKind::Calls)).unwrap();
        let csr = g.snapshot();

        // Forward from callee → empty.
        let r = bfs(&csr, csr.vertex_of(&callee).unwrap(), &BfsConfig::new(10));
        assert_eq!(r.depth[csr.vertex_of(&caller).unwrap().idx()], None);

        // Reverse from callee → caller at depth 1.
        let r = bfs(&csr, csr.vertex_of(&callee).unwrap(), &BfsConfig::new(10).reverse());
        assert_eq!(r.depth[csr.vertex_of(&caller).unwrap().idx()], Some(1));
    }

    #[test]
    fn bfs_edge_filter_restricts_kinds() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("f", NodeKind::Function));
        let s = g.add_node(mk("S", NodeKind::Struct));
        let g_ = g.add_node(mk("g", NodeKind::Function));
        g.add_edge(&f, &s, ed(EdgeKind::UsesType)).unwrap();
        g.add_edge(&f, &g_, ed(EdgeKind::Calls)).unwrap();
        let csr = g.snapshot();

        // Only Calls edges → struct unreachable.
        let r = bfs(
            &csr,
            csr.vertex_of(&f).unwrap(),
            &BfsConfig::new(10).with_edge_filter(&[EdgeKindTag::Calls]),
        );
        assert_eq!(r.depth[csr.vertex_of(&g_).unwrap().idx()], Some(1));
        assert_eq!(r.depth[csr.vertex_of(&s).unwrap().idx()], None);
    }

    #[test]
    fn bfs_records_parent_chain() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();
        let csr = g.snapshot();
        let r = bfs(&csr, csr.vertex_of(&a).unwrap(), &BfsConfig::new(10));
        assert_eq!(r.parent[csr.vertex_of(&b).unwrap().idx()], Some(csr.vertex_of(&a).unwrap()));
        assert_eq!(r.parent[csr.vertex_of(&c).unwrap().idx()], Some(csr.vertex_of(&b).unwrap()));
    }

    #[test]
    fn empty_graph_bfs_is_safe() {
        let g = CodeGraph::new();
        let csr = g.snapshot();
        let r = bfs(&csr, CsrVertex(0), &BfsConfig::new(5));
        assert!(r.depth.is_empty());
    }

    #[test]
    fn cyclic_graph_terminates() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&c, &a, ed(EdgeKind::Calls)).unwrap();

        let csr = g.snapshot();
        let r = bfs(&csr, csr.vertex_of(&a).unwrap(), &BfsConfig::new(100));
        // BFS must visit each node exactly once and not loop.
        for d in &r.depth {
            assert!(d.is_some());
        }
    }

    #[test]
    fn dense_graph_uses_pull() {
        // Build a graph with one source and many siblings — eventually
        // the frontier dominates and pull should kick in. We can't
        // easily inspect the internal direction but we can verify
        // correctness on a graph that exercises both paths.
        let mut g = CodeGraph::new();
        let root = g.add_node(mk("root", NodeKind::Function));
        let mut leaves = Vec::new();
        for i in 0..50 {
            let leaf = g.add_node(mk(&format!("leaf{i}"), NodeKind::Function));
            g.add_edge(&root, &leaf, ed(EdgeKind::Calls)).unwrap();
            leaves.push(leaf);
        }
        let csr = g.snapshot();
        let r = bfs(&csr, csr.vertex_of(&root).unwrap(), &BfsConfig::new(10));
        for l in &leaves {
            assert_eq!(r.depth[csr.vertex_of(l).unwrap().idx()], Some(1));
        }
    }
}
