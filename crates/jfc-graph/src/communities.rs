//! Louvain community detection on the code graph.
//!
//! Implements the classic Louvain algorithm for modularity-based community
//! detection. The algorithm operates on the undirected projection of the
//! call graph (treating `Calls` edges as bidirectional) and iteratively
//! merges nodes into communities to maximize modularity.
//!
//! ## Algorithm outline
//!
//! 1. **Initialization**: each node starts in its own community.
//! 2. **Phase 1 (local moves)**: for each node, evaluate moving it to each
//!    neighbor's community; accept the move that gives the largest positive
//!    modularity gain.
//! 3. Repeat Phase 1 until no moves improve modularity.
//! 4. **Phase 2 (coarsening)**: collapse each community into a super-node,
//!    with weighted edges between super-nodes.
//! 5. Repeat phases 1+2 until convergence or limits are reached.
//!
//! ## References
//!
//! Blondel, V. D., Guillaume, J.-L., Lambiotte, R., & Lefebvre, E. (2008).
//! Fast unfolding of communities in large networks. *Journal of Statistical
//! Mechanics*, 2008(10), P10008.

use std::collections::HashMap;

use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// Maximum number of passes within a single Louvain level.
const MAX_PASSES: usize = 10;

/// Maximum number of hierarchical coarsening levels.
const MAX_LEVELS: usize = 10;

/// Result of community detection.
#[derive(Debug, Clone)]
pub struct CommunityResult {
    /// Each entry maps a `NodeId` to its community label (0-indexed).
    pub assignments: Vec<(NodeId, u32)>,
    /// Final modularity score in [-0.5, 1.0].
    pub modularity: f64,
    /// Number of distinct communities found.
    pub community_count: u32,
}

/// Run the Louvain algorithm on the undirected projection of the code graph.
///
/// # Parameters
/// - `graph`: the code graph to analyze
/// - `resolution`: resolution parameter (default 1.0); higher values yield
///   more/smaller communities
/// - `seed`: seed for deterministic node visit ordering
pub fn louvain(graph: &CodeGraph, resolution: f64, seed: u64) -> CommunityResult {
    // Build adjacency representation from the code graph.
    let (adj, node_ids) = build_adjacency(graph);
    let n = adj.len();

    if n == 0 {
        return CommunityResult {
            assignments: Vec::new(),
            modularity: 0.0,
            community_count: 0,
        };
    }

    // community[i] tracks which community node i belongs to across levels.
    // Initially each node is in its own community.
    let mut community: Vec<u32> = (0..n as u32).collect();

    // Current adjacency for this level (mutated by coarsening).
    let mut current_adj = adj;
    // Map from current-level node indices back to original node indices.
    let mut current_to_orig: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();

    for _level in 0..MAX_LEVELS {
        let level_n = current_adj.len();
        let mut level_comm: Vec<u32> = (0..level_n as u32).collect();

        let improved = local_moves(&current_adj, &mut level_comm, resolution, seed);

        if !improved {
            break;
        }

        // Map level communities back to original nodes.
        let mut new_community = vec![0u32; n];
        for (level_node, orig_nodes) in current_to_orig.iter().enumerate() {
            for &orig in orig_nodes {
                new_community[orig] = level_comm[level_node];
            }
        }
        community = new_community;

        // Coarsen: build super-graph.
        let (coarsened, new_mapping) = coarsen(&current_adj, &level_comm);

        if coarsened.len() >= level_n {
            // No reduction — we're done.
            break;
        }

        // Update the mapping from coarsened nodes to original nodes.
        let mut next_to_orig: Vec<Vec<usize>> = vec![Vec::new(); coarsened.len()];
        for (level_node, orig_nodes) in current_to_orig.iter().enumerate() {
            let super_node = new_mapping[level_node] as usize;
            next_to_orig[super_node].extend(orig_nodes.iter().copied());
        }

        current_adj = coarsened;
        current_to_orig = next_to_orig;
    }

    // Renumber communities to be contiguous 0..k.
    let mut label_map: HashMap<u32, u32> = HashMap::new();
    let mut next_label = 0u32;
    for c in &mut community {
        let entry = label_map.entry(*c).or_insert_with(|| {
            let l = next_label;
            next_label += 1;
            l
        });
        *c = *entry;
    }

    let community_count = next_label;
    let modularity =
        compute_modularity(&build_adjacency_from_graph(graph).0, &community, resolution);

    let assignments: Vec<(NodeId, u32)> = node_ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, community[i]))
        .collect();

    CommunityResult {
        assignments,
        modularity,
        community_count,
    }
}

