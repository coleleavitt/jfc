//! File-based teammate mailbox system.
//!
//! Each teammate has a JSON inbox file at:
//!   `~/.claude/teams/{team}/inboxes/{agent-name}.json`
//!
//! Messages are JSON arrays of `MailboxMessage`. File locking (via an adjacent
//! `.lock` file) ensures atomic read-modify-write operations across processes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::fs;
use tracing::{debug, warn};

use super::types::{IdleNotification, MailboxMessage, ShutdownRequest};
use super::TEAM_LEAD_NAME;

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Get the base directory for team data: `~/.claude/teams/`
pub fn teams_base_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("teams")
}

/// Get the team directory: `~/.claude/teams/{sanitized_team_name}/`
pub fn team_dir(team_name: &str) -> PathBuf {
    teams_base_dir().join(super::sanitize_name(team_name))
}

/// Get the inboxes directory for a team.
fn inboxes_dir(team_name: &str) -> PathBuf {
    team_dir(team_name).join("inboxes")
}

/// Get the inbox file path for a specific agent.
fn inbox_path(agent_name: &str, team_name: &str) -> PathBuf {
    let sanitized = super::sanitize_name(agent_name);
    inboxes_dir(team_name).join(format!("{sanitized}.json"))
}

/// Lock file path for an inbox.
fn lock_path(inbox: &Path) -> PathBuf {
    inbox.with_extension("json.lock")
}

// ─── Ensure directories ──────────────────────────────────────────────────────

/// Ensure the inboxes directory exists.
pub async fn ensure_inbox_dir(team_name: &str) -> anyhow::Result<()> {
    let dir = inboxes_dir(team_name);
    fs::create_dir_all(&dir).await?;
    Ok(())
}

// ─── File locking ────────────────────────────────────────────────────────────

/// Simple advisory file lock using create-exclusive. Returns a guard that
/// releases the lock on drop.
///
/// This is a simplified version — production would use `flock(2)` or
/// `lockfile` crate. For in-process teammates (our primary mode), the Mutex
/// in the runner provides the real synchronization; this lock is for
/// cross-process safety with tmux-based teammates.
pub struct FileLock {
    path: PathBuf,
}

impl FileLock {
    /// Attempt to acquire the lock, retrying for up to `timeout`.
    pub async fn acquire(path: PathBuf, timeout: Duration) -> anyhow::Result<Self> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(_) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if tokio::time::Instant::now() >= deadline {
                        anyhow::bail!("mailbox lock timeout: {}", path.display());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Release the lock.
    pub async fn release(self) {
        let _ = fs::remove_file(&self.path).await;
    }
}

// ─── Read operations ─────────────────────────────────────────────────────────

/// Read all messages from an agent's inbox.
pub async fn read_mailbox(agent_name: &str, team_name: &str) -> Vec<MailboxMessage> {
    let path = inbox_path(agent_name, team_name);
    match fs::read_to_string(&path).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            warn!("[Mailbox] Failed to read inbox for {agent_name}: {e}");
            Vec::new()
        }
    }
}

/// Read only unread messages from an agent's inbox.
pub async fn read_unread_messages(agent_name: &str, team_name: &str) -> Vec<MailboxMessage> {
    read_mailbox(agent_name, team_name)
        .await
        .into_iter()
        .filter(|m| !m.read)
        .collect()
}

// ─── Write operations ────────────────────────────────────────────────────────

/// Write a message to a recipient's inbox.
pub async fn write_to_mailbox(
    recipient: &str,
    message: MailboxMessage,
    team_name: &str,
) -> anyhow::Result<()> {
    ensure_inbox_dir(team_name).await?;

    let path = inbox_path(recipient, team_name);
    let lock_file = lock_path(&path);

    debug!(
        "[Mailbox] writeToMailbox: recipient={recipient}, from={}, path={}",
        message.from,
        path.display()
    );

    // Ensure inbox file exists
    if !path.exists() {
        fs::write(&path, "[]").await?;
    }

    // Acquire lock
    let lock = FileLock::acquire(lock_file, Duration::from_secs(10)).await?;

    // Read current messages
    let mut messages = read_mailbox(recipient, team_name).await;

    // Append new message
    messages.push(message);

    // Write back
    let json = serde_json::to_string_pretty(&messages)?;
    fs::write(&path, json).await?;

    // Release lock
    lock.release().await;

    debug!("[Mailbox] Wrote message to {recipient}'s inbox");
    Ok(())
}

