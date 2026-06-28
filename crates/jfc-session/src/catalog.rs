use std::path::{Path, PathBuf};

use tracing::{debug, info};

use crate::SessionId;
use jfc_knowledge::{KnowledgeStore, SessionRow};

/// Load session metadata without full message deserialization. The
/// picker only needs the DB session row fields plus a message count —
/// it never inspects tool inputs or message parts.
pub async fn load_session_metadata(session_id: &SessionId) -> Option<SessionMetadata> {
    load_session_metadata_from_db(session_id.as_str())
}

fn open_session_db() -> Option<KnowledgeStore> {
    jfc_knowledge::block_on_knowledge(async { crate::open_default_knowledge_store().await.ok() })
}

fn metadata_from_row(row: SessionRow) -> SessionMetadata {
    SessionMetadata {
        id: SessionId::new(row.id),
        created_at: row.created_at.unwrap_or_default(),
        updated_at: row.updated_at,
        first_prompt: row.first_prompt,
        cwd: row.cwd,
        title: row.title,
        message_count: row.message_count.max(0) as usize,
    }
}

fn load_session_metadata_from_db(session_id: &str) -> Option<SessionMetadata> {
    let store = open_session_db()?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .get_session(session_id)
            .await
            .ok()
            .flatten()
            .map(metadata_from_row)
    })
}

#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub first_prompt: Option<String>,
    /// Working directory the session was created in. `None` means the DB row
    /// lacks a cwd, and consumers must treat that as "no warning" (see
    /// `cwd_mismatch_message`).
    pub cwd: Option<String>,
    /// User-set title (`/rename` slash). `None` falls back to first_prompt.
    pub title: Option<String>,
    pub message_count: usize,
}

impl SessionMetadata {
    /// v126 title precedence: customTitle → firstPrompt → formatted-id-timestamp.
    /// Picks the best human-readable label for the picker / sidebar.
    pub fn display_title(&self) -> String {
        if let Some(t) = self.title.as_deref().filter(|s| !s.trim().is_empty()) {
            return t.trim().to_owned();
        }
        if let Some(prompt) = self.first_prompt.as_deref() {
            let trimmed = prompt.trim();
            if !trimmed.is_empty() {
                // First line only — multi-line prompts blow up the row.
                let first_line = trimmed.lines().next().unwrap_or(trimmed);
                const MAX: usize = 60;
                if first_line.chars().count() > MAX {
                    let truncated: String = first_line.chars().take(MAX).collect();
                    return format!("{truncated}…");
                }
                return first_line.to_owned();
            }
        }
        // Fallback: pretty-print the timestamp from the id.
        format_session_id_timestamp(self.id.as_str())
    }

    /// Best timestamp to compare/display: prefers `updated_at`, falls back
    /// to `created_at`. Always returns *some* string so callers don't have
    /// to thread through `Option`.
    pub fn last_activity(&self) -> &str {
        self.updated_at.as_deref().unwrap_or(&self.created_at)
    }
}

/// Convert a session id like `ses_20260503_212945` into a friendly
/// `2026-05-03 21:29` for fallback display.
pub fn format_session_id_timestamp(id: &str) -> String {
    let cleaned = id.strip_prefix("ses_").unwrap_or(id);
    let mut parts = cleaned.splitn(2, '_');
    let date = parts.next().unwrap_or("");
    let time = parts.next().unwrap_or("");
    if date.len() == 8 && time.len() >= 4 {
        format!(
            "{}-{}-{} {}:{}",
            &date[..4],
            &date[4..6],
            &date[6..8],
            &time[..2],
            &time[2..4]
        )
    } else {
        id.to_owned()
    }
}

/// Split sessions into `(this_project, other_projects)` based on whether
/// each session's `cwd` matches `current_cwd`. Sessions with `cwd: None`
/// always land in `other_projects`. Order within each group is preserved
/// (callers are expected to have already sorted by recency).
///
/// Pure helper — kept free of `App` so it can be unit-tested with synthetic
/// `SessionMetadata`.
pub fn group_by_cwd(
    sessions: Vec<SessionMetadata>,
    current_cwd: Option<&str>,
) -> (Vec<SessionMetadata>, Vec<SessionMetadata>) {
    let mut this_project = Vec::new();
    let mut other = Vec::new();
    for s in sessions {
        match (current_cwd, s.cwd.as_deref()) {
            (Some(cur), Some(sc)) if sc == cur => this_project.push(s),
            _ => other.push(s),
        }
    }
    (this_project, other)
}

