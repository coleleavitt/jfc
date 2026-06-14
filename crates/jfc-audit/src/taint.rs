use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

const TAINT_CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedTaintSpecs {
    cache_version: u32,
    source_fingerprint: String,
    specs: TaintSpecs,
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
    project_root: PathBuf,
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
            project_root: project_root.to_path_buf(),
            cache_path,
            specs: None,
        }
    }

    /// Load cached specs from disk or discover fresh ones.
    pub async fn discover_specs(&mut self) -> Result<&TaintSpecs> {
        let source_fingerprint = source_fingerprint(&self.project_root)?;

        // Try cache first
        if let Ok(content) = fs::read_to_string(&self.cache_path) {
            match serde_json::from_str::<CachedTaintSpecs>(&content) {
                Ok(cached)
                    if cached.cache_version == TAINT_CACHE_VERSION
                        && cached.source_fingerprint == source_fingerprint =>
                {
                    debug!("loaded taint specs from cache");
                    self.specs = Some(cached.specs);
                    return Ok(self.specs.as_ref().unwrap());
                }
                Ok(_) => {
                    debug!("cached taint specs invalidated by source fingerprint");
                }
                Err(wrapper_err) => match serde_json::from_str::<TaintSpecs>(&content) {
                    Ok(_) => {
                        debug!(
                            "legacy taint specs cache has no source fingerprint; re-discovering"
                        );
                    }
                    Err(specs_err) => {
                        warn!(
                            wrapper_error = %wrapper_err,
                            specs_error = %specs_err,
                            "cached taint specs malformed, re-discovering"
                        );
                    }
                },
            }
        }

        // Discover fresh
        let specs = self.provider.discover_specs().await?;
        self.specs = Some(specs);
        self.write_cache(source_fingerprint)?;
        Ok(self.specs.as_ref().unwrap())
    }

    fn write_cache(&self, source_fingerprint: String) -> Result<()> {
        if let Some(parent) = self.cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let specs = self.specs.as_ref().ok_or_else(|| AuditError::Internal {
            message: "cannot write taint specs cache before specs are loaded".to_string(),
        })?;
        let cached = CachedTaintSpecs {
            cache_version: TAINT_CACHE_VERSION,
            source_fingerprint,
            specs: specs.clone(),
        };
        let json = serde_json::to_string_pretty(&cached)?;
        fs::write(&self.cache_path, json).map_err(|e| AuditError::Io {
            source: e,
            context: "writing taint specs cache".to_string(),
        })?;
        Ok(())
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

fn source_fingerprint(project_root: &Path) -> Result<String> {
    let mut paths = Vec::new();
    collect_source_paths(project_root, &mut paths)?;
    paths.sort();

    let mut hasher = Sha256::new();
    for path in paths {
        let rel = path.strip_prefix(project_root).unwrap_or(&path);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update([0]);
        let bytes = fs::read(&path).map_err(|e| AuditError::Io {
            source: e,
            context: format!(
                "reading source file for taint cache fingerprint: {}",
                path.display()
            ),
        })?;
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }

    Ok(hex_encode(&hasher.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn collect_source_paths(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|e| AuditError::Io {
        source: e,
        context: format!(
            "reading directory for taint cache fingerprint: {}",
            dir.display()
        ),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| AuditError::Io {
            source: e,
            context: format!("reading directory entry under {}", dir.display()),
        })?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();

        if path.is_dir() {
            if matches!(
                file_name.as_ref(),
                ".git" | ".jfc" | ".codegraph" | "target"
            ) {
                continue;
            }
            collect_source_paths(&path, out)?;
        } else if is_fingerprint_source_file(&path) {
            out.push(path);
        }
    }

    Ok(())
}

fn is_fingerprint_source_file(path: &Path) -> bool {
    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    if matches!(file_name, "Cargo.toml" | "Cargo.lock") {
        return true;
    }
    path.extension().and_then(|s| s.to_str()) == Some("rs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
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

    struct CountingProvider {
        specs: TaintSpecs,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl TaintSpecProvider for CountingProvider {
        async fn discover_specs(&self) -> Result<TaintSpecs> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.specs.clone())
        }
    }

    fn specs_with_source(handle: &str) -> TaintSpecs {
        TaintSpecs {
            sources: vec![TaintSource {
                handle: handle.to_string(),
                param: "input".to_string(),
                kind: SourceKind::UserInput,
            }],
            sinks: vec![],
            sanitizers: vec![],
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

    #[tokio::test]
    async fn source_fingerprint_invalidates_stale_cache_robust() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn old() {}\n").unwrap();

        let first_calls = Arc::new(AtomicUsize::new(0));
        let first_provider = CountingProvider {
            specs: specs_with_source("fn:old"),
            calls: first_calls.clone(),
        };
        let mut first = TaintTracker::new(first_provider, MockTaintGraph, tmp.path());
        let loaded = first.discover_specs().await.unwrap();
        assert_eq!(loaded.sources[0].handle, "fn:old");
        assert_eq!(first_calls.load(Ordering::SeqCst), 1);

        let unchanged_calls = Arc::new(AtomicUsize::new(0));
        let unchanged_provider = CountingProvider {
            specs: specs_with_source("fn:should_not_load"),
            calls: unchanged_calls.clone(),
        };
        let mut unchanged = TaintTracker::new(unchanged_provider, MockTaintGraph, tmp.path());
        let loaded = unchanged.discover_specs().await.unwrap();
        assert_eq!(loaded.sources[0].handle, "fn:old");
        assert_eq!(
            unchanged_calls.load(Ordering::SeqCst),
            0,
            "unchanged source fingerprint should use cache"
        );

        fs::write(src_dir.join("lib.rs"), "pub fn new() {}\n").unwrap();
        let changed_calls = Arc::new(AtomicUsize::new(0));
        let changed_provider = CountingProvider {
            specs: specs_with_source("fn:new"),
            calls: changed_calls.clone(),
        };
        let mut changed = TaintTracker::new(changed_provider, MockTaintGraph, tmp.path());
        let loaded = changed.discover_specs().await.unwrap();
        assert_eq!(loaded.sources[0].handle, "fn:new");
        assert_eq!(
            changed_calls.load(Ordering::SeqCst),
            1,
            "changed source fingerprint must re-discover specs"
        );
    }
}