/// Mark a message at a specific index as read.
pub async fn mark_message_read(
    agent_name: &str,
    team_name: &str,
    index: usize,
) -> anyhow::Result<()> {
    let path = inbox_path(agent_name, team_name);
    let lock_file = lock_path(&path);

    let lock = FileLock::acquire(lock_file, Duration::from_secs(10)).await?;

    let mut messages = read_mailbox(agent_name, team_name).await;
    if index < messages.len() {
        messages[index].read = true;
        let json = serde_json::to_string_pretty(&messages)?;
        fs::write(&path, json).await?;
    }

    lock.release().await;
    Ok(())
}

/// Mark all messages as read.
pub async fn mark_all_read(agent_name: &str, team_name: &str) -> anyhow::Result<()> {
    let path = inbox_path(agent_name, team_name);
    let lock_file = lock_path(&path);

    let lock = FileLock::acquire(lock_file, Duration::from_secs(10)).await?;

    let mut messages = read_mailbox(agent_name, team_name).await;
    for msg in &mut messages {
        msg.read = true;
    }
    let json = serde_json::to_string_pretty(&messages)?;
    fs::write(&path, json).await?;

    lock.release().await;
    Ok(())
}

/// Clear all messages from an agent's inbox.
pub async fn clear_mailbox(agent_name: &str, team_name: &str) -> anyhow::Result<()> {
    let path = inbox_path(agent_name, team_name);
    let lock_file = lock_path(&path);

    let lock = FileLock::acquire(lock_file, Duration::from_secs(10)).await?;
    fs::write(&path, "[]").await?;
    lock.release().await;

    debug!("[Mailbox] Cleared inbox for {agent_name}");
    Ok(())
}

// ─── Convenience: send message to leader ─────────────────────────────────────

/// Send a message to the team leader. Convenience wrapper around `write_to_mailbox`.
pub async fn send_to_leader(
    from: &str,
    text: &str,
    color: Option<&str>,
    team_name: &str,
) -> anyhow::Result<()> {
    let msg = MailboxMessage {
        from: from.to_owned(),
        text: text.to_owned(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        color: color.map(str::to_owned),
        summary: None,
        read: false,
    };
    write_to_mailbox(TEAM_LEAD_NAME, msg, team_name).await
}

/// Send an idle notification to the team leader.
pub async fn send_idle_notification(
    agent_name: &str,
    color: Option<&str>,
    team_name: &str,
    idle_reason: Option<&str>,
    summary: Option<&str>,
) -> anyhow::Result<()> {
    let notification = IdleNotification {
        msg_type: "idle_notification".to_owned(),
        from: agent_name.to_owned(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        idle_reason: idle_reason.map(str::to_owned),
        summary: summary.map(str::to_owned),
        completed_task_id: None,
        completed_status: None,
        failure_reason: None,
    };

    let text = serde_json::to_string(&notification)?;
    send_to_leader(agent_name, &text, color, team_name).await
}

// ─── Parsing helpers ─────────────────────────────────────────────────────────

/// Try to parse a mailbox message text as a shutdown request.
pub fn parse_shutdown_request(text: &str) -> Option<ShutdownRequest> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    if val.get("type")?.as_str()? == "shutdown_request" {
        serde_json::from_value(val).ok()
    } else {
        None
    }
}

/// Check if a message text contains an idle notification.
pub fn is_idle_notification(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("type")?.as_str().map(|s| s == "idle_notification"))
        .unwrap_or(false)
}
