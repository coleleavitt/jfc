//! Dominator tree computation. Generic over any directed graph with a
//! designated entry node.
//!
//! Pluggable into the call graph (entry = a public function) or a future
//! per-function CFG (entry = function entry block) when CFGs land. Pairs
//! with the existing `preconditions` operator and predicate extraction in
//! `crate::analysis` to answer the roadmap idiom: *what must execute before
//! this point? what exits does this call post-dominate?*
//!
//! # Forward-infrastructure caveat
//!
//! The roadmap calls for per-function CFGs as a prerequisite for "real"
//! dominator analysis. We don't have those yet — `jfc-graph` currently models
//! the **call graph** but not in-function control flow. By staying generic
//! over `petgraph::visit::*` traits, this module is ready to plug into a CFG
//! the moment one becomes available. In the meantime, callers can run it
//! against the call graph (via `CodeGraph::inner()`) for coarse-grained
//! "what callers must reach this function" answers.
//!
//! # Algorithm
//!
//! Wraps `petgraph::algo::dominators::simple_fast`, which implements the
//! Cooper-Harvey-Kennedy *Simple, Fast Dominance* algorithm[^chk]: iterative
//! data-flow over the reverse-postorder of the graph, refining `idom`
//! candidates by walking up the partially-built dominator tree. Cooper et al
//! show that in practice it converges in ~3 passes and outperforms the
//! Lengauer-Tarjan O(E·α(V)) algorithm on the realistic CFG sizes that
//! compilers see.
//!
//! Post-dominators are obtained by running the same algorithm on the reverse
//! graph (`petgraph::visit::Reversed`) seeded at the unique exit node.
//!
//! [^chk]: Cooper, Harvey, Kennedy, *A Simple, Fast Dominance Algorithm*,
//!         Rice University TR-06-33870, 2006.

use std::collections::HashMap;
use std::hash::Hash;

use petgraph::algo::dominators::simple_fast;
use petgraph::visit::{GraphBase, IntoNeighbors, IntoNeighborsDirected, Reversed, Visitable};

/// Dominator tree for a directed graph rooted at a designated entry node.
///
/// `N` is the petgraph node identifier type. For a `StableDiGraph` (what
/// [`crate::graph::CodeGraph`] wraps) that is `petgraph::stable_graph::NodeIndex`;
/// for a future CFG built on `DiGraph` it would be `petgraph::graph::NodeIndex`.
/// The struct stays generic so the same code services both.
#[derive(Debug, Clone)]
pub struct Dominators<N>
where
    N: Copy + Eq + Hash,
{
    /// `idom[node] = immediate dominator`. The entry node is intentionally
    /// **absent** from the map — by convention the root has no strict
    /// immediate dominator, and treating it as "self-dominator" would create
    /// a cycle on `dominators_chain` walks.
    idom: HashMap<N, N>,
    /// The entry node these dominators are computed from. Returned to callers
    /// so they can detect "node == root" without re-threading the value.
    root: N,
}

