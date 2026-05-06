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
use tracing::{debug, trace, warn};

use super::types::{IdleNotification, MailboxMessage, ShutdownRequest};
use super::TEAM_LEAD_NAME;

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Get the base directory for team data: `~/.claude/teams/`
///
/// In tests, `JFC_SWARM_HOME_OVERRIDE` redirects this to a temp directory so
/// parallel tests don't clobber the real `$HOME/.claude/teams`.
pub fn teams_base_dir() -> PathBuf {
    if let Some(base) = swarm_home_override() {
        return base.join(".claude").join("teams");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("teams")
}

/// Test-only override: `JFC_SWARM_HOME_OVERRIDE` points to a directory that
/// stands in for `$HOME`. Production never sets this; tests use it to keep
/// mailbox/permission/task fixtures inside a `TempDir`.
pub(crate) fn swarm_home_override() -> Option<PathBuf> {
    std::env::var_os("JFC_SWARM_HOME_OVERRIDE").map(PathBuf::from)
}

/// Get the team directory: `~/.claude/teams/{sanitized_team_name}/`
pub fn team_dir(team_name: &str) -> PathBuf {
    teams_base_dir().join(super::sanitize_name(team_name))
}

/// Get the inboxes directory for a team.
pub(crate) fn inboxes_dir(team_name: &str) -> PathBuf {
    team_dir(team_name).join("inboxes")
}

/// Get the inbox file path for a specific agent.
pub(crate) fn inbox_path(agent_name: &str, team_name: &str) -> PathBuf {
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
#[tracing::instrument(
    target = "jfc::swarm",
    level = "trace",
    skip_all,
    fields(recipient, from = %message.from, team = team_name)
)]
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
#[tracing::instrument(target = "jfc::swarm", level = "trace", skip_all, fields(agent = agent_name, team = team_name, index))]
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
    trace!(target: "jfc::swarm", len = text.len(), "parse_shutdown_request");
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::test_support::HomeOverride;
    use tempfile::TempDir;

    #[test]
    fn teams_base_dir_uses_override_normal() {
        let g = HomeOverride::new();
        assert_eq!(teams_base_dir(), g.home().join(".claude").join("teams"));
    }

    #[test]
    fn team_dir_sanitizes_name_normal() {
        let g = HomeOverride::new();
        assert_eq!(
            team_dir("My Team!"),
            g.home().join(".claude").join("teams").join("my-team-")
        );
    }

    #[test]
    fn inbox_path_sanitizes_agent_normal() {
        let _g = HomeOverride::new();
        let path = inbox_path("Alice/Bob", "alpha");
        assert!(path.ends_with("inboxes/alice-bob.json"));
    }

    #[tokio::test]
    async fn ensure_inbox_dir_creates_directory_normal() {
        let _g = HomeOverride::new();
        ensure_inbox_dir("alpha").await.unwrap();
        assert!(inboxes_dir("alpha").exists());
    }

    #[tokio::test]
    async fn read_mailbox_returns_empty_for_missing_file_robust() {
        let _g = HomeOverride::new();
        let messages = read_mailbox("ghost", "alpha").await;
        assert!(messages.is_empty());
    }

    #[tokio::test]
    async fn write_then_read_mailbox_round_trips_normal() {
        let _g = HomeOverride::new();
        let msg = MailboxMessage {
            from: "leader".into(),
            text: "hello".into(),
            timestamp: "2024-01-01T00:00:00Z".into(),
            color: Some("#ff0000".into()),
            summary: Some("greeting".into()),
            read: false,
        };
        write_to_mailbox("alice", msg.clone(), "alpha").await.unwrap();
        let got = read_mailbox("alice", "alpha").await;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].from, "leader");
        assert_eq!(got[0].text, "hello");
        assert!(!got[0].read);
    }

    #[tokio::test]
    async fn read_unread_filters_read_messages_normal() {
        let _g = HomeOverride::new();
        for (text, read) in [("a", false), ("b", true), ("c", false)] {
            write_to_mailbox(
                "alice",
                MailboxMessage {
                    from: "leader".into(),
                    text: text.into(),
                    timestamp: "2024".into(),
                    color: None,
                    summary: None,
                    read,
                },
                "alpha",
            )
            .await
            .unwrap();
        }
        let unread = read_unread_messages("alice", "alpha").await;
        assert_eq!(unread.len(), 2);
        assert_eq!(unread[0].text, "a");
        assert_eq!(unread[1].text, "c");
    }

    #[tokio::test]
    async fn mark_message_read_flips_flag_normal() {
        let _g = HomeOverride::new();
        write_to_mailbox(
            "alice",
            MailboxMessage {
                from: "leader".into(),
                text: "hi".into(),
                timestamp: "t".into(),
                color: None,
                summary: None,
                read: false,
            },
            "alpha",
        )
        .await
        .unwrap();
        mark_message_read("alice", "alpha", 0).await.unwrap();
        let msgs = read_mailbox("alice", "alpha").await;
        assert!(msgs[0].read);
    }

    #[tokio::test]
    async fn mark_message_read_out_of_bounds_is_no_op_robust() {
        let _g = HomeOverride::new();
        write_to_mailbox(
            "alice",
            MailboxMessage {
                from: "x".into(),
                text: "y".into(),
                timestamp: "t".into(),
                color: None,
                summary: None,
                read: false,
            },
            "alpha",
        )
        .await
        .unwrap();
        // Index past end: silently no-op.
        mark_message_read("alice", "alpha", 99).await.unwrap();
        let msgs = read_mailbox("alice", "alpha").await;
        assert!(!msgs[0].read);
    }

    #[tokio::test]
    async fn mark_all_read_flips_every_message_normal() {
        let _g = HomeOverride::new();
        for i in 0..3 {
            write_to_mailbox(
                "alice",
                MailboxMessage {
                    from: "x".into(),
                    text: format!("m{i}"),
                    timestamp: "t".into(),
                    color: None,
                    summary: None,
                    read: false,
                },
                "alpha",
            )
            .await
            .unwrap();
        }
        mark_all_read("alice", "alpha").await.unwrap();
        let msgs = read_mailbox("alice", "alpha").await;
        assert_eq!(msgs.len(), 3);
        assert!(msgs.iter().all(|m| m.read));
    }

    #[tokio::test]
    async fn clear_mailbox_empties_inbox_normal() {
        let _g = HomeOverride::new();
        write_to_mailbox(
            "alice",
            MailboxMessage {
                from: "x".into(),
                text: "y".into(),
                timestamp: "t".into(),
                color: None,
                summary: None,
                read: false,
            },
            "alpha",
        )
        .await
        .unwrap();
        clear_mailbox("alice", "alpha").await.unwrap();
        let msgs = read_mailbox("alice", "alpha").await;
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn send_to_leader_writes_to_team_lead_inbox_normal() {
        let _g = HomeOverride::new();
        send_to_leader("alice", "summary text", Some("#abcdef"), "alpha")
            .await
            .unwrap();
        let msgs = read_mailbox(super::TEAM_LEAD_NAME, "alpha").await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].from, "alice");
        assert_eq!(msgs[0].text, "summary text");
        assert_eq!(msgs[0].color.as_deref(), Some("#abcdef"));
    }

    #[tokio::test]
    async fn send_idle_notification_serializes_payload_normal() {
        let _g = HomeOverride::new();
        send_idle_notification("alice", None, "alpha", Some("done"), Some("ok"))
            .await
            .unwrap();
        let msgs = read_mailbox(super::TEAM_LEAD_NAME, "alpha").await;
        assert_eq!(msgs.len(), 1);
        assert!(is_idle_notification(&msgs[0].text));
        let v: serde_json::Value = serde_json::from_str(&msgs[0].text).unwrap();
        assert_eq!(v["type"], "idle_notification");
        assert_eq!(v["from"], "alice");
        assert_eq!(v["idleReason"], "done");
        assert_eq!(v["summary"], "ok");
    }

    #[test]
    fn parse_shutdown_request_recognizes_correct_type_normal() {
        let json = r#"{"type":"shutdown_request","requestId":"r1","from":"alice","reason":"done"}"#;
        let req = parse_shutdown_request(json).expect("must parse");
        assert_eq!(req.request_id, "r1");
        assert_eq!(req.from, "alice");
        assert_eq!(req.reason.as_deref(), Some("done"));
    }

    #[test]
    fn parse_shutdown_request_rejects_other_types_robust() {
        // Wrong type → None.
        assert!(parse_shutdown_request(r#"{"type":"idle_notification"}"#).is_none());
        // No type field → None.
        assert!(parse_shutdown_request(r#"{"requestId":"r"}"#).is_none());
        // Not JSON → None, not panic.
        assert!(parse_shutdown_request("not-json").is_none());
        // Empty → None.
        assert!(parse_shutdown_request("").is_none());
    }

    #[test]
    fn is_idle_notification_recognizes_type_normal() {
        assert!(is_idle_notification(r#"{"type":"idle_notification"}"#));
    }

    #[test]
    fn is_idle_notification_rejects_other_types_robust() {
        assert!(!is_idle_notification(r#"{"type":"shutdown_request"}"#));
        assert!(!is_idle_notification(r#"{"foo":"bar"}"#));
        assert!(!is_idle_notification("plain text"));
        assert!(!is_idle_notification(""));
    }

    #[tokio::test]
    async fn file_lock_acquires_and_releases_normal() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("test.lock");
        let lock = FileLock::acquire(lock_path.clone(), Duration::from_millis(100))
            .await
            .unwrap();
        assert!(lock_path.exists());
        lock.release().await;
        assert!(!lock_path.exists());
    }

    #[tokio::test]
    async fn file_lock_times_out_when_held_robust() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("contended.lock");
        // Hold the lock and try to acquire it again with a short timeout.
        let _holder = FileLock::acquire(lock_path.clone(), Duration::from_millis(50))
            .await
            .unwrap();
        let result = FileLock::acquire(lock_path, Duration::from_millis(80)).await;
        assert!(result.is_err(), "second acquire should time out");
    }

    #[tokio::test]
    async fn read_mailbox_returns_empty_on_corrupt_file_robust() {
        let _g = HomeOverride::new();
        // Manually create a corrupt inbox file.
        ensure_inbox_dir("alpha").await.unwrap();
        let path = inbox_path("alice", "alpha");
        tokio::fs::write(&path, "not valid json").await.unwrap();
        let msgs = read_mailbox("alice", "alpha").await;
        assert!(msgs.is_empty());
    }
}
