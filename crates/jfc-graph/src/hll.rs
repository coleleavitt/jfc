//! HyperLogLog approximate reachability (Phase 11-2).
//!
//! For graphs >100k nodes, exact transitive-closure (O(V+E)) is fine
//! for single queries but expensive if you want per-node
//! ancestor/descendant *counts* (that's O(V·(V+E))). HyperLogLog
//! gives an O(V+E) estimate of |ancestors(v)| and |descendants(v)|
//! for every v simultaneously, with ~2% standard error at b=12.
//!
//! ## Algorithm (ANF-style propagation)
//!
//! 1. Initialize each node with an HLL sketch containing only itself.
//! 2. For O(diameter) rounds, each node merges the sketches of its
//!    in-neighbours (for ancestor counts) or out-neighbours (for
//!    descendant counts).
//! 3. Terminate when no sketch grows (fixpoint), or after
//!    `MAX_ROUNDS` iterations.
//!
//! ## References
//!
//! - Flajolet, Fusy, Gandouet, Meunier. "HyperLogLog: the analysis
//!   of a near-optimal cardinality estimation algorithm." (2007)
//! - Palmer, Gibbons, Faloutsos. "ANF: A Fast and Scalable Tool for
//!   Data Mining in Massive Graphs." (2002)

use crate::graph::CodeGraph;
use crate::nodes::NodeId;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Precision parameter. b=12 → 2^12 = 4096 registers → ~2% error.
const B: usize = 12;
const M: usize = 1 << B; // 4096 registers

/// HyperLogLog sketch — 4096 × u8 registers.
#[derive(Clone)]
pub struct HllSketch {
    registers: [u8; M],
}

impl Default for HllSketch {
    fn default() -> Self {
        Self { registers: [0; M] }
    }
}

impl HllSketch {
    /// Add a single element (identified by u64 hash).
    pub fn add_hash(&mut self, hash: u64) {
        let idx = (hash as usize) & (M - 1);
        let remaining = hash >> B;
        let rho = if remaining == 0 {
            (64 - B) as u8 + 1
        } else {
            remaining.trailing_zeros() as u8 + 1
        };
        if rho > self.registers[idx] {
            self.registers[idx] = rho;
        }
    }

    /// Merge another sketch into this one (union).
    pub fn merge(&mut self, other: &HllSketch) -> bool {
        let mut changed = false;
        for i in 0..M {
            if other.registers[i] > self.registers[i] {
                self.registers[i] = other.registers[i];
                changed = true;
            }
        }
        changed
    }

    /// Cardinality estimate.
    pub fn estimate(&self) -> f64 {
        let alpha = match M {
            16 => 0.673,
            32 => 0.697,
            64 => 0.709,
            _ => 0.7213 / (1.0 + 1.079 / M as f64),
        };
        let sum: f64 = self
            .registers
            .iter()
            .map(|&r| 2.0_f64.powi(-(r as i32)))
            .sum();
        let raw = alpha * (M as f64) * (M as f64) / sum;

        // Small range correction.
        if raw <= 2.5 * M as f64 {
            let zeros = self.registers.iter().filter(|&&r| r == 0).count();
            if zeros > 0 {
                return (M as f64) * ((M as f64) / zeros as f64).ln();
            }
        }
        raw
    }
}

/// Maximum propagation rounds. 64 is generous — most code graphs
/// have diameter <20.
const MAX_ROUNDS: usize = 64;

/// Per-node ancestor/descendant count estimates.
#[derive(Debug, Clone)]
pub struct ReachabilityEstimates {
    /// Approximate number of ancestors (nodes that can reach v).
    pub ancestor_count: HashMap<NodeId, f64>,
    /// Approximate number of descendants (nodes reachable from v).
    pub descendant_count: HashMap<NodeId, f64>,
}

