//! Teleport — jump between sessions by switching git branch + loading session state.
//!
//! Combines git branch checkout with session resume to "jump into" another
//! agent's (or your own previous) work context.

use std::path::Path;
use std::process::Command;

/// Result of a teleport operation.
#[derive(Debug, Clone)]
pub struct TeleportResult {
    pub success: bool,
    pub previous_branch: String,
    pub target_branch: String,
    pub session_id: Option<String>,
    pub message: String,
}

/// Teleport to a session — switch git branch and optionally load session state.
pub fn teleport_to_session(
    repo_root: &Path,
    target_session_id: &str,
    target_branch: Option<&str>,
) -> TeleportResult {
    // 1. Get current branch (for undo)
    let previous_branch = get_current_branch(repo_root).unwrap_or_else(|| "HEAD".to_string());

    // 2. If a branch is specified, check it out
    if let Some(branch) = target_branch {
        match checkout_branch(repo_root, branch) {
            Ok(()) => {}
            Err(e) => {
                return TeleportResult {
                    success: false,
                    previous_branch,
                    target_branch: branch.to_string(),
                    session_id: Some(target_session_id.to_string()),
                    message: format!("Failed to checkout branch '{branch}': {e}"),
                };
            }
        }
    }

    let resolved_branch = target_branch
        .map(String::from)
        .unwrap_or_else(|| previous_branch.clone());

    TeleportResult {
        success: true,
        previous_branch,
        target_branch: resolved_branch,
        session_id: Some(target_session_id.to_string()),
        message: format!(
            "Teleported to session '{target_session_id}'. Use /resume {target_session_id} to load the conversation."
        ),
    }
}

/// Teleport back to the previous branch (undo a teleport).
pub fn teleport_back(repo_root: &Path, previous_branch: &str) -> TeleportResult {
    match checkout_branch(repo_root, previous_branch) {
        Ok(()) => TeleportResult {
            success: true,
            previous_branch: get_current_branch(repo_root).unwrap_or_default(),
            target_branch: previous_branch.to_string(),
            session_id: None,
            message: format!("Teleported back to branch '{previous_branch}'"),
        },
        Err(e) => TeleportResult {
            success: false,
            previous_branch: String::new(),
            target_branch: previous_branch.to_string(),
            session_id: None,
            message: format!("Failed to teleport back: {e}"),
        },
    }
}

/// List sessions that have associated branches (teleportable targets).
pub fn list_teleport_targets(repo_root: &Path) -> Vec<TeleportTarget> {
    let branches = list_jfc_branches(repo_root);
    branches
        .into_iter()
        .map(|branch| {
            let session_id = branch.strip_prefix("jfc/").map(String::from);
            TeleportTarget {
                branch: branch.clone(),
                session_id,
                is_current: false, // Caller should check
            }
        })
        .collect()
}

/// A potential teleport target.
#[derive(Debug, Clone)]
pub struct TeleportTarget {
    pub branch: String,
    pub session_id: Option<String>,
    pub is_current: bool,
}

// ─── Git helpers ────────────────────────────────────────────────────────────

fn get_current_branch(repo_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

fn checkout_branch(repo_root: &Path, branch: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(["checkout", branch])
        .current_dir(repo_root)
        .output()
        .map_err(|e| format!("git checkout failed: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn list_jfc_branches(repo_root: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["branch", "--list", "jfc/*", "--format=%(refname:short)"])
        .current_dir(repo_root)
        .output()
        .ok();

    match output {
        Some(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teleport_target_from_branch() {
        let targets = vec![TeleportTarget {
            branch: "jfc/feature-work".to_string(),
            session_id: Some("feature-work".to_string()),
            is_current: false,
        }];
        assert_eq!(targets[0].session_id.as_deref(), Some("feature-work"));
    }

    #[test]
    fn teleport_result_message() {
        let result = TeleportResult {
            success: true,
            previous_branch: "main".to_string(),
            target_branch: "jfc/test".to_string(),
            session_id: Some("test-session".to_string()),
            message: "Teleported".to_string(),
        };
        assert!(result.success);
    }
}
