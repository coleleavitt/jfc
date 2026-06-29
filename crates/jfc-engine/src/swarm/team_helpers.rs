//! Team roster management and directory helpers.
//!
//! Team rosters are persisted in the DB. The Claude-style paths stay as stable
//! compatibility handles for directory layout and one-time legacy import.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs;
use tokio::sync::Mutex as AsyncMutex;
use tracing::debug;

use super::mailbox;
use super::types::*;

const TEAM_FILE_SESSION_ID: &str = "__swarm__";
const TEAM_FILE_KIND: &str = "team_file";

/// Per-team async mutex registry. Member RMW helpers (`add_member`,
/// `remove_member_*`, `set_member_active`, `set_member_mode`) call
/// `with_team_lock` to serialize concurrent writes against the same
/// the team DB row — without this, two spawns racing through `add_member`
/// would both read the pre-spawn member list, both push their own
/// member, and the later write would overwrite the earlier one.
///
/// The outer `StdMutex` only protects the registry map insertion; the
/// inner `AsyncMutex` is what callers actually await on.
fn team_locks() -> &'static StdMutex<HashMap<String, Arc<AsyncMutex<()>>>> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn team_lock_for(team_name: &str) -> Arc<AsyncMutex<()>> {
    let mut map = team_locks().lock().expect("team_locks poisoned");
    Arc::clone(
        map.entry(team_name.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
    )
}

/// Run `f` while holding the per-team async mutex. Use for any helper
/// that does read-modify-write of the team roster.
async fn with_team_lock<F, Fut, T>(team_name: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let lock = team_lock_for(team_name);
    let _guard = lock.lock().await;
    f().await
}

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Get the legacy team config file path used for one-time import.
pub fn team_file_path(team_name: &str) -> PathBuf {
    mailbox::team_dir(team_name).join("config.json")
}

/// Get the tasks directory for a team.
///
/// Honors the same `JFC_SWARM_HOME_OVERRIDE` test hook that `mailbox::team_dir`
/// uses so tests keep both team config and task state inside one `TempDir`.
pub fn tasks_dir(team_name: &str) -> PathBuf {
    let home = mailbox::swarm_home_override()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
    home.join(".claude")
        .join("tasks")
        .join(sanitize_name(team_name))
}

// ─── Read / Write ────────────────────────────────────────────────────────────

/// Read a team file from the DB. Returns None if it doesn't exist.
pub async fn read_team_file(team_name: &str) -> Option<TeamFile> {
    let team_name = team_name.to_owned();
    tokio::task::spawn_blocking(move || read_team_file_db_or_legacy(&team_name))
        .await
        .ok()
        .flatten()
}

/// Read a team file synchronously (for use in sync contexts).
pub fn read_team_file_sync(team_name: &str) -> Option<TeamFile> {
    read_team_file_db_or_legacy(team_name)
}

/// Write (create or overwrite) a team roster row.
pub async fn write_team_file(team_name: &str, team_file: &TeamFile) -> anyhow::Result<()> {
    let dir = mailbox::team_dir(team_name);
    fs::create_dir_all(&dir).await?;
    let team_name = team_name.to_owned();
    let log_team_name = team_name.clone();
    let team_file = team_file.clone();
    tokio::task::spawn_blocking(move || write_team_file_db(&team_name, &team_file)).await??;
    debug!("[TeamHelpers] Wrote team DB row: {}", log_team_name);
    Ok(())
}

/// Write a team file with exclusive creation (fails if already exists).
pub async fn write_team_file_exclusive(
    team_name: &str,
    team_file: &TeamFile,
) -> anyhow::Result<()> {
    let dir = mailbox::team_dir(team_name);
    fs::create_dir_all(&dir).await?;
    if read_team_file(team_name).await.is_some() {
        anyhow::bail!("Team \"{team_name}\" already exists. Choose a different team_name.");
    }

    write_team_file(team_name, team_file).await?;
    debug!("[TeamHelpers] Created team DB row: {team_name}");
    Ok(())
}

// ─── Team Lifecycle ──────────────────────────────────────────────────────────

