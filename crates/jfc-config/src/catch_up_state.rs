//! DB persistence for Claude-compatible catch-up state.
//!
//! CC 2.1.167's catch-up skill reads/writes this file to track what it was
//! monitoring between sessions. JFC provides the data layer; skill integration
//! reads/writes via the public API here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const CATCH_UP_KIND: &str = "catch_up_state";
const CATCH_UP_KEY: &str = "state";

/// Canonical path to the catch-up state file.
pub fn catch_up_state_path(project_root: &Path) -> PathBuf {
    project_root.join(".claude").join("catch-up-state.json")
}

/// A single item the catch-up skill was tracking.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TrackedItem {
    /// Stable identifier for this item (e.g. `"pr/123"`, `"branch/feature-x"`).
    pub id: String,
    /// Category of item (e.g. `"pr"`, `"branch"`, `"issue"`, `"ci"`).
    pub kind: String,
    /// Human-readable description of what is being tracked.
    pub description: String,
    /// Epoch-ms timestamp of the last time this item was checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_ms: Option<u64>,
}

/// Full catch-up skill state persisted across sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CatchUpState {
    /// Items currently being tracked.
    #[serde(default)]
    pub tracked_items: Vec<TrackedItem>,
    /// Epoch-ms timestamp of the last catch-up run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_ms: Option<u64>,
    /// User's configured catch-up hours (e.g. `"9-17"` for 9am–5pm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catch_up_hours: Option<String>,
    /// Arbitrary extra metadata stored by the skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

fn project_session_id(project_root: &Path) -> String {
    format!("project:{}", jfc_knowledge::project_key(project_root))
}

fn project_store(project_root: &Path) -> std::io::Result<jfc_knowledge::KnowledgeStore> {
    let db_path = project_root.join(".jfc").join("knowledge.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open(&db_path))
        .map_err(std::io::Error::other)
}

/// Load the catch-up state from the project DB.
///
/// Returns `CatchUpState::default()` when no row exists or the row is malformed.
pub fn load_catch_up_state(project_root: &Path) -> CatchUpState {
    let Ok(store) = project_store(project_root) else {
        return CatchUpState::default();
    };
    if let Ok(Some(row)) = jfc_knowledge::block_on_knowledge(async {
        store
            .get_session_artifact(
                &project_session_id(project_root),
                CATCH_UP_KIND,
                CATCH_UP_KEY,
            )
            .await
    }) {
        return serde_json::from_str::<CatchUpState>(&row.value_json).unwrap_or_default();
    }
    let path = catch_up_state_path(project_root);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return CatchUpState::default();
    };
    let Ok(state) = serde_json::from_str::<CatchUpState>(&raw) else {
        return CatchUpState::default();
    };
    let _ = save_catch_up_state(project_root, &state);
    state
}

/// Persist the catch-up state to the project DB.
pub fn save_catch_up_state(project_root: &Path, state: &CatchUpState) -> std::io::Result<()> {
    let json = serde_json::to_string(state)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    let store = project_store(project_root)?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .upsert_session_artifact(
                &project_session_id(project_root),
                CATCH_UP_KIND,
                CATCH_UP_KEY,
                &json,
            )
            .await
    })
    .map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_save_load_normal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let state = CatchUpState {
            tracked_items: vec![TrackedItem {
                id: "pr/42".to_owned(),
                kind: "pr".to_owned(),
                description: "Fix the thing".to_owned(),
                last_checked_ms: Some(1_000_000),
            }],
            last_run_ms: Some(2_000_000),
            catch_up_hours: Some("9-17".to_owned()),
            extra: None,
        };
        save_catch_up_state(root, &state).unwrap();
        let loaded = load_catch_up_state(root);
        assert_eq!(loaded, state);
    }

    #[test]
    fn load_missing_returns_default_robust() {
        let dir = tempfile::tempdir().unwrap();
        let state = load_catch_up_state(dir.path());
        assert_eq!(state, CatchUpState::default());
        assert!(state.tracked_items.is_empty());
    }
}
