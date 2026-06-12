//! Persistence for `.claude/catch-up-state.json`.
//!
//! CC 2.1.167's catch-up skill reads/writes this file to track what it was
//! monitoring between sessions. JFC provides the data layer; skill integration
//! reads/writes via the public API here.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_write::write_atomic_sync;

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

/// Load the catch-up state from `<project_root>/.claude/catch-up-state.json`.
///
/// Returns `CatchUpState::default()` when the file is absent or malformed.
pub fn load_catch_up_state(project_root: &Path) -> CatchUpState {
    let path = catch_up_state_path(project_root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return CatchUpState::default();
        }
        Err(err) => {
            tracing::warn!(
                target: "jfc::config::catch_up_state",
                path = %path.display(),
                error = %err,
                "failed to read catch-up-state.json — using default state"
            );
            return CatchUpState::default();
        }
    };
    match serde_json::from_str::<CatchUpState>(&raw) {
        Ok(state) => state,
        Err(err) => {
            tracing::warn!(
                target: "jfc::config::catch_up_state",
                path = %path.display(),
                error = %err,
                "failed to parse catch-up-state.json — using default state"
            );
            CatchUpState::default()
        }
    }
}

/// Persist the catch-up state to `<project_root>/.claude/catch-up-state.json`.
///
/// Creates `.claude/` if needed. Uses atomic write to prevent corruption.
pub fn save_catch_up_state(project_root: &Path, state: &CatchUpState) -> std::io::Result<()> {
    let path = catch_up_state_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    write_atomic_sync(&path, json.as_bytes())
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
