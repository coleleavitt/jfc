//! Permission synchronization between swarm workers and the team leader.
//!
//! When a worker agent encounters a tool that requires user approval, it sends
//! a permission request to the leader's mailbox. The leader presents the
//! request to the user, collects approval/rejection, and sends a response
//! back to the worker's mailbox.
//!
//! Storage layout:
//! ```text
//! session_artifacts("__swarm__", "permission_pending", "{team}:{id}")
//! session_artifacts("__swarm__", "permission_resolved", "{team}:{id}")
//! ```

use std::path::PathBuf;
use std::time::Duration;

use tokio::fs;
use tracing::debug;

use super::mailbox;
use super::types::*;

const PERMISSION_SESSION_ID: &str = "__swarm__";
const PERMISSION_PENDING_KIND: &str = "permission_pending";
const PERMISSION_RESOLVED_KIND: &str = "permission_resolved";

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

fn permission_key(team_name: &str, request_id: &str) -> String {
    format!("{}:{request_id}", super::sanitize_name(team_name))
}

fn permission_key_prefix(team_name: &str) -> String {
    format!("{}:", super::sanitize_name(team_name))
}

async fn open_permission_store() -> anyhow::Result<jfc_knowledge::KnowledgeStore> {
    if let Some(home) = mailbox::swarm_home_override() {
        let path = home.join(".jfc").join("knowledge.db");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        return Ok(jfc_knowledge::KnowledgeStore::open(&path).await?);
    }
    Ok(jfc_knowledge::KnowledgeStore::open_default().await?)
}

async fn run_permission_db<T, F, Fut>(f: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
    F: FnOnce(jfc_knowledge::KnowledgeStore) -> Fut + Send + 'static,
{
    let store = open_permission_store().await?;
    f(store).await
}

fn legacy_permission_dir(kind: &str, team_name: &str) -> PathBuf {
    match kind {
        PERMISSION_PENDING_KIND => pending_dir(team_name),
        PERMISSION_RESOLVED_KIND => resolved_dir(team_name),
        _ => permission_dir(team_name),
    }
}

async fn import_legacy_permission_files(
    store: &jfc_knowledge::KnowledgeStore,
    kind: &str,
    team_name: &str,
) -> anyhow::Result<()> {
    let dir = legacy_permission_dir(kind, team_name);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(());
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|e| e == "json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(request) = serde_json::from_str::<SwarmPermissionRequest>(&content) else {
            continue;
        };
        let key = permission_key(team_name, &request.id);
        store
            .upsert_session_artifact(PERMISSION_SESSION_ID, kind, &key, &content)
            .await?;
        let _ = std::fs::remove_file(&path);
    }
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

/// Write a pending permission request to the DB.
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(id = %request.id, team = %request.team_name, tool = %request.tool_name))]
pub async fn write_permission_request(request: &SwarmPermissionRequest) -> anyhow::Result<()> {
    ensure_permission_dirs(&request.team_name).await?;
    let request = request.clone();
    let log_id = request.id.clone();
    let log_worker_name = request.worker_name.clone();
    let log_tool_name = request.tool_name.clone();
    run_permission_db(move |store| {
        Box::pin(async move {
            let key = permission_key(&request.team_name, &request.id);
            let json = serde_json::to_string(&request)?;
            store
                .upsert_session_artifact(
                    PERMISSION_SESSION_ID,
                    PERMISSION_PENDING_KIND,
                    &key,
                    &json,
                )
                .await?;
            Ok(())
        })
    })
    .await?;
    debug!(
        "[PermissionSync] Wrote pending request {} from {} for {}",
        log_id, log_worker_name, log_tool_name
    );
    Ok(())
}