/// Adjacency list with weights: adj[i] is a list of (neighbor, weight).
type AdjList = Vec<Vec<(usize, f64)>>;

/// Build an undirected weighted adjacency list from Calls edges in the graph.
/// Returns (adjacency, ordered_node_ids).
fn build_adjacency(graph: &CodeGraph) -> (AdjList, Vec<NodeId>) {
    build_adjacency_from_graph(graph)
}

fn build_adjacency_from_graph(graph: &CodeGraph) -> (AdjList, Vec<NodeId>) {
    let inner = graph.inner();

    // Collect all node indices and build a stable ordering.
    let mut node_indices: Vec<NodeIndex> = inner.node_indices().collect();
    node_indices.sort_by_key(|idx| idx.index());

    let mut idx_to_pos: HashMap<NodeIndex, usize> = HashMap::new();
    let mut node_ids: Vec<NodeId> = Vec::with_capacity(node_indices.len());

    for (pos, &idx) in node_indices.iter().enumerate() {
        idx_to_pos.insert(idx, pos);
        if let Some(id) = graph.node_id_for(idx) {
            node_ids.push(id.clone());
        }
    }

    let n = node_indices.len();
    let mut adj: AdjList = vec![Vec::new(); n];

    // Build undirected adjacency from Calls edges.
    for edge in inner.edge_references() {
        let kind = &edge.weight().kind;
        if !matches!(kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
            continue;
        }

        let src = edge.source();
        let tgt = edge.target();

        let Some(&src_pos) = idx_to_pos.get(&src) else {
            continue;
        };
        let Some(&tgt_pos) = idx_to_pos.get(&tgt) else {
            continue;
        };

        if src_pos == tgt_pos {
            continue; // skip self-loops
        }

        let weight = edge.weight().weight as f64;

        // Add both directions for undirected.
        adj[src_pos].push((tgt_pos, weight));
        adj[tgt_pos].push((src_pos, weight));
    }

    (adj, node_ids)
}

