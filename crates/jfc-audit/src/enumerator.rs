use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Kind of entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum EntryPointKind {
    Test,
    PublicApi,
    Main,
    FfiExport,
    Bench,
}

impl EntryPointKind {
    /// Priority weight (higher = audited first).
    pub fn priority_weight(self) -> u32 {
        match self {
            Self::FfiExport => 100,
            Self::Main => 80,
            Self::PublicApi => 60,
            Self::Test => 20,
            Self::Bench => 10,
        }
    }
}

/// A discovered entry point in the project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryPoint {
    pub handle: String,
    pub kind: EntryPointKind,
    pub reachable_count: usize,
    pub public_signature: String,
    pub file: String,
    pub line: u32,
}

/// Trait abstracting graph queries for the enumerator.
/// The real jfc-graph can implement this later.
pub trait GraphQuery: Send + Sync {
    /// List all entry points from the code graph.
    fn entrypoints(&self) -> Result<Vec<EntryPoint>>;

    /// Count reachable nodes from a given symbol handle.
    fn reachable_count(&self, handle: &str) -> Result<usize>;
}

/// Enumerates and prioritizes entry points for auditing.
pub struct SourceEnumerator<G: GraphQuery> {
    graph: G,
}

impl<G: GraphQuery> SourceEnumerator<G> {
    pub fn new(graph: G) -> Self {
        Self { graph }
    }

    /// Enumerate all entry points from the project graph.
    pub fn enumerate(&self) -> Result<Vec<EntryPoint>> {
        self.graph.entrypoints()
    }

    /// Enumerate and sort by audit priority.
    /// Priority: FfiExport > Main > PublicApi > Test > Bench, then by reachable_count desc.
    pub fn prioritize(&self) -> Result<Vec<EntryPoint>> {
        let mut entries = self.enumerate()?;
        entries.sort_by(|a, b| {
            let weight_cmp = b.kind.priority_weight().cmp(&a.kind.priority_weight());
            if weight_cmp != std::cmp::Ordering::Equal {
                return weight_cmp;
            }
            b.reachable_count.cmp(&a.reachable_count)
        });
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockGraph {
        entries: Vec<EntryPoint>,
    }

    impl GraphQuery for MockGraph {
        fn entrypoints(&self) -> Result<Vec<EntryPoint>> {
            Ok(self.entries.clone())
        }

        fn reachable_count(&self, _handle: &str) -> Result<usize> {
            Ok(42)
        }
    }

    struct EmptyGraph;

    impl GraphQuery for EmptyGraph {
        fn entrypoints(&self) -> Result<Vec<EntryPoint>> {
            Ok(vec![])
        }

        fn reachable_count(&self, _handle: &str) -> Result<usize> {
            Ok(0)
        }
    }

    #[test]
    fn enumerate_finds_entrypoints_normal() {
        let graph = MockGraph {
            entries: vec![
                EntryPoint {
                    handle: "fn:main".to_string(),
                    kind: EntryPointKind::Main,
                    reachable_count: 50,
                    public_signature: "fn main()".to_string(),
                    file: "src/main.rs".to_string(),
                    line: 1,
                },
                EntryPoint {
                    handle: "fn:lib::process".to_string(),
                    kind: EntryPointKind::PublicApi,
                    reachable_count: 30,
                    public_signature: "pub fn process(input: &str)".to_string(),
                    file: "src/lib.rs".to_string(),
                    line: 10,
                },
                EntryPoint {
                    handle: "fn:ffi_init".to_string(),
                    kind: EntryPointKind::FfiExport,
                    reachable_count: 20,
                    public_signature: "pub extern \"C\" fn ffi_init()".to_string(),
                    file: "src/ffi.rs".to_string(),
                    line: 5,
                },
            ],
        };

        let enumerator = SourceEnumerator::new(graph);
        let entries = enumerator.enumerate().unwrap();
        assert_eq!(entries.len(), 3);

        let prioritized = enumerator.prioritize().unwrap();
        // FfiExport should be first
        assert_eq!(prioritized[0].kind, EntryPointKind::FfiExport);
        // Then Main
        assert_eq!(prioritized[1].kind, EntryPointKind::Main);
        // Then PublicApi
        assert_eq!(prioritized[2].kind, EntryPointKind::PublicApi);
    }

    #[test]
    fn enumerate_empty_crate_robust() {
        let enumerator = SourceEnumerator::new(EmptyGraph);
        let entries = enumerator.enumerate().unwrap();
        assert!(entries.is_empty());

        let prioritized = enumerator.prioritize().unwrap();
        assert!(prioritized.is_empty());
    }
}