impl<N> Dominators<N>
where
    N: Copy + Eq + Hash,
{
    /// Compute dominators using Cooper-Harvey-Kennedy *Simple, Fast Dominance*.
    ///
    /// Generic over any directed graph that exposes the petgraph visitor
    /// traits. Concretely: a `&StableDiGraph<_, _>`, `&DiGraph<_, _>`, or any
    /// adapter (`Reversed`, `EdgeFiltered`, …) implementing
    /// `IntoNeighbors + Visitable`.
    ///
    /// O(V·E) worst case; near-linear in practice on realistic CFGs.
    pub fn build<G>(graph: G, entry: N) -> Self
    where
        G: IntoNeighbors + Visitable + GraphBase<NodeId = N>,
    {
        let pg = simple_fast(graph, entry);
        // Translate petgraph's `Dominators<N>` into our flat `idom` map. We
        // could carry petgraph's structure directly, but the `HashMap` form
        // is what slicing/taint passes will consume and it pins the API down
        // independent of petgraph internals.
        let mut idom = HashMap::new();
        // We can only see nodes by walking the dominator iterators from
        // every reachable node — petgraph doesn't expose the inner map. The
        // cleanest enumeration is via `strict_dominators` for each node we
        // encounter while walking, but petgraph doesn't expose a node list
        // either. Instead we leverage `immediate_dominator` per node, which
        // requires us to know the reachable node set up front.
        //
        // Strategy: walk the dominator chain from any node we encounter via
        // `immediately_dominated_by(root)` and recurse. This is BFS down the
        // dominator tree from the root.
        let mut frontier: Vec<N> = pg.immediately_dominated_by(entry).collect();
        while let Some(n) = frontier.pop() {
            if let Some(parent) = pg.immediate_dominator(n) {
                idom.insert(n, parent);
            }
            frontier.extend(pg.immediately_dominated_by(n));
        }

        Self { idom, root: entry }
    }

    /// Root (entry) node these dominators were computed from.
    pub fn root(&self) -> N {
        self.root
    }

    /// Immediate dominator of `node`, or `None` if `node` is the root or is
    /// unreachable from it.
    pub fn immediate_dominator(&self, node: &N) -> Option<&N> {
        self.idom.get(node)
    }

    /// Walk the dominator chain from `node` up to the root.
    ///
    /// Returns the chain in **bottom-up** order: `[idom(node), idom(idom(node)), …, root]`.
    /// The seed `node` itself is **not** included — callers asking "what must
    /// execute before this point" want the strict ancestors. If `node` is the
    /// root, returns an empty vector. If `node` is unreachable, also empty.
    pub fn dominators_chain(&self, node: &N) -> Vec<N> {
        let mut chain = Vec::new();
        let mut cursor = self.idom.get(node).copied();
        while let Some(n) = cursor {
            chain.push(n);
            // Stop at the root — it is intentionally absent from `idom` so
            // this terminates naturally, but spell it out for clarity.
            if n == self.root {
                break;
            }
            cursor = self.idom.get(&n).copied();
        }
        chain
    }

    /// True iff `a` dominates `b` (`a` lies on every path from root to `b`).
    /// A node always dominates itself (reflexive).
    pub fn dominates(&self, a: &N, b: &N) -> bool {
        if a == b {
            return true;
        }
        let mut cursor = self.idom.get(b).copied();
        while let Some(n) = cursor {
            if &n == a {
                return true;
            }
            if n == self.root {
                return &n == a;
            }
            cursor = self.idom.get(&n).copied();
        }
        false
    }
}

/// Post-dominator tree: who must execute *after* a given node on every path
/// to the exit. Computed by running the dominator algorithm on the **reverse**
/// graph rooted at the exit node.
///
/// Same internal shape as [`Dominators`]; the type-level distinction prevents
/// callers from confusing the two relations at the API boundary.
#[derive(Debug, Clone)]
pub struct PostDominators<N>
where
    N: Copy + Eq + Hash,
{
    inner: Dominators<N>,
}

