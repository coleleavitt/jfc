//! Permission synchronization between swarm workers and the team leader.
//!
//! When a worker agent encounters a tool that requires user approval, it sends
//! a permission request to the leader's mailbox. The leader presents the
//! request to the user, collects approval/rejection, and sends a response
//! back to the worker's mailbox.
//!
//! File layout:
//! ```text
//! ~/.claude/teams/{team}/permissions/
//!   pending/    — requests awaiting leader decision
//!   resolved/   — completed decisions (worker polls these)
//! ```

use std::path::PathBuf;
use std::time::Duration;

use tokio::fs;
use tracing::debug;

use super::mailbox;
use super::types::*;

// ─── Path helpers ────────────────────────────────────────────────────────────

fn permission_dir(team_name: &str) -> PathBuf {
    mailbox::team_dir(team_name).join("permissions")
}

fn pending_dir(team_name: &str) -> PathBuf {
    permission_dir(team_name).join("pending")
}

fn resolved_dir(team_name: &str) -> PathBuf {
    permission_dir(team_name).join("resolved")
}

async fn ensure_permission_dirs(team_name: &str) -> anyhow::Result<()> {
    fs::create_dir_all(pending_dir(team_name)).await?;
    fs::create_dir_all(resolved_dir(team_name)).await?;
    Ok(())
}

// ─── Request ID generation ───────────────────────────────────────────────────

/// Generate a unique permission request ID.
pub fn generate_request_id() -> String {
    format!(
        "perm-{}-{}",
        super::team_helpers::now_millis(),
        &uuid::Uuid::new_v4().to_string()[..7]
    )
}

// ─── Create requests ─────────────────────────────────────────────────────────

/// Create a new permission request.
pub fn create_permission_request(
    tool_name: &str,
    tool_use_id: &str,
    input: serde_json::Value,
    description: &str,
    worker_id: &str,
    worker_name: &str,
    worker_color: Option<&str>,
    team_name: &str,
) -> SwarmPermissionRequest {
    SwarmPermissionRequest {
        id: generate_request_id(),
        worker_id: worker_id.to_owned(),
        worker_name: worker_name.to_owned(),
        worker_color: worker_color.map(str::to_owned),
        team_name: team_name.to_owned(),
        tool_name: tool_name.to_owned(),
        tool_use_id: tool_use_id.to_owned(),
        description: description.to_owned(),
        input,
        permission_suggestions: Vec::new(),
        status: PermissionRequestStatus::Pending,
        resolved_by: None,
        resolved_at: None,
        feedback: None,
        updated_input: None,
        permission_updates: Vec::new(),
        created_at: super::team_helpers::now_millis(),
    }
}

// ─── Write / Read requests ───────────────────────────────────────────────────

/// Write a pending permission request to disk.
pub async fn write_permission_request(
    request: &SwarmPermissionRequest,
) -> anyhow::Result<()> {
    ensure_permission_dirs(&request.team_name).await?;
    let path = pending_dir(&request.team_name).join(format!("{}.json", request.id));
    let json = serde_json::to_string_pretty(request)?;
    fs::write(&path, json).await?;
    debug!(
        "[PermissionSync] Wrote pending request {} from {} for {}",
        request.id, request.worker_name, request.tool_name
    );
    Ok(())
}

/// Read all pending permission requests for a team.
pub async fn read_pending_permissions(team_name: &str) -> Vec<SwarmPermissionRequest> {
    let dir = pending_dir(team_name);
    let mut entries = match fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(request) = serde_json::from_str::<SwarmPermissionRequest>(&content) {
                    results.push(request);
                }
            }
        }
    }

    results.sort_by_key(|r| r.created_at);
    results
}

/// Resolve a permission request (move from pending to resolved).
pub async fn resolve_permission(
    request_id: &str,
    resolution: &PermissionResolution,
    team_name: &str,
) -> anyhow::Result<bool> {
    ensure_permission_dirs(team_name).await?;

    let pending_path = pending_dir(team_name).join(format!("{request_id}.json"));
    let resolved_path = resolved_dir(team_name).join(format!("{request_id}.json"));

    // Read the pending request
    let content = match fs::read_to_string(&pending_path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("[PermissionSync] Pending request not found: {request_id}");
            return Ok(false);
        }
        Err(e) => return Err(e.into()),
    };

    let mut request: SwarmPermissionRequest = serde_json::from_str(&content)?;

    // Update with resolution
    request.status = match resolution.decision {
        PermissionDecision::Approved => PermissionRequestStatus::Approved,
        PermissionDecision::Rejected => PermissionRequestStatus::Rejected,
    };
    request.resolved_by = Some(resolution.resolved_by.clone());
    request.resolved_at = Some(super::team_helpers::now_millis());
    request.feedback = resolution.feedback.clone();
    request.updated_input = resolution.updated_input.clone();
    request.permission_updates = resolution.permission_updates.clone();

    // Write to resolved, remove from pending
    let json = serde_json::to_string_pretty(&request)?;
    fs::write(&resolved_path, json).await?;
    fs::remove_file(&pending_path).await?;

    debug!("[PermissionSync] Resolved request {request_id} with {:?}", resolution.decision);
    Ok(true)
}

/// Read a resolved permission request (worker polls this).
pub async fn read_resolved_permission(
    request_id: &str,
    team_name: &str,
) -> Option<SwarmPermissionRequest> {
    let path = resolved_dir(team_name).join(format!("{request_id}.json"));
    let content = fs::read_to_string(&path).await.ok()?;
    serde_json::from_str(&content).ok()
}

/// Delete a resolved permission file after the worker processes it.
pub async fn delete_resolved_permission(request_id: &str, team_name: &str) -> anyhow::Result<()> {
    let path = resolved_dir(team_name).join(format!("{request_id}.json"));
    match fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Poll for a permission response (worker side). Returns the decision once resolved.
pub async fn poll_for_response(
    request_id: &str,
    team_name: &str,
    timeout: Duration,
) -> Option<SwarmPermissionRequest> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(resolved) = read_resolved_permission(request_id, team_name).await {
            // Clean up the resolved file
            let _ = delete_resolved_permission(request_id, team_name).await;
            return Some(resolved);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(super::POLL_INTERVAL_MS)).await;
    }
}

/// Clean up old resolved permission files (> max_age old).
pub async fn cleanup_old_resolutions(team_name: &str, max_age: Duration) -> u32 {
    let dir = resolved_dir(team_name);
    let mut entries = match fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let now = super::team_helpers::now_millis();
    let max_age_ms = max_age.as_millis() as u64;
    let mut cleaned = 0u32;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(content) = fs::read_to_string(&path).await {
                if let Ok(req) = serde_json::from_str::<SwarmPermissionRequest>(&content) {
                    let resolved_at = req.resolved_at.unwrap_or(req.created_at);
                    if now.saturating_sub(resolved_at) >= max_age_ms {
                        let _ = fs::remove_file(&path).await;
                        cleaned += 1;
                    }
                }
            }
        }
    }

    if cleaned > 0 {
        debug!("[PermissionSync] Cleaned up {cleaned} old resolutions");
    }
    cleaned
}
