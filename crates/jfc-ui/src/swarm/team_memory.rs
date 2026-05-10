//! Team shared memory — a watched filesystem directory that all teammates access.
//!
//! When any teammate writes to `.jfc-teams/<team>/memory/`, all other teammates
//! in the team get notified via their mailbox. This enables coordination through
//! shared state rather than point-to-point messaging.
//!
//! File layout:
//! ```text
//! .jfc-teams/<team>/memory/
//! ├── coordination.md       # Free-form coordination notes
//! ├── progress.json         # Structured progress tracking
//! ├── discoveries.md        # Findings from exploration
//! └── <arbitrary files>     # Agents can create any files
//! ```

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::mailbox;
use super::team_helpers;

/// Event emitted when team memory changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMemoryEvent {
    pub team_name: String,
    pub file_path: String,
    pub event_type: MemoryEventType,
    pub author: String,
    pub timestamp: u64,
}

/// Types of memory file events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryEventType {
    Created,
    Modified,
    Deleted,
}

/// Get the team memory directory path.
pub fn team_memory_dir(team_name: &str) -> PathBuf {
    mailbox::team_dir(team_name).join("memory")
}

/// Ensure the team memory directory exists.
pub fn ensure_memory_dir(team_name: &str) -> std::io::Result<PathBuf> {
    let dir = team_memory_dir(team_name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Write a file to team memory and notify all teammates.
pub async fn write_memory_file(
    team_name: &str,
    file_name: &str,
    content: &str,
    author: &str,
) -> anyhow::Result<PathBuf> {
    let dir = ensure_memory_dir(team_name)?;
    let file_path = dir.join(file_name);
    tokio::fs::write(&file_path, content).await?;

    // Notify all team members
    notify_memory_change(team_name, file_name, MemoryEventType::Modified, author).await?;

    Ok(file_path)
}

/// Read a file from team memory.
pub async fn read_memory_file(team_name: &str, file_name: &str) -> anyhow::Result<String> {
    let file_path = team_memory_dir(team_name).join(file_name);
    let content = tokio::fs::read_to_string(&file_path).await?;
    Ok(content)
}

/// List all files in team memory.
pub async fn list_memory_files(team_name: &str) -> anyhow::Result<Vec<MemoryFileInfo>> {
    let dir = team_memory_dir(team_name);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        if metadata.is_file() {
            files.push(MemoryFileInfo {
                name: entry.file_name().to_string_lossy().to_string(),
                size: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

/// Delete a file from team memory and notify.
pub async fn delete_memory_file(
    team_name: &str,
    file_name: &str,
    author: &str,
) -> anyhow::Result<()> {
    let file_path = team_memory_dir(team_name).join(file_name);
    if file_path.exists() {
        tokio::fs::remove_file(&file_path).await?;
        notify_memory_change(team_name, file_name, MemoryEventType::Deleted, author).await?;
    }
    Ok(())
}

/// Append content to a team memory file (creates if not exists).
pub async fn append_memory_file(
    team_name: &str,
    file_name: &str,
    content: &str,
    author: &str,
) -> anyhow::Result<()> {
    let dir = ensure_memory_dir(team_name)?;
    let file_path = dir.join(file_name);

    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
        .await?;
    file.write_all(content.as_bytes()).await?;
    file.write_all(b"\n").await?;

    notify_memory_change(team_name, file_name, MemoryEventType::Modified, author).await?;
    Ok(())
}

/// Notify all team members about a memory change via their mailboxes.
async fn notify_memory_change(
    team_name: &str,
    file_name: &str,
    event_type: MemoryEventType,
    author: &str,
) -> anyhow::Result<()> {
    let notification = format!(
        "[team-memory:{}] {} {} {}",
        match event_type {
            MemoryEventType::Created => "created",
            MemoryEventType::Modified => "modified",
            MemoryEventType::Deleted => "deleted",
        },
        author,
        match event_type {
            MemoryEventType::Created => "created",
            MemoryEventType::Modified => "updated",
            MemoryEventType::Deleted => "deleted",
        },
        file_name
    );

    // Get all active team members and notify via mailbox
    let members = team_helpers::get_active_teammates(team_name).await;
    for member in &members {
        if member.name != author {
            let msg = super::types::MailboxMessage {
                from: author.to_string(),
                text: notification.clone(),
                timestamp: team_helpers::now_millis().to_string(),
                color: None,
                summary: Some(format!(
                    "Memory: {file_name} {}",
                    match event_type {
                        MemoryEventType::Created => "created",
                        MemoryEventType::Modified => "updated",
                        MemoryEventType::Deleted => "deleted",
                    }
                )),
                read: false,
            };
            let _ = mailbox::write_to_mailbox(&member.name, msg, team_name).await;
        }
    }

    Ok(())
}

/// Info about a file in team memory.
#[derive(Debug, Clone)]
pub struct MemoryFileInfo {
    pub name: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Build a context summary of team memory for inclusion in the system prompt.
pub async fn build_team_memory_context(team_name: &str) -> String {
    let files = list_memory_files(team_name).await.unwrap_or_default();
    if files.is_empty() {
        return String::new();
    }

    let mut ctx = String::from("## Team Shared Memory\n\nFiles in team memory:\n");
    for f in &files {
        ctx.push_str(&format!("- `{}` ({} bytes)\n", f.name, f.size));
    }
    ctx.push_str("\nUse `read_memory_file` to read, `write_memory_file` to update.\n");
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_memory_dir_path() {
        let dir = team_memory_dir("my-team");
        assert!(dir.to_string_lossy().contains("my-team"));
        assert!(dir.to_string_lossy().ends_with("memory"));
    }

    #[test]
    fn memory_event_serialization() {
        let event = TeamMemoryEvent {
            team_name: "test".to_string(),
            file_path: "notes.md".to_string(),
            event_type: MemoryEventType::Modified,
            author: "agent-1".to_string(),
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("Modified"));
        assert!(json.contains("notes.md"));
    }
}