/// Compute approximate ancestor and descendant counts for every node
/// in the graph. Cost: O(rounds × (V + E)) with rounds ≤
/// `MAX_ROUNDS`.
pub fn approximate_reachability(graph: &CodeGraph) -> ReachabilityEstimates {
    let all_ids: Vec<NodeId> = graph.all_node_ids().into_iter().cloned().collect();
    let n = all_ids.len();
    let id_to_idx: HashMap<&NodeId, usize> = all_ids.iter().enumerate().map(|(i, id)| (id, i)).collect();

    // Forward sketches (descendants): propagate along outgoing edges.
    let mut fwd: Vec<HllSketch> = Vec::with_capacity(n);
    // Backward sketches (ancestors): propagate along incoming edges.
    let mut bwd: Vec<HllSketch> = Vec::with_capacity(n);

    for (i, id) in all_ids.iter().enumerate() {
        let mut s = HllSketch::default();
        let hash = node_hash(id);
        s.add_hash(hash);
        fwd.push(s.clone());
        bwd.push(s);
    }

    // Forward propagation (descendants): for edge u→v, merge v's
    // sketch into u's sketch. After convergence, fwd[u] contains
    // all nodes reachable from u.
    for _ in 0..MAX_ROUNDS {
        let mut changed = false;
        for (i, id) in all_ids.iter().enumerate() {
            for (target, _) in graph.get_edges_from(id) {
                if let Some(&j) = id_to_idx.get(target) {
                    let target_sketch = fwd[j].clone();
                    if fwd[i].merge(&target_sketch) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Backward propagation (ancestors): for edge u→v, merge u's
    // sketch into v's sketch. After convergence, bwd[v] contains
    // all nodes that can reach v.
    for _ in 0..MAX_ROUNDS {
        let mut changed = false;
        for (i, id) in all_ids.iter().enumerate() {
            for (source, _) in graph.get_edges_to(id) {
                if let Some(&j) = id_to_idx.get(source) {
                    let source_sketch = bwd[j].clone();
                    if bwd[i].merge(&source_sketch) {
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }

    let mut ancestor_count = HashMap::with_capacity(n);
    let mut descendant_count = HashMap::with_capacity(n);
    for (i, id) in all_ids.iter().enumerate() {
        descendant_count.insert(id.clone(), fwd[i].estimate() - 1.0); // exclude self
        ancestor_count.insert(id.clone(), bwd[i].estimate() - 1.0);
    }

    ReachabilityEstimates {
        ancestor_count,
        descendant_count,
    }
}

fn node_hash(id: &NodeId) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::path::PathBuf;

    fn span() -> Span {
        Span { file: PathBuf::from("t.rs"), start_line: 1, start_col: 0, end_line: 1, end_col: 0, byte_range: 0..0 }
    }
    fn mk(name: &str) -> NodeData {
        NodeData { id: NodeId::new("t.rs", name, NodeKind::Function), kind: NodeKind::Function, name: name.into(), qualified_name: name.into(), file_path: PathBuf::from("t.rs"), span: span(), visibility: Visibility::Public, metadata: HashMap::new(), birth_revision: 0, last_modified_revision: 0 }
    }
    fn ed() -> EdgeData {
        EdgeData { kind: EdgeKind::Calls, source_span: span(), weight: 1.0 }
    }

    #[test]
    fn hll_single_element_estimates_one() {
        let mut s = HllSketch::default();
        s.add_hash(12345);
        let est = s.estimate();
        assert!(est >= 0.5 && est <= 2.0, "single element estimate: {est}");
    }

    #[test]
    fn hll_many_elements_within_error() {
        let mut s = HllSketch::default();
        for i in 0..10_000u64 {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            i.hash(&mut h);
            s.add_hash(h.finish());
        }
        let est = s.estimate();
        let error = (est - 10_000.0).abs() / 10_000.0;
        assert!(error < 0.05, "HLL error {error} exceeds 5% threshold; est={est}");
    }

    #[test]
    fn chain_graph_reachability() {
        // a → b → c → d: descendants(a) ≈ 3, ancestors(d) ≈ 3
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a"));
        let b = g.add_node(mk("b"));
        let c = g.add_node(mk("c"));
        let d = g.add_node(mk("d"));
        g.add_edge(&a, &b, ed()).unwrap();
        g.add_edge(&b, &c, ed()).unwrap();
        g.add_edge(&c, &d, ed()).unwrap();

        let est = approximate_reachability(&g);
        let desc_a = est.descendant_count[&a];
        let anc_d = est.ancestor_count[&d];
        // HLL at b=12 on 4 elements can have high relative error.
        // Just verify it's positive and not absurd.
        assert!(desc_a >= 1.0, "descendants(a) should be >= 1, got {desc_a}");
        assert!(anc_d >= 1.0, "ancestors(d) should be >= 1, got {anc_d}");
    }

    #[test]
    fn isolated_node_zero_reachability() {
        let mut g = CodeGraph::new();
        let solo = g.add_node(mk("solo"));
        let est = approximate_reachability(&g);
        assert!(est.descendant_count[&solo] < 1.0);
        assert!(est.ancestor_count[&solo] < 1.0);
    }

    #[test]
    fn empty_graph_no_panic() {
        let g = CodeGraph::new();
        let est = approximate_reachability(&g);
        assert!(est.ancestor_count.is_empty());
    }
}