/// Render the cwd in shortened form for the sidebar's secondary line:
/// home directory becomes `~`, paths under home become `~/rest`, and
/// other absolute paths are shown as their basename. Returns `"—"` when
/// the cwd is missing so the row still has something to show in the muted slot.
pub fn shorten_cwd(cwd: Option<&str>) -> String {
    let Some(cwd) = cwd else {
        return "—".to_owned();
    };
    let home = dirs::home_dir().and_then(|p| p.to_str().map(str::to_owned));
    if let Some(home) = home {
        if cwd == home {
            return "~".to_owned();
        }
        if let Some(rest) = cwd.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    // Not under home: show the basename so we don't blow up narrow sidebars
    // with a long absolute path. Strip trailing slash first; bare `/` stays
    // as `/` (root) rather than collapsing to an empty string.
    let trimmed = cwd.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_owned();
    }
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_owned()
}

/// Format a delta between an RFC3339 timestamp and `now` as a short
/// human label like `"14m ago"`, `"3h ago"`, `"2d ago"`. Falls back to
/// `"—"` when the input doesn't parse. Compact form is used because
/// the sidebar's secondary line is shared with the cwd badge and msg
/// count and panics on width.
pub fn relative_time(timestamp: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return "—".to_owned();
    };
    let parsed_utc = parsed.with_timezone(&chrono::Utc);
    let delta = now.signed_duration_since(parsed_utc);
    let secs = delta.num_seconds();
    if secs < 0 {
        // Future timestamp (clock skew) — just say "now".
        return "now".to_owned();
    }
    if secs < 60 {
        return "just now".to_owned();
    }
    let mins = delta.num_minutes();
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = delta.num_hours();
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = delta.num_days();
    if days < 30 {
        return format!("{days}d ago");
    }
    let months = days / 30;
    if months < 12 {
        return format!("{months}mo ago");
    }
    let years = days / 365;
    format!("{years}y ago")
}

/// Pure helper: produces a warning message when a resumed session's
/// recorded cwd differs from the current cwd. Returns `None` if the
/// session has no cwd, if the current cwd is empty (we
/// can't compare to anything meaningful), or if the two paths match.
///
/// Mirrors codex-rs `tui/src/session_resume.rs:99-111` — the surface
/// is informational; the resume still proceeds.
pub fn cwd_mismatch_message(session_cwd: Option<&str>, current_cwd: &str) -> Option<String> {
    let session_cwd = session_cwd?;
    if current_cwd.is_empty() {
        return None;
    }
    if cwd_paths_match(session_cwd, current_cwd) {
        return None;
    }
    Some(format!(
        "Session was created in {session_cwd}; current cwd is {current_cwd}"
    ))
}

fn cwd_matches_filter(session_cwd: Option<&str>, cwd_filter: Option<&str>) -> bool {
    match cwd_filter {
        None => true,
        Some(target) => session_cwd.is_some_and(|cwd| cwd_paths_match(cwd, target)),
    }
}

fn cwd_paths_match(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    normalized_cwd(left) == normalized_cwd(right)
}

fn normalized_cwd(path: &str) -> PathBuf {
    let trimmed = path.trim_end_matches('/');
    let stable = if trimmed.is_empty() { "/" } else { trimmed };
    Path::new(stable)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(stable))
}

pub async fn list_sessions() -> Vec<SessionId> {
    list_sessions_filtered(None)
        .await
        .into_iter()
        .map(|meta| meta.id)
        .collect()
}

pub async fn has_any_session() -> bool {
    !list_sessions().await.is_empty()
}

/// List sessions with metadata, sorted by most recent update.
/// When `cwd_filter` is `Some(path)`, only sessions whose `cwd` matches
/// are returned. Pass `None` for the
/// "show all" mode (mirrors codex-rs `--show-all` / v126's all-sessions).
pub async fn list_sessions_with_metadata() -> Vec<SessionMetadata> {
    list_sessions_filtered(None).await
}

