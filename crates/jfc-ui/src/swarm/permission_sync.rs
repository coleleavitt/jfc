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
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(id = %request.id, team = %request.team_name, tool = %request.tool_name))]
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
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(id = request_id, team = team_name, decision = ?resolution.decision))]
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::test_support::HomeOverride;

    fn make_request(team: &str, tool: &str) -> SwarmPermissionRequest {
        create_permission_request(
            tool,
            "tool-use-1",
            serde_json::json!({"file": "x"}),
            "test perm",
            "alice@team",
            "alice",
            Some("#FF0000"),
            team,
        )
    }

    #[test]
    fn generate_request_id_is_unique_normal() {
        let a = generate_request_id();
        let b = generate_request_id();
        assert_ne!(a, b);
        assert!(a.starts_with("perm-"));
    }

    #[test]
    fn create_permission_request_populates_fields_normal() {
        let req = create_permission_request(
            "Bash",
            "use-1",
            serde_json::json!({"cmd": "ls"}),
            "list dir",
            "alice@alpha",
            "alice",
            Some("#abc"),
            "alpha",
        );
        assert_eq!(req.tool_name, "Bash");
        assert_eq!(req.tool_use_id, "use-1");
        assert_eq!(req.worker_id, "alice@alpha");
        assert_eq!(req.worker_name, "alice");
        assert_eq!(req.worker_color.as_deref(), Some("#abc"));
        assert_eq!(req.team_name, "alpha");
        assert_eq!(req.status, PermissionRequestStatus::Pending);
        assert!(req.resolved_by.is_none());
        assert!(req.created_at > 0);
    }

    #[tokio::test]
    async fn write_and_read_pending_round_trip_normal() {
        let _g = HomeOverride::new();
        let req = make_request("alpha", "Bash");
        write_permission_request(&req).await.unwrap();
        let pending = read_pending_permissions("alpha").await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, req.id);
        assert_eq!(pending[0].tool_name, "Bash");
    }

    #[tokio::test]
    async fn read_pending_permissions_empty_when_dir_missing_robust() {
        let _g = HomeOverride::new();
        // Directory has never been created — read should yield an empty Vec,
        // not panic.
        let pending = read_pending_permissions("ghost").await;
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn read_pending_permissions_sorted_by_created_at_normal() {
        let _g = HomeOverride::new();
        let mut r1 = make_request("alpha", "first");
        r1.created_at = 100;
        let mut r2 = make_request("alpha", "second");
        r2.created_at = 50;
        let mut r3 = make_request("alpha", "third");
        r3.created_at = 200;
        // Make the IDs unique so they're stored as separate files.
        r1.id = "perm-100".into();
        r2.id = "perm-050".into();
        r3.id = "perm-200".into();
        write_permission_request(&r1).await.unwrap();
        write_permission_request(&r2).await.unwrap();
        write_permission_request(&r3).await.unwrap();
        let pending = read_pending_permissions("alpha").await;
        let order: Vec<_> = pending.iter().map(|r| r.tool_name.as_str()).collect();
        assert_eq!(order, vec!["second", "first", "third"]);
    }

    #[tokio::test]
    async fn resolve_permission_moves_pending_to_resolved_normal() {
        let _g = HomeOverride::new();
        let req = make_request("alpha", "Bash");
        let id = req.id.clone();
        write_permission_request(&req).await.unwrap();

        let resolution = PermissionResolution {
            decision: PermissionDecision::Approved,
            resolved_by: "user".into(),
            feedback: Some("approved".into()),
            updated_input: None,
            permission_updates: Vec::new(),
        };
        let moved = resolve_permission(&id, &resolution, "alpha").await.unwrap();
        assert!(moved);

        // Pending should be empty.
        assert!(read_pending_permissions("alpha").await.is_empty());

        // Resolved file should be readable with status updated.
        let resolved = read_resolved_permission(&id, "alpha")
            .await
            .expect("resolved file present");
        assert_eq!(resolved.status, PermissionRequestStatus::Approved);
        assert_eq!(resolved.resolved_by.as_deref(), Some("user"));
        assert_eq!(resolved.feedback.as_deref(), Some("approved"));
        assert!(resolved.resolved_at.is_some());
    }

    #[tokio::test]
    async fn resolve_permission_rejected_decision_normal() {
        let _g = HomeOverride::new();
        let req = make_request("alpha", "WriteFile");
        let id = req.id.clone();
        write_permission_request(&req).await.unwrap();

        let resolution = PermissionResolution {
            decision: PermissionDecision::Rejected,
            resolved_by: "leader".into(),
            feedback: None,
            updated_input: None,
            permission_updates: Vec::new(),
        };
        resolve_permission(&id, &resolution, "alpha").await.unwrap();
        let resolved = read_resolved_permission(&id, "alpha").await.unwrap();
        assert_eq!(resolved.status, PermissionRequestStatus::Rejected);
    }

    #[tokio::test]
    async fn resolve_permission_returns_false_for_missing_robust() {
        let _g = HomeOverride::new();
        let resolution = PermissionResolution {
            decision: PermissionDecision::Approved,
            resolved_by: "u".into(),
            feedback: None,
            updated_input: None,
            permission_updates: Vec::new(),
        };
        let moved = resolve_permission("nonexistent", &resolution, "alpha")
            .await
            .unwrap();
        assert!(!moved);
    }

    #[tokio::test]
    async fn read_resolved_permission_returns_none_when_absent_robust() {
        let _g = HomeOverride::new();
        assert!(read_resolved_permission("nope", "alpha").await.is_none());
    }

    #[tokio::test]
    async fn delete_resolved_permission_is_idempotent_robust() {
        let _g = HomeOverride::new();
        // Deleting a non-existent file should not error.
        delete_resolved_permission("nope", "alpha").await.unwrap();
    }

    #[tokio::test]
    async fn delete_resolved_permission_removes_file_normal() {
        let _g = HomeOverride::new();
        let req = make_request("alpha", "Bash");
        let id = req.id.clone();
        write_permission_request(&req).await.unwrap();
        resolve_permission(
            &id,
            &PermissionResolution {
                decision: PermissionDecision::Approved,
                resolved_by: "u".into(),
                feedback: None,
                updated_input: None,
                permission_updates: Vec::new(),
            },
            "alpha",
        )
        .await
        .unwrap();
        assert!(read_resolved_permission(&id, "alpha").await.is_some());

        delete_resolved_permission(&id, "alpha").await.unwrap();
        assert!(read_resolved_permission(&id, "alpha").await.is_none());
    }

    #[tokio::test]
    async fn poll_for_response_returns_immediately_when_resolved_normal() {
        let _g = HomeOverride::new();
        let req = make_request("alpha", "Bash");
        let id = req.id.clone();
        write_permission_request(&req).await.unwrap();
        resolve_permission(
            &id,
            &PermissionResolution {
                decision: PermissionDecision::Approved,
                resolved_by: "u".into(),
                feedback: None,
                updated_input: None,
                permission_updates: Vec::new(),
            },
            "alpha",
        )
        .await
        .unwrap();

        let resp = poll_for_response(&id, "alpha", Duration::from_millis(500)).await;
        let resp = resp.expect("must find resolved");
        assert_eq!(resp.status, PermissionRequestStatus::Approved);
        // After polling, the resolved file is cleaned up.
        assert!(read_resolved_permission(&id, "alpha").await.is_none());
    }

    #[tokio::test]
    async fn poll_for_response_times_out_when_unresolved_robust() {
        let _g = HomeOverride::new();
        // No resolved file ever appears — poll should give up cleanly.
        let resp = poll_for_response("never", "alpha", Duration::from_millis(50)).await;
        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn cleanup_old_resolutions_removes_aged_files_normal() {
        let _g = HomeOverride::new();
        // Write a resolved file with a very old `resolved_at`.
        ensure_permission_dirs("alpha").await.unwrap();
        let mut req = make_request("alpha", "Bash");
        req.status = PermissionRequestStatus::Approved;
        req.resolved_at = Some(0); // unix epoch — very old
        let path = resolved_dir("alpha").join(format!("{}.json", req.id));
        let json = serde_json::to_string_pretty(&req).unwrap();
        tokio::fs::write(&path, json).await.unwrap();

        let cleaned = cleanup_old_resolutions("alpha", Duration::from_secs(60)).await;
        assert_eq!(cleaned, 1);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn cleanup_old_resolutions_keeps_fresh_files_normal() {
        let _g = HomeOverride::new();
        ensure_permission_dirs("alpha").await.unwrap();
        let mut req = make_request("alpha", "Bash");
        req.status = PermissionRequestStatus::Approved;
        req.resolved_at = Some(super::super::team_helpers::now_millis()); // brand-new
        let path = resolved_dir("alpha").join(format!("{}.json", req.id));
        let json = serde_json::to_string_pretty(&req).unwrap();
        tokio::fs::write(&path, json).await.unwrap();

        let cleaned = cleanup_old_resolutions("alpha", Duration::from_secs(3600)).await;
        assert_eq!(cleaned, 0);
        assert!(path.exists());
    }

    #[tokio::test]
    async fn cleanup_old_resolutions_returns_zero_when_dir_missing_robust() {
        let _g = HomeOverride::new();
        let cleaned = cleanup_old_resolutions("ghost", Duration::from_secs(1)).await;
        assert_eq!(cleaned, 0);
    }
}