/// Read all pending permission requests for a team.
pub async fn read_pending_permissions(team_name: &str) -> Vec<SwarmPermissionRequest> {
    let team_name = team_name.to_owned();
    run_permission_db(move |store| {
        Box::pin(async move {
            import_legacy_permission_files(&store, PERMISSION_PENDING_KIND, &team_name).await?;
            let prefix = permission_key_prefix(&team_name);
            let mut results = store
                .list_session_artifacts(PERMISSION_SESSION_ID, PERMISSION_PENDING_KIND, 10_000)
                .await?
                .into_iter()
                .filter(|row| row.key.starts_with(&prefix))
                .filter_map(|row| {
                    serde_json::from_str::<SwarmPermissionRequest>(&row.value_json).ok()
                })
                .collect::<Vec<_>>();
            results.sort_by_key(|r| r.created_at);
            Ok(results)
        })
    })
    .await
    .unwrap_or_default()
}

/// Resolve a permission request (move from pending to resolved).
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(id = request_id, team = team_name, decision = ?resolution.decision))]
pub async fn resolve_permission(
    request_id: &str,
    resolution: &PermissionResolution,
    team_name: &str,
) -> anyhow::Result<bool> {
    ensure_permission_dirs(team_name).await?;

    let request_id_owned = request_id.to_owned();
    let team_name_owned = team_name.to_owned();
    let decision = resolution.decision;
    let resolution = resolution.clone();
    let moved = run_permission_db(move |store| {
        Box::pin(async move {
            import_legacy_permission_files(&store, PERMISSION_PENDING_KIND, &team_name_owned)
                .await?;
            let key = permission_key(&team_name_owned, &request_id_owned);
            let Some(row) = store
                .get_session_artifact(PERMISSION_SESSION_ID, PERMISSION_PENDING_KIND, &key)
                .await?
            else {
                debug!("[PermissionSync] Pending request not found: {request_id_owned}");
                return Ok(false);
            };

            let mut request: SwarmPermissionRequest = serde_json::from_str(&row.value_json)?;
            request.status = match resolution.decision {
                PermissionDecision::Approved => PermissionRequestStatus::Approved,
                PermissionDecision::Rejected => PermissionRequestStatus::Rejected,
            };
            request.resolved_by = Some(resolution.resolved_by);
            request.resolved_at = Some(super::team_helpers::now_millis());
            request.feedback = resolution.feedback;
            request.updated_input = resolution.updated_input;
            request.permission_updates = resolution.permission_updates;

            let json = serde_json::to_string(&request)?;
            store
                .upsert_session_artifact(
                    PERMISSION_SESSION_ID,
                    PERMISSION_RESOLVED_KIND,
                    &key,
                    &json,
                )
                .await?;
            store
                .delete_session_artifact(PERMISSION_SESSION_ID, PERMISSION_PENDING_KIND, &key)
                .await?;
            Ok(true)
        })
    })
    .await?;

    debug!("[PermissionSync] Resolved request {request_id} with {decision:?}");
    Ok(moved)
}

/// Read a resolved permission request (worker polls this).
pub async fn read_resolved_permission(
    request_id: &str,
    team_name: &str,
) -> Option<SwarmPermissionRequest> {
    let request_id = request_id.to_owned();
    let team_name = team_name.to_owned();
    run_permission_db(move |store| {
        Box::pin(async move {
            import_legacy_permission_files(&store, PERMISSION_RESOLVED_KIND, &team_name).await?;
            let key = permission_key(&team_name, &request_id);
            let row = store
                .get_session_artifact(PERMISSION_SESSION_ID, PERMISSION_RESOLVED_KIND, &key)
                .await?;
            Ok(row.and_then(|row| serde_json::from_str(&row.value_json).ok()))
        })
    })
    .await
    .ok()
    .flatten()
}

/// Delete a resolved permission row after the worker processes it.
pub async fn delete_resolved_permission(request_id: &str, team_name: &str) -> anyhow::Result<()> {
    let request_id = request_id.to_owned();
    let team_name = team_name.to_owned();
    run_permission_db(move |store| {
        Box::pin(async move {
            let key = permission_key(&team_name, &request_id);
            store
                .delete_session_artifact(PERMISSION_SESSION_ID, PERMISSION_RESOLVED_KIND, &key)
                .await?;
            Ok(())
        })
    })
    .await
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
            // Clean up the resolved row.
            let _ = delete_resolved_permission(request_id, team_name).await;
            return Some(resolved);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(super::POLL_INTERVAL_MS)).await;
    }
}

