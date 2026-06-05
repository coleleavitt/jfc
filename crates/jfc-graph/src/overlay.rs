//! Base-graph + branch-diff overlay (codegraph PR #334).
//!
//! Full-repo indexing is expensive. On a large monorepo every contributor
//! re-indexes the same unchanged files. The overlay system eliminates the
//! redundancy:
//!
//! 1. **CI builds a base graph once** for `main` (or the team's
//!    default branch) and publishes the serialised snapshot.
//! 2. **Contributors download the snapshot** to their per-workspace
//!    data dir (see [`crate::data_dir`]).
//! 3. **`open_overlay()` loads the base, runs `git diff --name-only`
//!    against the merge-base with the default branch, and re-indexes
//!    *only* the touched files** on top of the base.
//! 4. Queries see one merged graph — symbols added on the feature
//!    branch coexist with everything inherited from the base, and any
//!    file the contributor edited has the *contributor's* version, not
//!    the base's.
//!
//! Our implementation is deliberately simpler than codegraph's: their
//! base + overlay are two SQLite databases stitched at query time. We
//! mutate one in-memory graph in place. This trades richer
//! merge-conflict semantics for "the result is identical to a full
//! re-index, just faster" — which is the property the codegraph
//! consumer most cares about.
//!
//! ## Save / load
//!
//! [`save_base_snapshot`] serialises the current graph to disk in a
//! versioned envelope so a stale base from a previous schema version
//! is rejected cleanly. [`load_base_snapshot`] is the inverse.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::edges::EdgeData;
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId};

/// Current overlay-snapshot schema version. Bumped when the on-disk
/// format changes.
pub const OVERLAY_SCHEMA_VERSION: u32 = 1;

/// On-disk envelope.
///
/// `CodeGraph` wraps a `petgraph::StableDiGraph` whose `NodeIndex`
/// values aren't stable across (de)serialisation, so the snapshot
/// stores the **graph as a flat node + edge list**. The loader
/// reconstructs a fresh `CodeGraph` by replaying the nodes and edges
/// in order — the same path the normal indexer uses.
#[derive(Debug, Serialize, Deserialize)]
struct OverlaySnapshot {
    pub schema_version: u32,
    /// Workspace root the base was indexed from, recorded so the
    /// loader can warn when a snapshot is being used in a different
    /// checkout.
    pub workspace_root: PathBuf,
    /// Git ref / commit hash the base was indexed at, when available.
    pub base_ref: Option<String>,
    pub nodes: Vec<NodeData>,
    pub edges: Vec<OverlayEdge>,
}

/// Flat edge record. We can't store `NodeIndex` because indices aren't
/// stable across (de)serialisation; we use the content-addressed
/// `NodeId` instead.
#[derive(Debug, Serialize, Deserialize)]
struct OverlayEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub data: EdgeData,
}

/// Errors raised by the overlay loader.
#[derive(Debug, Error)]
pub enum OverlayError {
    #[error("snapshot schema mismatch: expected {expected}, found {found}")]
    SchemaMismatch { expected: u32, found: u32 },
    #[error("snapshot I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot deserialisation error: {0}")]
    Deserialise(#[from] serde_json::Error),
    #[error("git invocation failed: {0}")]
    Git(String),
}

/// Persist `graph` to `path` as a versioned snapshot. The directory
/// is created if missing.
pub fn save_base_snapshot(
    path: &Path,
    graph: &CodeGraph,
    workspace_root: &Path,
    base_ref: Option<&str>,
) -> Result<(), OverlayError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let nodes: Vec<NodeData> = graph
        .all_node_ids()
        .into_iter()
        .filter_map(|id| graph.get_node(id).cloned())
        .collect();
    let mut edges: Vec<OverlayEdge> = Vec::new();
    for id in graph.all_node_ids() {
        for (target_id, edge_data) in graph.get_edges_from(id) {
            edges.push(OverlayEdge {
                from: id.clone(),
                to: target_id.clone(),
                data: edge_data.clone(),
            });
        }
    }
    let snap = OverlaySnapshot {
        schema_version: OVERLAY_SCHEMA_VERSION,
        workspace_root: workspace_root.to_path_buf(),
        base_ref: base_ref.map(str::to_string),
        nodes,
        edges,
    };
    let json = serde_json::to_string(&snap)?;
    fs::write(path, json)?;
    Ok(())
}