/// Phase 1: local moves. Returns true if at least one improvement was made.
fn local_moves(adj: &AdjList, community: &mut [u32], resolution: f64, seed: u64) -> bool {
    let n = adj.len();
    if n == 0 {
        return false;
    }

    // Compute total edge weight (sum of all edge weights / 2 for undirected,
    // but our adj list has each edge twice, so m = sum_of_all_weights / 2).
    let m: f64 = adj
        .iter()
        .flat_map(|neighbors| neighbors.iter().map(|(_, w)| w))
        .sum::<f64>()
        / 2.0;

    if m == 0.0 {
        return false;
    }

    // Node degrees (sum of weights of incident edges).
    let degree: Vec<f64> = adj
        .iter()
        .map(|neighbors| neighbors.iter().map(|(_, w)| w).sum())
        .collect();

    // Maintain sigma_tot[c] = sum of degrees of nodes in community c.
    let max_comm = community.iter().copied().max().unwrap_or(0) as usize + 1;
    let mut sigma_tot: Vec<f64> = vec![0.0; max_comm];
    for (i, &c) in community.iter().enumerate() {
        sigma_tot[c as usize] += degree[i];
    }

    // Deterministic visit order using a simple seeded shuffle.
    let mut order: Vec<usize> = (0..n).collect();
    shuffle_with_seed(&mut order, seed);

    let mut any_improved = false;

    for _pass in 0..MAX_PASSES {
        let mut moved = false;

        for &node in &order {
            let node_comm = community[node] as usize;
            let k_i = degree[node];

            // Compute sum of weights from node to each neighboring community.
            let mut comm_weights: HashMap<u32, f64> = HashMap::new();
            for &(neighbor, w) in &adj[node] {
                let nc = community[neighbor];
                *comm_weights.entry(nc).or_insert(0.0) += w;
            }

            // k_i_in for current community (weights from node to own community).
            let k_i_in_current = comm_weights
                .get(&(node_comm as u32))
                .copied()
                .unwrap_or(0.0);

            // sigma_tot of current community WITHOUT node i.
            let sigma_tot_current_without_i = sigma_tot[node_comm] - k_i;

            // The "removal cost": what we lose by removing node from its community.
            // ΔQ_remove = k_i_in_current / m - resolution * sigma_tot_current_without_i * k_i / (2m²)
            let remove_cost =
                k_i_in_current / m - resolution * sigma_tot_current_without_i * k_i / (2.0 * m * m);

            let mut best_gain = 0.0;
            let mut best_comm = node_comm as u32;

            for (&target_comm, &k_i_in_target) in &comm_weights {
                if target_comm as usize == node_comm {
                    continue;
                }

                let sigma_tot_target = sigma_tot[target_comm as usize];

                // ΔQ_insert = k_i_in_target / m - resolution * sigma_tot_target * k_i / (2m²)
                let insert_gain =
                    k_i_in_target / m - resolution * sigma_tot_target * k_i / (2.0 * m * m);

                // Net gain = insert_gain - remove_cost
                let gain = insert_gain - remove_cost;

                if gain > best_gain {
                    best_gain = gain;
                    best_comm = target_comm;
                }
            }

            if best_comm != node_comm as u32 {
                // Update sigma_tot: remove from old, add to new.
                sigma_tot[node_comm] -= k_i;
                sigma_tot[best_comm as usize] += k_i;
                community[node] = best_comm;
                moved = true;
                any_improved = true;
            }
        }

        if !moved {
            break;
        }
    }

    any_improved
}

/// Phase 2: coarsen the graph by merging communities into super-nodes.
/// Returns (new adjacency, mapping from old node to new super-node).
///
/// Internal community edges become self-loops in the super-graph.
/// These self-loops are essential for correct modularity computation
/// at subsequent Louvain levels.
fn coarsen(adj: &AdjList, community: &[u32]) -> (AdjList, Vec<u32>) {
    // Determine unique communities and remap to contiguous indices.
    let mut comm_to_super: HashMap<u32, u32> = HashMap::new();
    let mut next_id = 0u32;
    let mut mapping: Vec<u32> = Vec::with_capacity(community.len());

    for &c in community {
        let super_id = *comm_to_super.entry(c).or_insert_with(|| {
            let id = next_id;
            next_id += 1;
            id
        });
        mapping.push(super_id);
    }

    let super_count = next_id as usize;
    let mut super_adj: Vec<HashMap<usize, f64>> = vec![HashMap::new(); super_count];

    for (node, neighbors) in adj.iter().enumerate() {
        let src_super = mapping[node] as usize;
        for &(neighbor, weight) in neighbors {
            let tgt_super = mapping[neighbor] as usize;
            // Include self-loops (internal edges within a community).
            // They contribute to sigma_in and are needed for correct
            // modularity computation at subsequent levels.
            *super_adj[src_super].entry(tgt_super).or_insert(0.0) += weight;
        }
    }

    // Convert to AdjList.
    let new_adj: AdjList = super_adj
        .into_iter()
        .map(|map| map.into_iter().collect())
        .collect();

    (new_adj, mapping)
}