/// Create a new team. Returns the paths created.
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(team = team_name))]
pub async fn create_team(
    team_name: &str,
    description: Option<&str>,
    lead_agent_id: &str,
    lead_model: Option<&str>,
    cwd: &str,
) -> anyhow::Result<TeamFile> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let team_file = TeamFile {
        name: team_name.to_owned(),
        description: description.map(str::to_owned),
        created_at: now,
        lead_agent_id: lead_agent_id.to_owned(),
        lead_session_id: None,
        members: vec![TeamMember {
            agent_id: lead_agent_id.to_owned(),
            name: super::TEAM_LEAD_NAME.to_owned(),
            agent_type: Some(super::TEAM_LEAD_NAME.to_owned()),
            model: lead_model.map(str::to_owned),
            color: None,
            plan_mode_required: None,
            joined_at: now,
            cwd: Some(cwd.to_owned()),
            worktree_path: None,
            backend_type: None,
            is_active: Some(true),
            mode: None,
        }],
    };

    write_team_file_exclusive(team_name, &team_file).await?;

    // Create tasks directory
    let tasks = tasks_dir(team_name);
    fs::create_dir_all(&tasks).await?;

    // Ensure inboxes directory
    mailbox::ensure_inbox_dir(team_name).await?;

    Ok(team_file)
}

/// Delete a team and all its associated directories.
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(team = team_name))]
pub async fn delete_team(team_name: &str) -> anyhow::Result<()> {
    let team_name_owned = team_name.to_owned();
    tokio::task::spawn_blocking(move || delete_team_file_db(&team_name_owned)).await??;

    // Remove team directory
    let team = mailbox::team_dir(team_name);
    if team.exists() {
        fs::remove_dir_all(&team).await?;
        debug!("[TeamHelpers] Removed team directory: {}", team.display());
    }

    // Remove tasks directory
    let tasks = tasks_dir(team_name);
    if tasks.exists() {
        fs::remove_dir_all(&tasks).await?;
        debug!("[TeamHelpers] Removed tasks directory: {}", tasks.display());
    }

    Ok(())
}

fn team_file_key(team_name: &str) -> String {
    sanitize_name(team_name)
}

fn open_team_store() -> jfc_knowledge::Result<jfc_knowledge::KnowledgeStore> {
    jfc_knowledge::block_on_knowledge(async {
        if let Some(home) = mailbox::swarm_home_override() {
            let path = home.join(".jfc").join("knowledge.db");
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            return jfc_knowledge::KnowledgeStore::open(&path).await;
        }
        jfc_knowledge::KnowledgeStore::open_default().await
    })
}

fn read_team_file_db_or_legacy(team_name: &str) -> Option<TeamFile> {
    let store = open_team_store().ok()?;
    let key = team_file_key(team_name);
    let row = jfc_knowledge::block_on_knowledge(async {
        store
            .get_session_artifact(TEAM_FILE_SESSION_ID, TEAM_FILE_KIND, &key)
            .await
    });
    if let Ok(Some(row)) = row {
        return serde_json::from_str(&row.value_json).ok();
    }
    let path = team_file_path(team_name);
    let content = std::fs::read_to_string(&path).ok()?;
    let team_file = serde_json::from_str::<TeamFile>(&content).ok()?;
    let _ = write_team_file_db(team_name, &team_file);
    Some(team_file)
}

fn write_team_file_db(team_name: &str, team_file: &TeamFile) -> anyhow::Result<()> {
    let json = serde_json::to_string(team_file)?;
    let store = open_team_store()?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .upsert_session_artifact(
                TEAM_FILE_SESSION_ID,
                TEAM_FILE_KIND,
                &team_file_key(team_name),
                &json,
            )
            .await
    })
    .map_err(|e| anyhow::anyhow!("{}", e))
}

fn delete_team_file_db(team_name: &str) -> anyhow::Result<()> {
    let store = open_team_store()?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .delete_session_artifact(
                TEAM_FILE_SESSION_ID,
                TEAM_FILE_KIND,
                &team_file_key(team_name),
            )
            .await
    })?;
    Ok(())
}

// ─── Member Operations ───────────────────────────────────────────────────────