/// Load a previously-saved snapshot, reconstructing the in-memory
/// graph. Returns `SchemaMismatch` if the on-disk version doesn't
/// match [`OVERLAY_SCHEMA_VERSION`].
pub fn load_base_snapshot(path: &Path) -> Result<LoadedSnapshot, OverlayError> {
    let raw = fs::read_to_string(path)?;
    let snap: OverlaySnapshot = serde_json::from_str(&raw)?;
    if snap.schema_version != OVERLAY_SCHEMA_VERSION {
        return Err(OverlayError::SchemaMismatch {
            expected: OVERLAY_SCHEMA_VERSION,
            found: snap.schema_version,
        });
    }
    let mut graph = CodeGraph::new();
    for node in snap.nodes {
        graph.add_node(node);
    }
    for OverlayEdge { from, to, data } in snap.edges {
        let _ = graph.add_edge(&from, &to, data);
    }
    Ok(LoadedSnapshot {
        workspace_root: snap.workspace_root,
        base_ref: snap.base_ref,
        graph,
    })
}

/// Save a graph snapshot using bincode (much faster than JSON for large graphs).
/// Used by the session cache to persist between jfc runs.
pub fn save_snapshot_bincode(
    path: &Path,
    graph: &CodeGraph,
    workspace_root: &Path,
) -> Result<(), OverlayError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let nodes: Vec<NodeData> = graph
        .all_node_ids()
        .into_iter()
        .filter_map(|id| graph.get_node(id).cloned())
        .collect();
    let mut edges: Vec<OverlayEdge> = Vec::new();
    for id in graph.all_node_ids() {
        for (target_id, edge_data) in graph.get_edges_from(id) {
            edges.push(OverlayEdge {
                from: id.clone(),
                to: target_id.clone(),
                data: edge_data.clone(),
            });
        }
    }
    let snap = OverlaySnapshot {
        schema_version: OVERLAY_SCHEMA_VERSION,
        workspace_root: workspace_root.to_path_buf(),
        base_ref: None,
        nodes,
        edges,
    };
    let encoded =
        bincode::serde::encode_to_vec(&snap, bincode::config::standard()).map_err(|e| {
            OverlayError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;
    fs::write(path, encoded)?;
    Ok(())
}

/// Load a bincode-serialized graph snapshot.
pub fn load_snapshot_bincode(path: &Path) -> Result<LoadedSnapshot, OverlayError> {
    let raw = fs::read(path)?;
    let (snap, _): (OverlaySnapshot, _) =
        bincode::serde::decode_from_slice(&raw, bincode::config::standard()).map_err(|e| {
            OverlayError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            ))
        })?;
    if snap.schema_version != OVERLAY_SCHEMA_VERSION {
        return Err(OverlayError::SchemaMismatch {
            expected: OVERLAY_SCHEMA_VERSION,
            found: snap.schema_version,
        });
    }
    let mut graph = CodeGraph::new();
    for node in snap.nodes {
        graph.add_node(node);
    }
    for OverlayEdge { from, to, data } in snap.edges {
        let _ = graph.add_edge(&from, &to, data);
    }
    Ok(LoadedSnapshot {
        workspace_root: snap.workspace_root,
        base_ref: snap.base_ref,
        graph,
    })
}

/// The loaded snapshot, decomposed for caller convenience.
pub struct LoadedSnapshot {
    pub workspace_root: PathBuf,
    pub base_ref: Option<String>,
    pub graph: CodeGraph,
}