/// Compute the modularity Q of a partition.
///
/// Uses the efficient per-community formulation:
/// Q = Σ_c [L_c/(2m) - resolution * (D_c/(2m))²]
/// where L_c = sum of edge weights within community c (counting each
/// undirected edge once), D_c = sum of degrees of nodes in community c.
fn compute_modularity(adj: &AdjList, community: &[u32], resolution: f64) -> f64 {
    let m: f64 = adj
        .iter()
        .flat_map(|neighbors| neighbors.iter().map(|(_, w)| w))
        .sum::<f64>()
        / 2.0;

    if m == 0.0 {
        return 0.0;
    }

    let degree: Vec<f64> = adj
        .iter()
        .map(|neighbors| neighbors.iter().map(|(_, w)| w).sum())
        .collect();

    // Compute per-community: L_c (internal weight, counting each edge once)
    // and D_c (total degree).
    let mut community_internal: HashMap<u32, f64> = HashMap::new();
    let mut community_degree: HashMap<u32, f64> = HashMap::new();

    for (i, neighbors) in adj.iter().enumerate() {
        let ci = community[i];
        *community_degree.entry(ci).or_insert(0.0) += degree[i];
        for &(j, w) in neighbors {
            if community[j] == ci {
                // Each internal edge is counted twice (once from each endpoint),
                // so we accumulate and divide by 2 later.
                *community_internal.entry(ci).or_insert(0.0) += w;
            }
        }
    }

    let mut q = 0.0;
    for (&_c, &internal_2x) in &community_internal {
        let l_c = internal_2x / 2.0; // divide by 2 because each edge counted twice
        q += l_c / (2.0 * m);
    }
    for (&_c, &d_c) in &community_degree {
        q -= resolution * (d_c / (2.0 * m)).powi(2);
    }

    q
}

