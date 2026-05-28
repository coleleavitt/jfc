use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{AuditError, Result};
use crate::types::TaintHop;

/// Kind of taint source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    UserInput,
    NetworkData,
    FileData,
    EnvironmentVariable,
    CommandLineArg,
}

/// Kind of taint sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SinkKind {
    CommandExecution,
    SqlQuery,
    FileWrite,
    NetworkSend,
    MemoryAllocation,
    UnsafePointer,
}

/// A taint source in the code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintSource {
    pub handle: String,
    pub param: String,
    pub kind: SourceKind,
}

/// A taint sink in the code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintSink {
    pub handle: String,
    pub param: String,
    pub kind: SinkKind,
}

/// A sanitizer that breaks a taint chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sanitizer {
    pub handle: String,
    pub kind: SinkKind,
}

/// Complete taint specification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintSpecs {
    pub sources: Vec<TaintSource>,
    pub sinks: Vec<TaintSink>,
    pub sanitizers: Vec<Sanitizer>,
}

/// A traced taint chain from source to sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaintChain {
    pub source: TaintSource,
    pub sink: TaintSink,
    pub hops: Vec<TaintHop>,
    pub sanitized: bool,
}

/// Provider trait for discovering taint specifications (e.g. via LLM analysis).
#[async_trait]
pub trait TaintSpecProvider: Send + Sync {
    /// Discover taint sources, sinks, and sanitizers from the codebase.
    async fn discover_specs(&self) -> Result<TaintSpecs>;
}

/// Trait for tracing taint through the code graph.
pub trait TaintGraph: Send + Sync {
    /// Trace taint from a source parameter through the call graph.
    /// Returns hops until a sink or sanitizer is reached.
    fn trace(&self, source_handle: &str, source_param: &str) -> Result<Vec<TaintHop>>;
}

/// Tracks taint flow through the program.
pub struct TaintTracker<P: TaintSpecProvider, G: TaintGraph> {
    provider: P,
    graph: G,
    cache_path: PathBuf,
    specs: Option<TaintSpecs>,
}

impl<P: TaintSpecProvider, G: TaintGraph> TaintTracker<P, G> {
    pub fn new(provider: P, graph: G, project_root: &Path) -> Self {
        let cache_path = project_root
            .join(".jfc")
            .join("audit")
            .join("taint_specs.json");
        Self {
            provider,
            graph,
            cache_path,
            specs: None,
        }
    }

    /// Load cached specs from disk or discover fresh ones.
    pub async fn discover_specs(&mut self) -> Result<&TaintSpecs> {
        // Try cache first
        if let Ok(content) = fs::read_to_string(&self.cache_path) {
            match serde_json::from_str::<TaintSpecs>(&content) {
                Ok(specs) => {
                    debug!("loaded taint specs from cache");
                    self.specs = Some(specs);
                    return Ok(self.specs.as_ref().unwrap());
                }
                Err(e) => {
                    warn!(error = %e, "cached taint specs malformed, re-discovering");
                }
            }
        }

        // Discover fresh
        let specs = self.provider.discover_specs().await?;

        // Cache to disk
        if let Some(parent) = self.cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&specs)?;
        fs::write(&self.cache_path, json).map_err(|e| AuditError::Io {
            source: e,
            context: "writing taint specs cache".to_string(),
        })?;

        self.specs = Some(specs);
        Ok(self.specs.as_ref().unwrap())
    }

    /// Trace all taint chains from known sources to sinks.
    pub fn trace(&self) -> Result<Vec<TaintChain>> {
        let specs = self.specs.as_ref().ok_or_else(|| AuditError::Internal {
            message: "specs not loaded; call discover_specs() first".to_string(),
        })?;

        let mut chains = Vec::new();

        for source in &specs.sources {
            let hops = self.graph.trace(&source.handle, &source.param)?;

            if hops.is_empty() {
                continue;
            }

            // Check if any hop reaches a known sink
            for sink in &specs.sinks {
                let reaches_sink = hops.iter().any(|h| h.to_symbol == sink.handle);
                if reaches_sink {
                    // Check if sanitized
                    let sanitized = hops.iter().any(|h| {
                        specs
                            .sanitizers
                            .iter()
                            .any(|s| s.handle == h.to_symbol && s.kind == sink.kind)
                    });

                    chains.push(TaintChain {
                        source: source.clone(),
                        sink: sink.clone(),
                        hops: hops.clone(),
                        sanitized,
                    });
                }
            }
        }

        Ok(chains)
    }

    /// Get current specs (if loaded).
    pub fn specs(&self) -> Option<&TaintSpecs> {
        self.specs.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct MockProvider {
        specs: TaintSpecs,
    }

    #[async_trait]
    impl TaintSpecProvider for MockProvider {
        async fn discover_specs(&self) -> Result<TaintSpecs> {
            Ok(self.specs.clone())
        }
    }

    struct MockTaintGraph;

    impl TaintGraph for MockTaintGraph {
        fn trace(&self, source_handle: &str, _source_param: &str) -> Result<Vec<TaintHop>> {
            if source_handle == "fn:read_input" {
                Ok(vec![
                    TaintHop {
                        from_symbol: "fn:read_input".to_string(),
                        to_symbol: "fn:process".to_string(),
                        edge_kind: "call".to_string(),
                        transforms: vec![],
                    },
                    TaintHop {
                        from_symbol: "fn:process".to_string(),
                        to_symbol: "fn:execute_cmd".to_string(),
                        edge_kind: "call".to_string(),
                        transforms: vec![],
                    },
                ])
            } else {
                Ok(vec![])
            }
        }
    }

    #[tokio::test]
    async fn trace_finds_chain_normal() {
        let tmp = TempDir::new().unwrap();
        let specs = TaintSpecs {
            sources: vec![TaintSource {
                handle: "fn:read_input".to_string(),
                param: "input".to_string(),
                kind: SourceKind::UserInput,
            }],
            sinks: vec![TaintSink {
                handle: "fn:execute_cmd".to_string(),
                param: "cmd".to_string(),
                kind: SinkKind::CommandExecution,
            }],
            sanitizers: vec![],
        };

        let provider = MockProvider {
            specs: specs.clone(),
        };
        let graph = MockTaintGraph;

        let mut tracker = TaintTracker::new(provider, graph, tmp.path());
        tracker.discover_specs().await.unwrap();

        let chains = tracker.trace().unwrap();
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].source.handle, "fn:read_input");
        assert_eq!(chains[0].sink.handle, "fn:execute_cmd");
        assert!(!chains[0].sanitized);
        assert_eq!(chains[0].hops.len(), 2);
    }

    #[tokio::test]
    async fn malformed_specs_graceful_robust() {
        let tmp = TempDir::new().unwrap();
        let audit_dir = tmp.path().join(".jfc").join("audit");
        fs::create_dir_all(&audit_dir).unwrap();

        // Write malformed cache
        fs::write(audit_dir.join("taint_specs.json"), "not valid json {{{").unwrap();

        let specs = TaintSpecs {
            sources: vec![],
            sinks: vec![],
            sanitizers: vec![],
        };
        let provider = MockProvider { specs };
        let graph = MockTaintGraph;

        let mut tracker = TaintTracker::new(provider, graph, tmp.path());
        // Should recover from malformed cache and re-discover
        let result = tracker.discover_specs().await;
        assert!(result.is_ok());
        let loaded_specs = result.unwrap();
        assert!(loaded_specs.sources.is_empty());
    }
}
