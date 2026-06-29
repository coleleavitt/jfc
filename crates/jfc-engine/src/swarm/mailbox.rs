//! DB-backed teammate mailbox system.
//!
//! The public path helpers stay for team-directory compatibility, but message
//! traffic is persisted in `jfc-knowledge.agent_mailbox`, keyed by team+agent.

use std::path::PathBuf;

use tokio::fs;
use tracing::{debug, trace};

use super::TEAM_LEAD_NAME;
use super::types::{IdleNotification, MailboxMessage, ShutdownRequest};

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
pub fn swarm_home_override() -> Option<PathBuf> {
    std::env::var_os("JFC_SWARM_HOME_OVERRIDE").map(PathBuf::from)
}

/// Get the team directory: `~/.claude/teams/{sanitized_team_name}/`
pub fn team_dir(team_name: &str) -> PathBuf {
    teams_base_dir().join(super::sanitize_name(team_name))
}

/// Get the inboxes directory for a team.
pub fn inboxes_dir(team_name: &str) -> PathBuf {
    team_dir(team_name).join("inboxes")
}

/// Get the inbox file path for a specific agent.
pub fn inbox_path(agent_name: &str, team_name: &str) -> PathBuf {
    let sanitized = super::sanitize_name(agent_name);
    inboxes_dir(team_name).join(format!("{sanitized}.json"))
}

// ─── Ensure directories ──────────────────────────────────────────────────────

/// Ensure the inboxes directory exists.
pub async fn ensure_inbox_dir(team_name: &str) -> anyhow::Result<()> {
    let dir = inboxes_dir(team_name);
    fs::create_dir_all(&dir).await?;
    Ok(())
}

// ─── Read operations ─────────────────────────────────────────────────────────

/// Read all messages from an agent's inbox.
pub async fn read_mailbox(agent_name: &str, team_name: &str) -> Vec<MailboxMessage> {
    let key = mailbox_key(agent_name, team_name);
    let rows = match run_mailbox_db(move |store| {
        Box::pin(async move { store.list_agent_mailbox(&key, false).await })
    })
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            debug!("[Mailbox] Failed to read DB inbox for {agent_name}: {err}");
            Vec::new()
        }
    };
    rows.into_iter().filter_map(row_to_message).collect()
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

    debug!(
        "[Mailbox] writeToMailbox: recipient={recipient}, from={}, team={team_name}",
        message.from
    );

    let row = mailbox_row(recipient, team_name, message)?;
    run_mailbox_db(move |store| Box::pin(async move { store.enqueue_agent_mailbox(&row).await }))
        .await?;

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
    let key = mailbox_key(agent_name, team_name);
    run_mailbox_db(move |store| {
        Box::pin(async move {
            let rows = store.list_agent_mailbox(&key, false).await?;
            if let Some(row) = rows.get(index) {
                store.mark_agent_mailbox_read(&row.id).await?;
            }
            Ok(())
        })
    })
    .await?;
    Ok(())
}

/// Mark all messages as read.
pub async fn mark_all_read(agent_name: &str, team_name: &str) -> anyhow::Result<()> {
    let key = mailbox_key(agent_name, team_name);
    run_mailbox_db(move |store| {
        Box::pin(async move {
            store.mark_all_agent_mailbox_read(&key).await?;
            Ok(())
        })
    })
    .await?;
    Ok(())
}

/// Clear all messages from an agent's inbox.
pub async fn clear_mailbox(agent_name: &str, team_name: &str) -> anyhow::Result<()> {
    let key = mailbox_key(agent_name, team_name);
    run_mailbox_db(move |store| {
        Box::pin(async move {
            store.clear_agent_mailbox(&key).await?;
            Ok(())
        })
    })
    .await?;
    debug!("[Mailbox] Cleared inbox for {agent_name}");
    Ok(())
}

fn mailbox_key(agent_name: &str, team_name: &str) -> String {
    format!(
        "team:{}:agent:{}",
        super::sanitize_name(team_name),
        super::sanitize_name(agent_name)
    )
}

fn mailbox_row(
    recipient: &str,
    team_name: &str,
    message: MailboxMessage,
) -> anyhow::Result<jfc_knowledge::AgentMailboxRow> {
    let read = message.read;
    let content = serde_json::to_string(&message)?;
    Ok(jfc_knowledge::AgentMailboxRow {
        id: uuid::Uuid::new_v4().simple().to_string(),
        to_agent: mailbox_key(recipient, team_name),
        from_agent: Some(message.from),
        thread_id: Some(super::sanitize_name(team_name)),
        task_id: None,
        priority: 0,
        content,
        read_at_ms: read.then(|| chrono::Utc::now().timestamp_millis()),
        summarized_at_ms: None,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    })
}

fn row_to_message(row: jfc_knowledge::AgentMailboxRow) -> Option<MailboxMessage> {
    let mut message: MailboxMessage = serde_json::from_str(&row.content).ok()?;
    message.read = row.read_at_ms.is_some();
    Some(message)
}

async fn run_mailbox_db<T, F, Fut>(f: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    Fut: std::future::Future<Output = jfc_knowledge::Result<T>> + Send + 'static,
    F: FnOnce(jfc_knowledge::KnowledgeStore) -> Fut + Send + 'static,
{
    let store = open_mailbox_store().await?;
    f(store).await.map_err(anyhow::Error::from)
}

async fn open_mailbox_store() -> jfc_knowledge::Result<jfc_knowledge::KnowledgeStore> {
    if let Some(home) = swarm_home_override() {
        let path = home.join(".jfc").join("knowledge.db");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        return jfc_knowledge::KnowledgeStore::open(&path).await;
    }
    jfc_knowledge::KnowledgeStore::open_default().await
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
        write_to_mailbox("alice", msg.clone(), "alpha")
            .await
            .unwrap();
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
    async fn mailbox_write_uses_db_not_inbox_file_normal() {
        let _g = HomeOverride::new();
        write_to_mailbox(
            "alice",
            MailboxMessage {
                from: "leader".into(),
                text: "db-backed".into(),
                timestamp: "t".into(),
                color: None,
                summary: None,
                read: false,
            },
            "alpha",
        )
        .await
        .unwrap();
        assert!(!inbox_path("alice", "alpha").exists());
        assert_eq!(read_mailbox("alice", "alpha").await[0].text, "db-backed");
    }

    #[tokio::test]
    async fn mailbox_team_keys_are_isolated_normal() {
        let _g = HomeOverride::new();
        for team in ["alpha", "beta"] {
            write_to_mailbox(
                "alice",
                MailboxMessage {
                    from: "leader".into(),
                    text: team.into(),
                    timestamp: "t".into(),
                    color: None,
                    summary: None,
                    read: false,
                },
                team,
            )
            .await
            .unwrap();
        }
        assert_eq!(read_mailbox("alice", "alpha").await[0].text, "alpha");
        assert_eq!(read_mailbox("alice", "beta").await[0].text, "beta");
    }

    #[tokio::test]
    async fn read_mailbox_ignores_legacy_corrupt_file_robust() {
        let _g = HomeOverride::new();
        ensure_inbox_dir("alpha").await.unwrap();
        let path = inbox_path("alice", "alpha");
        tokio::fs::write(&path, "not valid json").await.unwrap();
        let msgs = read_mailbox("alice", "alpha").await;
        assert!(msgs.is_empty());
    }
}