/// Add a member to the team file.
///
/// Serializes against `remove_member_*` / `set_member_*` for the same
/// team via `with_team_lock`. Without the lock, two `add_member` calls
/// racing during a multi-spawn turn would both observe the pre-spawn
/// roster, both push their own member, and the later write would clobber
/// the earlier one — only the last-written teammate would survive in the DB
/// row.
pub async fn add_member(team_name: &str, member: TeamMember) -> anyhow::Result<()> {
    with_team_lock(team_name, || async move {
        let mut team_file = read_team_file(team_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;
        team_file.members.push(member);
        write_team_file(team_name, &team_file).await
    })
    .await
}

/// Remove a member from the team file by agent ID.
pub async fn remove_member_by_id(team_name: &str, agent_id: &str) -> anyhow::Result<bool> {
    with_team_lock(team_name, || async move {
        let mut team_file = read_team_file(team_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

        let original_len = team_file.members.len();
        team_file.members.retain(|m| m.agent_id != agent_id);

        if team_file.members.len() == original_len {
            return Ok(false);
        }

        write_team_file(team_name, &team_file).await?;
        Ok(true)
    })
    .await
}

/// Remove a member from the team file by name.
pub async fn remove_member_by_name(team_name: &str, name: &str) -> anyhow::Result<bool> {
    with_team_lock(team_name, || async move {
        let mut team_file = read_team_file(team_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

        let original_len = team_file.members.len();
        team_file.members.retain(|m| m.name != name);

        if team_file.members.len() == original_len {
            return Ok(false);
        }

        write_team_file(team_name, &team_file).await?;
        Ok(true)
    })
    .await
}

/// Update a member's active status.
pub async fn set_member_active(
    team_name: &str,
    member_name: &str,
    is_active: bool,
) -> anyhow::Result<()> {
    with_team_lock(team_name, || async move {
        let mut team_file = read_team_file(team_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

        if let Some(member) = team_file.members.iter_mut().find(|m| m.name == member_name) {
            member.is_active = Some(is_active);
            write_team_file(team_name, &team_file).await?;
        }

        Ok(())
    })
    .await
}

/// Update a member's permission mode.
pub async fn set_member_mode(team_name: &str, member_name: &str, mode: &str) -> anyhow::Result<()> {
    with_team_lock(team_name, || async move {
        let mut team_file = read_team_file(team_name)
            .await
            .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

        if let Some(member) = team_file.members.iter_mut().find(|m| m.name == member_name) {
            member.mode = Some(mode.to_owned());
            write_team_file(team_name, &team_file).await?;
        }

        Ok(())
    })
    .await
}

/// Get the leader's name from the team file.
pub async fn get_leader_name(team_name: &str) -> Option<String> {
    let team_file = read_team_file(team_name).await?;
    team_file
        .members
        .iter()
        .find(|m| m.agent_id == team_file.lead_agent_id)
        .map(|m| m.name.clone())
        .or_else(|| Some(super::TEAM_LEAD_NAME.to_owned()))
}

/// Get active (non-lead) members of a team.
pub async fn get_active_teammates(team_name: &str) -> Vec<TeamMember> {
    read_team_file(team_name)
        .await
        .map(|tf| {
            tf.members
                .into_iter()
                .filter(|m| m.name != super::TEAM_LEAD_NAME && m.is_active != Some(false))
                .collect()
        })
        .unwrap_or_default()
}

/// Check if the current context is the team leader (no agent ID, or agent ID is team-lead).
pub fn is_team_leader(team_context: &TeamContext) -> bool {
    team_context.lead_agent_id.is_some()
}

// ─── Timestamp helpers ───────────────────────────────────────────────────────

/// Get current time as milliseconds since epoch.
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::test_support::HomeOverride;

    #[test]
    fn team_file_path_lands_in_team_dir_normal() {
        let _g = HomeOverride::new();
        let path = team_file_path("alpha");
        assert!(path.ends_with("teams/alpha/config.json"));
    }

    #[test]
    fn tasks_dir_uses_override_normal() {
        let _g = HomeOverride::new();
        let dir = tasks_dir("My Team");
        assert!(dir.ends_with("tasks/my-team"));
    }

    #[test]
    fn now_millis_is_nonzero_normal() {
        // Sanity: should produce a roughly-current epoch millisecond stamp.
        let n = now_millis();
        assert!(
            n > 1_500_000_000_000,
            "expected post-2017 ms epoch, got {n}"
        );
    }

    #[test]
    fn is_team_leader_returns_true_when_lead_set_normal() {
        let mut ctx = TeamContext::default();
        assert!(!is_team_leader(&ctx));
        ctx.lead_agent_id = Some("lead@alpha".into());
        assert!(is_team_leader(&ctx));
    }

    #[tokio::test]
    async fn read_team_file_returns_none_for_missing_robust() {
        let _g = HomeOverride::new();
        assert!(read_team_file("ghost").await.is_none());
    }

    #[test]
    fn read_team_file_sync_returns_none_for_missing_robust() {
        let _g = HomeOverride::new();
        assert!(read_team_file_sync("ghost").is_none());
    }

    #[tokio::test]
    async fn create_team_writes_config_and_dirs_normal() {
        let _g = HomeOverride::new();
        let tf = create_team(
            "alpha",
            Some("first team"),
            "lead@alpha",
            Some("opus-4-7"),
            "/tmp/cwd",
        )
        .await
        .unwrap();
        assert_eq!(tf.name, "alpha");
        assert_eq!(tf.description.as_deref(), Some("first team"));
        assert_eq!(tf.members.len(), 1);
        assert_eq!(tf.members[0].name, super::super::TEAM_LEAD_NAME);

        // The task/inbox compatibility dirs must exist; the roster itself is DB-backed.
        assert!(tasks_dir("alpha").exists());
        assert!(super::mailbox::team_dir("alpha").join("inboxes").exists());

        // Round-trip read.
        let read_back = read_team_file("alpha").await.expect("read");
        assert_eq!(read_back.name, "alpha");
    }

    #[tokio::test]
    async fn create_team_fails_when_already_exists_robust() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        // Second create with same name → exclusive write rejects it.
        let err = create_team("alpha", None, "lead@alpha", None, "/tmp").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn delete_team_removes_config_and_tasks_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        assert!(read_team_file("alpha").await.is_some());

        delete_team("alpha").await.unwrap();
        assert!(read_team_file("alpha").await.is_none());
        assert!(!tasks_dir("alpha").exists());
    }

    #[tokio::test]
    async fn delete_team_is_idempotent_robust() {
        let _g = HomeOverride::new();
        // No team exists → delete should still succeed.
        delete_team("ghost").await.unwrap();
    }

    #[tokio::test]
    async fn add_member_appends_to_roster_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let new_member = TeamMember {
            agent_id: "alice@alpha".into(),
            name: "alice".into(),
            agent_type: Some("researcher".into()),
            model: None,
            color: Some("#4FC3F7".into()),
            plan_mode_required: Some(false),
            joined_at: 0,
            cwd: None,
            worktree_path: None,
            backend_type: Some(BackendType::InProcess),
            is_active: Some(true),
            mode: None,
        };
        add_member("alpha", new_member).await.unwrap();
        let tf = read_team_file("alpha").await.unwrap();
        assert_eq!(tf.members.len(), 2);
        assert!(tf.members.iter().any(|m| m.name == "alice"));
    }

    #[tokio::test]
    async fn add_member_fails_for_missing_team_robust() {
        let _g = HomeOverride::new();
        let m = TeamMember {
            agent_id: "x".into(),
            name: "x".into(),
            agent_type: None,
            model: None,
            color: None,
            plan_mode_required: None,
            joined_at: 0,
            cwd: None,
            worktree_path: None,
            backend_type: None,
            is_active: None,
            mode: None,
        };
        assert!(add_member("ghost", m).await.is_err());
    }

    #[tokio::test]
    async fn remove_member_by_id_removes_match_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let m = TeamMember {
            agent_id: "alice@alpha".into(),
            name: "alice".into(),
            agent_type: None,
            model: None,
            color: None,
            plan_mode_required: None,
            joined_at: 0,
            cwd: None,
            worktree_path: None,
            backend_type: None,
            is_active: None,
            mode: None,
        };
        add_member("alpha", m).await.unwrap();
        let removed = remove_member_by_id("alpha", "alice@alpha").await.unwrap();
        assert!(removed);
        let tf = read_team_file("alpha").await.unwrap();
        assert_eq!(tf.members.len(), 1);
    }

    #[tokio::test]
    async fn remove_member_by_id_returns_false_for_missing_robust() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let removed = remove_member_by_id("alpha", "nonexistent").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn remove_member_by_name_removes_match_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let m = TeamMember {
            agent_id: "alice@alpha".into(),
            name: "alice".into(),
            agent_type: None,
            model: None,
            color: None,
            plan_mode_required: None,
            joined_at: 0,
            cwd: None,
            worktree_path: None,
            backend_type: None,
            is_active: None,
            mode: None,
        };
        add_member("alpha", m).await.unwrap();
        let removed = remove_member_by_name("alpha", "alice").await.unwrap();
        assert!(removed);
    }

    #[tokio::test]
    async fn remove_member_by_name_returns_false_for_missing_robust() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let removed = remove_member_by_name("alpha", "ghost").await.unwrap();
        assert!(!removed);
    }

    #[tokio::test]
    async fn set_member_active_updates_flag_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let m = TeamMember {
            agent_id: "alice@alpha".into(),
            name: "alice".into(),
            agent_type: None,
            model: None,
            color: None,
            plan_mode_required: None,
            joined_at: 0,
            cwd: None,
            worktree_path: None,
            backend_type: None,
            is_active: Some(true),
            mode: None,
        };
        add_member("alpha", m).await.unwrap();
        set_member_active("alpha", "alice", false).await.unwrap();
        let tf = read_team_file("alpha").await.unwrap();
        let alice = tf.members.iter().find(|m| m.name == "alice").unwrap();
        assert_eq!(alice.is_active, Some(false));
    }

    #[tokio::test]
    async fn set_member_mode_updates_mode_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let m = TeamMember {
            agent_id: "alice@alpha".into(),
            name: "alice".into(),
            agent_type: None,
            model: None,
            color: None,
            plan_mode_required: None,
            joined_at: 0,
            cwd: None,
            worktree_path: None,
            backend_type: None,
            is_active: Some(true),
            mode: None,
        };
        add_member("alpha", m).await.unwrap();
        set_member_mode("alpha", "alice", "bypass-permissions")
            .await
            .unwrap();
        let tf = read_team_file("alpha").await.unwrap();
        let alice = tf.members.iter().find(|m| m.name == "alice").unwrap();
        assert_eq!(alice.mode.as_deref(), Some("bypass-permissions"));
    }

    #[tokio::test]
    async fn get_leader_name_returns_lead_member_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        let leader = get_leader_name("alpha").await.unwrap();
        assert_eq!(leader, super::super::TEAM_LEAD_NAME);
    }

    #[tokio::test]
    async fn get_leader_name_none_for_missing_team_robust() {
        let _g = HomeOverride::new();
        assert!(get_leader_name("ghost").await.is_none());
    }

    #[tokio::test]
    async fn get_active_teammates_excludes_lead_and_inactive_normal() {
        let _g = HomeOverride::new();
        create_team("alpha", None, "lead@alpha", None, "/tmp")
            .await
            .unwrap();
        // Active teammate.
        add_member(
            "alpha",
            TeamMember {
                agent_id: "alice@alpha".into(),
                name: "alice".into(),
                agent_type: None,
                model: None,
                color: None,
                plan_mode_required: None,
                joined_at: 0,
                cwd: None,
                worktree_path: None,
                backend_type: None,
                is_active: Some(true),
                mode: None,
            },
        )
        .await
        .unwrap();
        // Inactive teammate.
        add_member(
            "alpha",
            TeamMember {
                agent_id: "bob@alpha".into(),
                name: "bob".into(),
                agent_type: None,
                model: None,
                color: None,
                plan_mode_required: None,
                joined_at: 0,
                cwd: None,
                worktree_path: None,
                backend_type: None,
                is_active: Some(false),
                mode: None,
            },
        )
        .await
        .unwrap();

        let active = get_active_teammates("alpha").await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "alice");
    }

    #[tokio::test]
    async fn get_active_teammates_empty_for_missing_team_robust() {
        let _g = HomeOverride::new();
        let active = get_active_teammates("ghost").await;
        assert!(active.is_empty());
    }

    #[test]
    fn read_team_file_sync_round_trips_normal() {
        // Use blocking I/O directly so this test stays in a normal sync ctx.
        let _g = HomeOverride::new();
        let dir = mailbox::team_dir("alpha");
        std::fs::create_dir_all(&dir).unwrap();
        let tf = TeamFile {
            name: "alpha".into(),
            description: None,
            created_at: 1,
            lead_agent_id: "lead@alpha".into(),
            lead_session_id: None,
            members: vec![],
        };
        let json = serde_json::to_string(&tf).unwrap();
        std::fs::write(dir.join("config.json"), json).unwrap();

        let read_back = read_team_file_sync("alpha").expect("read");
        assert_eq!(read_back.name, "alpha");
    }
}