pub async fn list_sessions_filtered(cwd_filter: Option<&str>) -> Vec<SessionMetadata> {
    debug!(target: "jfc::session", ?cwd_filter, "listing sessions with filter");
    let mut sessions = Vec::new();
    if let Some(store) = open_session_db() {
        if let Ok(rows) = store.list_sessions(None, 100_000).await {
            sessions.extend(
                rows.into_iter()
                    .map(metadata_from_row)
                    .filter(|meta| cwd_matches_filter(meta.cwd.as_deref(), cwd_filter)),
            );
        }
    }
    sessions.sort_by(|a, b| {
        let a_time = a.updated_at.as_ref().unwrap_or(&a.created_at);
        let b_time = b.updated_at.as_ref().unwrap_or(&b.created_at);
        b_time.cmp(a_time)
    });
    info!(target: "jfc::session", count = sessions.len(), ?cwd_filter, "sessions filtered from DB");
    sessions
}

/// Lazy variant: list session IDs only (sorted by DB recency)
/// without reading metadata for each. Use when the caller only needs
/// the IDs (e.g. /resume autocomplete).
pub async fn list_session_ids_only() -> Vec<SessionId> {
    list_sessions().await
}

/// Most recent session for the *current cwd*. Mirrors v126
/// (cli.js:480735-480741) and codex-rs default behavior — `--continue`
/// in project A doesn't accidentally resume a session from project B.
/// Pass `None` for the globally-most-recent behavior.
pub async fn most_recent_session_for_cwd(cwd: Option<&str>) -> Option<SessionId> {
    list_sessions_filtered(cwd)
        .await
        .into_iter()
        .next()
        .map(|meta| meta.id)
}

/// Globally most-recent session id (`--global` flag).
pub async fn most_recent_session() -> Option<SessionId> {
    let result = list_sessions().await.into_iter().next();
    debug!(target: "jfc::session", found = result.is_some(), "most recent session (global)");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cwd_filter_matches_trailing_slash_variant_regression() {
        assert!(cwd_matches_filter(
            Some("/tmp/project/"),
            Some("/tmp/project")
        ));
    }

    #[test]
    fn cwd_filter_matches_canonical_equivalent_path_regression() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let equivalent = nested.join("..").join("nested");

        assert!(cwd_matches_filter(nested.to_str(), equivalent.to_str()));
    }

    #[test]
    fn cwd_mismatch_uses_normalized_paths_normal() {
        assert_eq!(
            cwd_mismatch_message(Some("/tmp/project/"), "/tmp/project"),
            None
        );
    }

    #[test]
    fn most_recent_session_for_cwd_uses_normalized_db_cwd_regression() {
        let db = crate::TestKnowledgeDb::new();
        let project = db.root().join("project");
        std::fs::create_dir(&project).unwrap();
        let equivalent = project.join("..").join("project");

        jfc_knowledge::block_on_knowledge(async {
            let store = crate::open_default_knowledge_store().await.unwrap();
            store
                .upsert_session(&jfc_knowledge::SessionRow {
                    id: "matching-project".to_owned(),
                    cwd: Some(project.display().to_string()),
                    model: None,
                    created_at: Some("2026-01-01T00:00:00Z".to_owned()),
                    updated_at: Some("2026-01-01T00:00:01Z".to_owned()),
                    first_prompt: None,
                    title: None,
                    message_count: 1,
                })
                .await
                .unwrap();
            store
                .upsert_session(&jfc_knowledge::SessionRow {
                    id: "newer-other-project".to_owned(),
                    cwd: Some(db.root().join("other").display().to_string()),
                    model: None,
                    created_at: Some("2026-01-01T00:00:00Z".to_owned()),
                    updated_at: Some("2026-01-01T00:00:02Z".to_owned()),
                    first_prompt: None,
                    title: None,
                    message_count: 1,
                })
                .await
                .unwrap();
        });

        let picked = jfc_knowledge::block_on_knowledge(async {
            most_recent_session_for_cwd(equivalent.to_str()).await
        });

        assert_eq!(
            picked.map(|id| id.as_str().to_owned()),
            Some("matching-project".to_owned())
        );
    }
}