impl<N> PostDominators<N>
where
    N: Copy + Eq + Hash,
{
    /// Build post-dominators by running the dominator algorithm on the
    /// reversed graph seeded at `exit`.
    ///
    /// `exit` should be a unique exit node. CFGs with multiple exits should
    /// add a synthetic super-exit node before calling this.
    pub fn build<G>(graph: G, exit: N) -> Self
    where
        // `Reversed<G>` requires `IntoNeighborsDirected` on `G` to provide
        // its `IntoNeighbors` impl; the dominator algorithm then sees the
        // graph with edges flipped.
        G: IntoNeighborsDirected + Visitable + GraphBase<NodeId = N>,
    {
        let inner = Dominators::build(Reversed(graph), exit);
        Self { inner }
    }

    pub fn exit(&self) -> N {
        self.inner.root()
    }

    pub fn immediate_post_dominator(&self, node: &N) -> Option<&N> {
        self.inner.immediate_dominator(node)
    }

    pub fn post_dominators_chain(&self, node: &N) -> Vec<N> {
        self.inner.dominators_chain(node)
    }

    pub fn post_dominates(&self, a: &N, b: &N) -> bool {
        self.inner.dominates(a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use petgraph::graph::{DiGraph, NodeIndex};

    /// Build the diamond:
    ///
    /// ```text
    ///       A
    ///      / \
    ///     B   C
    ///      \ /
    ///       D
    /// ```
    ///
    /// Returns `(graph, [A, B, C, D])` indexed in that order.
    fn diamond() -> (DiGraph<&'static str, ()>, [NodeIndex; 4]) {
        let mut g = DiGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        let d = g.add_node("D");
        g.add_edge(a, b, ());
        g.add_edge(a, c, ());
        g.add_edge(b, d, ());
        g.add_edge(c, d, ());
        (g, [a, b, c, d])
    }

    #[test]
    fn dominators_simple_diamond_normal() {
        let (g, [a, b, c, d]) = diamond();
        let dom = Dominators::build(&g, a);

        // A is the root: it has no immediate dominator.
        assert_eq!(dom.immediate_dominator(&a), None);
        // B and C are immediately dominated by A (they have only one
        // predecessor — A — so it must dominate them).
        assert_eq!(dom.immediate_dominator(&b), Some(&a));
        assert_eq!(dom.immediate_dominator(&c), Some(&a));
        // D's immediate dominator is A: D's predecessors (B, C) both
        // descend from A but do not dominate D themselves (paths exist
        // through the other branch).
        assert_eq!(dom.immediate_dominator(&d), Some(&a));

        // Reflexive + transitive checks.
        assert!(dom.dominates(&a, &b));
        assert!(dom.dominates(&a, &c));
        assert!(dom.dominates(&a, &d));
        assert!(dom.dominates(&b, &b));
        assert!(!dom.dominates(&b, &d));
        assert!(!dom.dominates(&b, &c));
    }

    #[test]
    fn dominators_chain_walks_to_entry_normal() {
        // Build a longer chain: A -> B -> C -> D (linear) so we can check
        // that the chain walks all the way back.
        let mut g = DiGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let c = g.add_node("C");
        let d = g.add_node("D");
        g.add_edge(a, b, ());
        g.add_edge(b, c, ());
        g.add_edge(c, d, ());

        let dom = Dominators::build(&g, a);

        // Chain from D: bottom-up ancestors are C, B, A.
        assert_eq!(dom.dominators_chain(&d), vec![c, b, a]);
        // Chain from B: just A.
        assert_eq!(dom.dominators_chain(&b), vec![a]);
        // Chain from root: empty (root has no strict dominators).
        assert_eq!(dom.dominators_chain(&a), Vec::<NodeIndex>::new());
    }

    #[test]
    fn post_dominators_diamond_meet_at_exit_normal() {
        let (g, [a, b, c, d]) = diamond();
        // Compute post-dominators with D as the unique exit.
        let pd = PostDominators::build(&g, d);

        // D is the exit: no immediate post-dominator.
        assert_eq!(pd.immediate_post_dominator(&d), None);
        // B and C both flow only to D, so D post-dominates them.
        assert_eq!(pd.immediate_post_dominator(&b), Some(&d));
        assert_eq!(pd.immediate_post_dominator(&c), Some(&d));
        // A's post-dominator is D: every path from A reaches D.
        assert_eq!(pd.immediate_post_dominator(&a), Some(&d));

        assert!(pd.post_dominates(&d, &a));
        assert!(pd.post_dominates(&d, &b));
        // B does NOT post-dominate A (path A -> C -> D bypasses B).
        assert!(!pd.post_dominates(&b, &a));
    }

    #[test]
    fn dominators_unreachable_node_has_no_idom_boundary() {
        // Disconnected node: should not appear in the dominator map.
        let mut g = DiGraph::new();
        let a = g.add_node("A");
        let b = g.add_node("B");
        let orphan = g.add_node("orphan");
        g.add_edge(a, b, ());

        let dom = Dominators::build(&g, a);

        assert_eq!(dom.immediate_dominator(&orphan), None);
        assert_eq!(dom.dominators_chain(&orphan), Vec::<NodeIndex>::new());
        assert!(!dom.dominates(&a, &orphan));
    }
}