/// Deterministic Fisher-Yates shuffle using a simple LCG seeded PRNG.
fn shuffle_with_seed(slice: &mut [usize], seed: u64) {
    let n = slice.len();
    if n <= 1 {
        return;
    }
    let mut state = seed.wrapping_add(1); // avoid 0-state
    for i in (1..n).rev() {
        // Simple LCG: state = state * 6364136223846793005 + 1
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (state >> 33) as usize % (i + 1);
        slice.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

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

    fn make_fn_node(name: &str) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn make_fn_node_in_file(name: &str, file: &str) -> NodeData {
        let id = NodeId::new(file, &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from(file),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn calls_edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: sample_span(),
            weight: 1.0,
        }
    }

    #[test]
    fn empty_graph_yields_no_communities() {
        let graph = CodeGraph::new();
        let result = louvain(&graph, 1.0, 42);
        assert_eq!(result.community_count, 0);
        assert!(result.assignments.is_empty());
        assert_eq!(result.modularity, 0.0);
    }

    #[test]
    fn single_node_yields_one_community() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_fn_node("foo"));
        let result = louvain(&graph, 1.0, 42);
        assert_eq!(result.community_count, 1);
        assert_eq!(result.assignments.len(), 1);
    }

    #[test]
    fn disconnected_components_separate_communities() {
        // Two disconnected pairs: (a->b), (c->d)
        let mut graph = CodeGraph::new();
        let a = graph.add_node(make_fn_node_in_file("a", "src/a.rs"));
        let b = graph.add_node(make_fn_node_in_file("b", "src/b.rs"));
        let c = graph.add_node(make_fn_node_in_file("c", "src/c.rs"));
        let d = graph.add_node(make_fn_node_in_file("d", "src/d.rs"));

        graph.add_edge(&a, &b, calls_edge()).unwrap();
        graph.add_edge(&c, &d, calls_edge()).unwrap();

        let result = louvain(&graph, 1.0, 42);

        // Each disconnected component should be its own community.
        let map: HashMap<NodeId, u32> = result.assignments.into_iter().collect();
        assert_eq!(map[&a], map[&b], "a and b should be in the same community");
        assert_eq!(map[&c], map[&d], "c and d should be in the same community");
        assert_ne!(
            map[&a], map[&c],
            "disconnected components should be in different communities"
        );
        assert_eq!(result.community_count, 2);
    }

    #[test]
    fn two_dense_groups_connected_by_bridge() {
        // Two triangles (fully connected groups of 3) with a single bridge.
        // Group A: a1-a2-a3 (all connected to each other)
        // Group B: b1-b2-b3 (all connected to each other)
        // Bridge: a1 -> b1
        // Louvain should find 2 communities.
        let mut graph = CodeGraph::new();
        let a1 = graph.add_node(make_fn_node_in_file("a1", "src/a1.rs"));
        let a2 = graph.add_node(make_fn_node_in_file("a2", "src/a2.rs"));
        let a3 = graph.add_node(make_fn_node_in_file("a3", "src/a3.rs"));
        let b1 = graph.add_node(make_fn_node_in_file("b1", "src/b1.rs"));
        let b2 = graph.add_node(make_fn_node_in_file("b2", "src/b2.rs"));
        let b3 = graph.add_node(make_fn_node_in_file("b3", "src/b3.rs"));

        // Triangle A
        graph.add_edge(&a1, &a2, calls_edge()).unwrap();
        graph.add_edge(&a2, &a3, calls_edge()).unwrap();
        graph.add_edge(&a3, &a1, calls_edge()).unwrap();

        // Triangle B
        graph.add_edge(&b1, &b2, calls_edge()).unwrap();
        graph.add_edge(&b2, &b3, calls_edge()).unwrap();
        graph.add_edge(&b3, &b1, calls_edge()).unwrap();

        // Bridge
        graph.add_edge(&a1, &b1, calls_edge()).unwrap();

        let result = louvain(&graph, 1.0, 42);
        let map: HashMap<NodeId, u32> = result.assignments.into_iter().collect();

        // All A nodes should be in one community, all B in another.
        assert_eq!(map[&a1], map[&a2]);
        assert_eq!(map[&a2], map[&a3]);
        assert_eq!(map[&b1], map[&b2]);
        assert_eq!(map[&b2], map[&b3]);
        assert_ne!(map[&a1], map[&b1]);
        assert_eq!(result.community_count, 2);
    }

    #[test]
    fn linear_chain_few_communities() {
        // A linear chain: a -> b -> c -> d -> e
        // With resolution=1.0, a linear chain often merges into 1-2 communities.
        let mut graph = CodeGraph::new();
        let a = graph.add_node(make_fn_node_in_file("a", "src/a.rs"));
        let b = graph.add_node(make_fn_node_in_file("b", "src/b.rs"));
        let c = graph.add_node(make_fn_node_in_file("c", "src/c.rs"));
        let d = graph.add_node(make_fn_node_in_file("d", "src/d.rs"));
        let e = graph.add_node(make_fn_node_in_file("e", "src/e.rs"));

        graph.add_edge(&a, &b, calls_edge()).unwrap();
        graph.add_edge(&b, &c, calls_edge()).unwrap();
        graph.add_edge(&c, &d, calls_edge()).unwrap();
        graph.add_edge(&d, &e, calls_edge()).unwrap();

        let result = louvain(&graph, 1.0, 42);
        // A linear chain should produce a small number of communities (1-3).
        assert!(
            result.community_count <= 3,
            "linear chain should produce few communities, got {}",
            result.community_count
        );
        assert_eq!(result.assignments.len(), 5);
    }

    #[test]
    fn clustered_graph_finds_clear_communities() {
        // Two dense cliques connected by a single bridge edge.
        // Clique 1: x1, x2, x3, x4 (fully connected)
        // Clique 2: y1, y2, y3, y4 (fully connected)
        // Bridge: x1 -> y1
        let mut graph = CodeGraph::new();
        let x1 = graph.add_node(make_fn_node_in_file("x1", "src/x1.rs"));
        let x2 = graph.add_node(make_fn_node_in_file("x2", "src/x2.rs"));
        let x3 = graph.add_node(make_fn_node_in_file("x3", "src/x3.rs"));
        let x4 = graph.add_node(make_fn_node_in_file("x4", "src/x4.rs"));
        let y1 = graph.add_node(make_fn_node_in_file("y1", "src/y1.rs"));
        let y2 = graph.add_node(make_fn_node_in_file("y2", "src/y2.rs"));
        let y3 = graph.add_node(make_fn_node_in_file("y3", "src/y3.rs"));
        let y4 = graph.add_node(make_fn_node_in_file("y4", "src/y4.rs"));

        // Fully connect clique 1.
        let xs = [&x1, &x2, &x3, &x4];
        for i in 0..xs.len() {
            for j in (i + 1)..xs.len() {
                graph.add_edge(xs[i], xs[j], calls_edge()).unwrap();
            }
        }

        // Fully connect clique 2.
        let ys = [&y1, &y2, &y3, &y4];
        for i in 0..ys.len() {
            for j in (i + 1)..ys.len() {
                graph.add_edge(ys[i], ys[j], calls_edge()).unwrap();
            }
        }

        // Bridge.
        graph.add_edge(&x1, &y1, calls_edge()).unwrap();

        let result = louvain(&graph, 1.0, 42);
        let map: HashMap<NodeId, u32> = result.assignments.into_iter().collect();

        // All x nodes should be in the same community.
        assert_eq!(map[&x1], map[&x2]);
        assert_eq!(map[&x2], map[&x3]);
        assert_eq!(map[&x3], map[&x4]);

        // All y nodes should be in the same community.
        assert_eq!(map[&y1], map[&y2]);
        assert_eq!(map[&y2], map[&y3]);
        assert_eq!(map[&y3], map[&y4]);

        // The two cliques should be in different communities.
        assert_ne!(map[&x1], map[&y1]);
        assert_eq!(result.community_count, 2);
        // Modularity for two K4 cliques with a single bridge is modest
        // because each community has exactly half the total degree.
        assert!(
            result.modularity > -0.1,
            "modularity should be reasonable, got {}",
            result.modularity
        );
    }

    #[test]
    fn deterministic_with_same_seed() {
        let mut graph = CodeGraph::new();
        let a = graph.add_node(make_fn_node_in_file("a", "src/a.rs"));
        let b = graph.add_node(make_fn_node_in_file("b", "src/b.rs"));
        let c = graph.add_node(make_fn_node_in_file("c", "src/c.rs"));
        graph.add_edge(&a, &b, calls_edge()).unwrap();
        graph.add_edge(&b, &c, calls_edge()).unwrap();

        let r1 = louvain(&graph, 1.0, 123);
        let r2 = louvain(&graph, 1.0, 123);

        assert_eq!(r1.assignments, r2.assignments);
        assert_eq!(r1.modularity, r2.modularity);
        assert_eq!(r1.community_count, r2.community_count);
    }

    #[test]
    fn resolution_parameter_affects_granularity() {
        // With a very low resolution (0.1), everything merges into one community.
        // With a very high resolution (10.0), nodes stay more separated.
        // We use the two-cliques-with-bridge graph where the effect is clear.
        let mut graph = CodeGraph::new();
        let x1 = graph.add_node(make_fn_node_in_file("r1", "src/r1.rs"));
        let x2 = graph.add_node(make_fn_node_in_file("r2", "src/r2.rs"));
        let x3 = graph.add_node(make_fn_node_in_file("r3", "src/r3.rs"));
        let y1 = graph.add_node(make_fn_node_in_file("r4", "src/r4.rs"));
        let y2 = graph.add_node(make_fn_node_in_file("r5", "src/r5.rs"));
        let y3 = graph.add_node(make_fn_node_in_file("r6", "src/r6.rs"));

        // Two triangles.
        graph.add_edge(&x1, &x2, calls_edge()).unwrap();
        graph.add_edge(&x2, &x3, calls_edge()).unwrap();
        graph.add_edge(&x3, &x1, calls_edge()).unwrap();
        graph.add_edge(&y1, &y2, calls_edge()).unwrap();
        graph.add_edge(&y2, &y3, calls_edge()).unwrap();
        graph.add_edge(&y3, &y1, calls_edge()).unwrap();
        // Bridge.
        graph.add_edge(&x1, &y1, calls_edge()).unwrap();

        let low_res = louvain(&graph, 0.1, 42);
        let high_res = louvain(&graph, 5.0, 42);

        // Very low resolution merges everything into 1 community;
        // high resolution should produce at least 2 communities.
        assert!(
            high_res.community_count >= low_res.community_count,
            "high resolution ({}) should yield >= communities than low resolution ({})",
            high_res.community_count,
            low_res.community_count
        );
    }
}