/// Clean up old resolved permission rows (> max_age old).
pub async fn cleanup_old_resolutions(team_name: &str, max_age: Duration) -> u32 {
    let team_name = team_name.to_owned();
    run_permission_db(move |store| {
        Box::pin(async move {
            import_legacy_permission_files(&store, PERMISSION_RESOLVED_KIND, &team_name).await?;
            let prefix = permission_key_prefix(&team_name);
            let now = super::team_helpers::now_millis();
            let max_age_ms = max_age.as_millis() as u64;
            let mut cleaned = 0u32;

            for row in store
                .list_session_artifacts(PERMISSION_SESSION_ID, PERMISSION_RESOLVED_KIND, 10_000)
                .await?
                .into_iter()
                .filter(|row| row.key.starts_with(&prefix))
            {
                let Ok(req) = serde_json::from_str::<SwarmPermissionRequest>(&row.value_json)
                else {
                    continue;
                };
                let resolved_at = req.resolved_at.unwrap_or(req.created_at);
                if now.saturating_sub(resolved_at) >= max_age_ms {
                    store
                        .delete_session_artifact(
                            PERMISSION_SESSION_ID,
                            PERMISSION_RESOLVED_KIND,
                            &row.key,
                        )
                        .await?;
                    cleaned += 1;
                }
            }
            Ok(cleaned)
        })
    })
    .await
    .unwrap_or(0)
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

        // Resolved row should be readable with status updated.
        let resolved = read_resolved_permission(&id, "alpha")
            .await
            .expect("resolved row present");
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
        // Deleting a non-existent row should not error.
        delete_resolved_permission("nope", "alpha").await.unwrap();
    }

    #[tokio::test]
    async fn delete_resolved_permission_removes_row_normal() {
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
        let mut req = make_request("alpha", "Bash");
        req.status = PermissionRequestStatus::Approved;
        req.resolved_at = Some(0); // unix epoch — very old
        let key = permission_key("alpha", &req.id);
        let json = serde_json::to_string(&req).unwrap();
        run_permission_db(move |store| {
            Box::pin(async move {
                store
                    .upsert_session_artifact(
                        PERMISSION_SESSION_ID,
                        PERMISSION_RESOLVED_KIND,
                        &key,
                        &json,
                    )
                    .await?;
                Ok(())
            })
        })
        .await
        .unwrap();

        let cleaned = cleanup_old_resolutions("alpha", Duration::from_secs(60)).await;
        assert_eq!(cleaned, 1);
        assert!(read_resolved_permission(&req.id, "alpha").await.is_none());
    }

    #[tokio::test]
    async fn cleanup_old_resolutions_keeps_fresh_files_normal() {
        let _g = HomeOverride::new();
        let mut req = make_request("alpha", "Bash");
        req.status = PermissionRequestStatus::Approved;
        req.resolved_at = Some(super::super::team_helpers::now_millis()); // brand-new
        let key = permission_key("alpha", &req.id);
        let json = serde_json::to_string(&req).unwrap();
        run_permission_db(move |store| {
            Box::pin(async move {
                store
                    .upsert_session_artifact(
                        PERMISSION_SESSION_ID,
                        PERMISSION_RESOLVED_KIND,
                        &key,
                        &json,
                    )
                    .await?;
                Ok(())
            })
        })
        .await
        .unwrap();

        let cleaned = cleanup_old_resolutions("alpha", Duration::from_secs(3600)).await;
        assert_eq!(cleaned, 0);
        assert!(read_resolved_permission(&req.id, "alpha").await.is_some());
    }

    #[tokio::test]
    async fn cleanup_old_resolutions_returns_zero_when_dir_missing_robust() {
        let _g = HomeOverride::new();
        let cleaned = cleanup_old_resolutions("ghost", Duration::from_secs(1)).await;
        assert_eq!(cleaned, 0);
    }
}
