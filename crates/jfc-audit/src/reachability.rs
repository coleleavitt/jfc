use serde::{Deserialize, Serialize};

use crate::enumerator::EntryPointKind;
use crate::error::Result;

/// A proof that a target is reachable from an entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityProof {
    /// Ordered path of symbol handles from entry to target.
    pub path: Vec<String>,
    /// Preconditions that must hold along the path (if/match guards).
    pub preconditions: Vec<String>,
    /// Kind of entry point the path starts from.
    pub entrypoint_kind: EntryPointKind,
    /// Depth (number of edges) from entry to target.
    pub depth: usize,
}

/// Trait for graph-based reachability queries.
pub trait ReachabilityGraph: Send + Sync {
    /// Find a path from any entry point to the target symbol.
    /// Returns the path as an ordered list of symbol handles, or None if unreachable.
    fn find_path_to(&self, target: &str) -> Result<Option<Vec<String>>>;

    /// Find a path with preconditions (if/match guards along the way).
    fn find_path_with_preconditions(
        &self,
        target: &str,
    ) -> Result<Option<(Vec<String>, Vec<String>, EntryPointKind)>>;
}

/// Proves reachability of targets from entry points using graph queries.
pub struct ReachabilityProver<G: ReachabilityGraph> {
    graph: G,
}

impl<G: ReachabilityGraph> ReachabilityProver<G> {
    pub fn new(graph: G) -> Self {
        Self { graph }
    }

    /// Prove that a target symbol is reachable from some entry point.
    /// Returns None if no path exists (target is dead code).
    pub fn prove(&self, target: &str) -> Result<Option<Vec<String>>> {
        self.graph.find_path_to(target)
    }

    /// Prove reachability with full precondition extraction.
    pub fn prove_with_preconditions(&self, target: &str) -> Result<Option<ReachabilityProof>> {
        match self.graph.find_path_with_preconditions(target)? {
            Some((path, preconditions, entrypoint_kind)) => {
                let depth = path.len().saturating_sub(1);
                Ok(Some(ReachabilityProof {
                    path,
                    preconditions,
                    entrypoint_kind,
                    depth,
                }))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockReachableGraph;

    impl ReachabilityGraph for MockReachableGraph {
        fn find_path_to(&self, target: &str) -> Result<Option<Vec<String>>> {
            Ok(Some(vec![
                "fn:main".to_string(),
                "fn:process".to_string(),
                target.to_string(),
            ]))
        }

        fn find_path_with_preconditions(
            &self,
            target: &str,
        ) -> Result<Option<(Vec<String>, Vec<String>, EntryPointKind)>> {
            Ok(Some((
                vec![
                    "fn:main".to_string(),
                    "fn:process".to_string(),
                    target.to_string(),
                ],
                vec!["input.len() > 0".to_string()],
                EntryPointKind::Main,
            )))
        }
    }

    struct MockUnreachableGraph;

    impl ReachabilityGraph for MockUnreachableGraph {
        fn find_path_to(&self, _target: &str) -> Result<Option<Vec<String>>> {
            Ok(None)
        }

        fn find_path_with_preconditions(
            &self,
            _target: &str,
        ) -> Result<Option<(Vec<String>, Vec<String>, EntryPointKind)>> {
            Ok(None)
        }
    }

    #[test]
    fn reachable_target_returns_path_normal() {
        let prover = ReachabilityProver::new(MockReachableGraph);

        let path = prover.prove("fn:vulnerable_sink").unwrap();
        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], "fn:main");
        assert_eq!(path[2], "fn:vulnerable_sink");

        let proof = prover
            .prove_with_preconditions("fn:vulnerable_sink")
            .unwrap();
        assert!(proof.is_some());
        let proof = proof.unwrap();
        assert_eq!(proof.depth, 2);
        assert_eq!(proof.entrypoint_kind, EntryPointKind::Main);
        assert!(!proof.preconditions.is_empty());
    }

    #[test]
    fn unreachable_target_returns_none_robust() {
        let prover = ReachabilityProver::new(MockUnreachableGraph);

        let path = prover.prove("fn:dead_code").unwrap();
        assert!(path.is_none());

        let proof = prover.prove_with_preconditions("fn:dead_code").unwrap();
        assert!(proof.is_none());
    }
}
