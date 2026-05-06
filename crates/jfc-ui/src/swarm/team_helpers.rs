//! Team file management and directory helpers.
//!
//! Manages the team config file at `~/.claude/teams/{name}/config.json` and
//! the associated task directory at `~/.claude/tasks/{name}/`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs;
use tracing::debug;

use super::mailbox;
use super::types::*;

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Get the team config file path.
pub fn team_file_path(team_name: &str) -> PathBuf {
    mailbox::team_dir(team_name).join("config.json")
}

/// Get the tasks directory for a team.
pub fn tasks_dir(team_name: &str) -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("tasks")
        .join(sanitize_name(team_name))
}

// ─── Read / Write ────────────────────────────────────────────────────────────

/// Read a team file. Returns None if it doesn't exist.
pub async fn read_team_file(team_name: &str) -> Option<TeamFile> {
    let path = team_file_path(team_name);
    let content = fs::read_to_string(&path).await.ok()?;
    serde_json::from_str(&content).ok()
}

/// Read a team file synchronously (for use in sync contexts).
pub fn read_team_file_sync(team_name: &str) -> Option<TeamFile> {
    let path = team_file_path(team_name);
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Write (create or overwrite) a team file.
pub async fn write_team_file(team_name: &str, team_file: &TeamFile) -> anyhow::Result<()> {
    let dir = mailbox::team_dir(team_name);
    fs::create_dir_all(&dir).await?;
    let path = dir.join("config.json");
    let json = serde_json::to_string_pretty(team_file)?;
    fs::write(&path, json).await?;
    debug!("[TeamHelpers] Wrote team file: {}", path.display());
    Ok(())
}

/// Write a team file with exclusive creation (fails if already exists).
pub async fn write_team_file_exclusive(
    team_name: &str,
    team_file: &TeamFile,
) -> anyhow::Result<()> {
    let dir = mailbox::team_dir(team_name);
    fs::create_dir_all(&dir).await?;
    let path = dir.join("config.json");

    // Check if file already exists
    if path.exists() {
        anyhow::bail!(
            "Team \"{team_name}\" already exists at {}. Choose a different team_name.",
            path.display()
        );
    }

    let json = serde_json::to_string_pretty(team_file)?;
    fs::write(&path, json).await?;
    debug!("[TeamHelpers] Created team file: {}", path.display());
    Ok(())
}

// ─── Team Lifecycle ──────────────────────────────────────────────────────────

/// Create a new team. Returns the paths created.
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
pub async fn delete_team(team_name: &str) -> anyhow::Result<()> {
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

// ─── Member Operations ───────────────────────────────────────────────────────

/// Add a member to the team file.
pub async fn add_member(team_name: &str, member: TeamMember) -> anyhow::Result<()> {
    let mut team_file = read_team_file(team_name)
        .await
        .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

    team_file.members.push(member);
    write_team_file(team_name, &team_file).await
}

/// Remove a member from the team file by agent ID.
pub async fn remove_member_by_id(team_name: &str, agent_id: &str) -> anyhow::Result<bool> {
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
}

/// Remove a member from the team file by name.
pub async fn remove_member_by_name(team_name: &str, name: &str) -> anyhow::Result<bool> {
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
}

/// Update a member's active status.
pub async fn set_member_active(
    team_name: &str,
    member_name: &str,
    is_active: bool,
) -> anyhow::Result<()> {
    let mut team_file = read_team_file(team_name)
        .await
        .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

    if let Some(member) = team_file.members.iter_mut().find(|m| m.name == member_name) {
        member.is_active = Some(is_active);
        write_team_file(team_name, &team_file).await?;
    }

    Ok(())
}

/// Update a member's permission mode.
pub async fn set_member_mode(
    team_name: &str,
    member_name: &str,
    mode: &str,
) -> anyhow::Result<()> {
    let mut team_file = read_team_file(team_name)
        .await
        .ok_or_else(|| anyhow::anyhow!("Team '{team_name}' not found"))?;

    if let Some(member) = team_file.members.iter_mut().find(|m| m.name == member_name) {
        member.mode = Some(mode.to_owned());
        write_team_file(team_name, &team_file).await?;
    }

    Ok(())
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