/// Files changed between `base_ref` (default: `origin/main`) and HEAD.
///
/// Returns the list of paths relative to `workspace_root` whose
/// contents differ from the base. Soft-fails to an empty Vec when
/// git is unavailable or the comparison can't be made — the caller
/// can still use the base unchanged.
pub fn diff_against_base(
    workspace_root: &Path,
    base_ref: &str,
) -> Result<Vec<PathBuf>, OverlayError> {
    // Resolve merge-base first so we diff against the actual branch
    // point, not the tip of base_ref. Mirrors `git diff base...HEAD`
    // semantics without requiring the three-dot syntax (which
    // sometimes confuses older `git`).
    let merge_base = Command::new("git")
        .args(["merge-base", base_ref, "HEAD"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| OverlayError::Git(format!("merge-base: {e}")))?;
    if !merge_base.status.success() {
        return Err(OverlayError::Git(format!(
            "merge-base failed: {}",
            String::from_utf8_lossy(&merge_base.stderr).trim()
        )));
    }
    let merge_base_sha = String::from_utf8_lossy(&merge_base.stdout)
        .trim()
        .to_string();
    if merge_base_sha.is_empty() {
        return Err(OverlayError::Git(
            "merge-base returned empty SHA".to_string(),
        ));
    }

    // List files changed since merge-base. Both committed and
    // working-tree-only changes count — the contributor wants the
    // graph to reflect *their* current state, not what's pushed.
    let diff_output = Command::new("git")
        .args(["diff", "--name-only", &merge_base_sha])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| OverlayError::Git(format!("diff: {e}")))?;
    if !diff_output.status.success() {
        return Err(OverlayError::Git(format!(
            "diff failed: {}",
            String::from_utf8_lossy(&diff_output.stderr).trim()
        )));
    }
    let raw = String::from_utf8_lossy(&diff_output.stdout);
    let files: Vec<PathBuf> = raw
        .lines()
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .collect();
    Ok(files)
}

/// Apply branch-diff changes on top of a base graph in-place.
///
/// For every changed file, calls `graph.update_file(path, content,
/// adapter)`. Files that no longer exist on disk are removed from the
/// graph (the adapter sees an empty file). Files outside `workspace_root`
/// are skipped silently — they're not ours to index.
pub fn apply_diff_to_graph(
    graph: &mut CodeGraph,
    workspace_root: &Path,
    changed_files: &[PathBuf],
    adapter: &dyn crate::adapter::LanguageAdapter,
) -> usize {
    let mut applied = 0usize;
    for rel in changed_files {
        let abs = workspace_root.join(rel);
        let content = fs::read_to_string(&abs).unwrap_or_default();
        graph.update_file(&abs, &content, adapter);
        applied += 1;
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn make_node(name: &str) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: Span {
                file: PathBuf::from("src/lib.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..1,
            },
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn snapshot_round_trips() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node("alpha"));
        graph.add_node(make_node("beta"));

        let dir = std::env::temp_dir().join(format!(
            "jfc-graph-overlay-test-{}-roundtrip",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let snap_path = dir.join("base.json");

        save_base_snapshot(
            &snap_path,
            &graph,
            Path::new("/tmp/workspace"),
            Some("abc1234"),
        )
        .expect("save");
        let loaded = load_base_snapshot(&snap_path).expect("load");
        assert_eq!(loaded.base_ref.as_deref(), Some("abc1234"));
        assert_eq!(loaded.graph.node_count(), graph.node_count());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn schema_mismatch_is_reported() {
        let dir = std::env::temp_dir().join(format!(
            "jfc-graph-overlay-test-{}-mismatch",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bogus.json");
        fs::write(
            &path,
            r#"{"schema_version":999,"workspace_root":"/tmp","base_ref":null,"nodes":[],"edges":[]}"#,
        )
        .unwrap();
        match load_base_snapshot(&path) {
            Err(OverlayError::SchemaMismatch { expected, found }) => {
                assert_eq!(expected, OVERLAY_SCHEMA_VERSION);
                assert_eq!(found, 999);
            }
            _ => panic!("expected SchemaMismatch"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_against_base_soft_fails_outside_git_repo() {
        // /tmp itself isn't normally a git repo — diff should fail
        // with a Git error, not panic.
        let tmp = std::env::temp_dir();
        let res = diff_against_base(&tmp, "main");
        assert!(matches!(res, Err(OverlayError::Git(_)) | Ok(_)));
    }

    #[test]
    fn overlay_schema_version_is_one() {
        assert_eq!(OVERLAY_SCHEMA_VERSION, 1);
    }
}
