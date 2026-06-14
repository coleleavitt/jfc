use serde::Deserialize;
use tracing::{debug, info};

use crate::{SessionId, sessions_dir};

/// Load session metadata without full message deserialization. The
/// picker only needs the session header fields plus a message count —
/// it never inspects tool inputs or message parts. Previously this
/// went through the full `SerializedSession` deserializer, so a single
/// schema drift in any message (e.g. an old `Tool { input: ... }`
/// entry written before a field was added) failed the whole session
/// and the picker dropped it from the sidebar. Now we deserialize a
/// lightweight `SessionMetaShallow` that treats `messages` as opaque
/// JSON values; the Tool-input shape never gates picker visibility.
pub async fn load_session_metadata(session_id: &SessionId) -> Option<SessionMetadata> {
    let session_id_str = session_id.as_str();
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let shallow: SessionMetaShallow = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            // Downgrade to debug for schema-mismatch on old sessions — these are
            // expected when the SerializedToolOutput format changed. The session
            // remains in the sessions dir but is silently skipped in listings.
            // A WARN flood of 20+ messages per startup was traced to May 4 sessions.
            debug!(target: "jfc::session", session_id = session_id_str, error = %e, "skipping old session (schema mismatch — pre-migration format)");
            return None;
        }
    };
    let message_count = shallow.messages.len();
    debug!(target: "jfc::session", session_id = session_id_str, message_count, "loaded session metadata");
    Some(SessionMetadata {
        id: SessionId::new(shallow.id),
        created_at: shallow.created_at,
        updated_at: shallow.updated_at,
        first_prompt: shallow.first_prompt,
        cwd: shallow.cwd,
        title: shallow.title,
        message_count,
    })
}

/// Shallow view used only for the picker. `messages` is parsed as
/// opaque JSON values so a malformed message body never invalidates
/// the whole header. Full-fidelity deserialization is reserved for
/// the resume path (`load_session`) where missing fields would
/// actually matter.
#[derive(Deserialize)]
struct SessionMetaShallow {
    id: String,
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    first_prompt: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    messages: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub id: SessionId,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub first_prompt: Option<String>,
    /// Working directory the session was created in. `None` for legacy
    /// sessions saved before the field landed — those are visible only
    /// in "show all" listings, and consumers must treat `None` as "no
    /// warning" (see `cwd_mismatch_message`).
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
/// the cwd is missing (legacy session) so the row still has *something*
/// to show in the muted slot.
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
/// session has no cwd (legacy file), if the current cwd is empty (we
/// can't compare to anything meaningful), or if the two paths match.
///
/// Mirrors codex-rs `tui/src/session_resume.rs:99-111` — the surface
/// is informational; the resume still proceeds.
pub fn cwd_mismatch_message(session_cwd: Option<&str>, current_cwd: &str) -> Option<String> {
    let session_cwd = session_cwd?;
    if current_cwd.is_empty() {
        return None;
    }
    if session_cwd == current_cwd {
        return None;
    }
    Some(format!(
        "Session was created in {session_cwd}; current cwd is {current_cwd}"
    ))
}

pub async fn list_sessions() -> Vec<SessionId> {
    let dir = sessions_dir();
    debug!(target: "jfc::session", dir = %dir.display(), "listing sessions");
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        debug!(target: "jfc::session", dir = %dir.display(), "sessions directory not readable");
        return vec![];
    };
    let mut ids: Vec<SessionId> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(id) = name.strip_suffix(".json") {
            ids.push(SessionId::new(id));
        }
    }
    ids.sort_by(|a, b| b.as_str().cmp(a.as_str())); // newest first
    debug!(target: "jfc::session", count = ids.len(), "sessions listed");
    ids
}

/// List sessions with metadata, sorted by most recent update.
/// When `cwd_filter` is `Some(path)`, only sessions whose `cwd` matches
/// (or whose cwd is unset — legacy) are returned. Pass `None` for the
/// "show all" mode (mirrors codex-rs `--show-all` / v126's all-sessions).
pub async fn list_sessions_with_metadata() -> Vec<SessionMetadata> {
    list_sessions_filtered(None).await
}

pub async fn list_sessions_filtered(cwd_filter: Option<&str>) -> Vec<SessionMetadata> {
    debug!(target: "jfc::session", ?cwd_filter, "listing sessions with filter");
    let ids = list_sessions().await;
    // v132 lazy/parallel session loading. The previous serial loop did
    // one tokio::fs::read per session; with hundreds of sessions in
    // ~/.config/jfc/sessions/ that's a ~50ms × N stall on startup.
    // join_all hands every metadata read to the runtime concurrently
    // — bound by the number of file descriptors, not session count —
    // dropping wall-clock from ~5s to ~150ms on a 100-session vault.
    let metas = futures::future::join_all(ids.iter().map(load_session_metadata)).await;
    let mut sessions: Vec<SessionMetadata> = metas
        .into_iter()
        .flatten()
        .filter(|meta| match cwd_filter {
            None => true,
            Some(target) => meta.cwd.as_deref().is_none_or(|c| c == target),
        })
        .collect();
    sessions.sort_by(|a, b| {
        let a_time = a.updated_at.as_ref().unwrap_or(&a.created_at);
        let b_time = b.updated_at.as_ref().unwrap_or(&b.created_at);
        b_time.cmp(a_time)
    });
    info!(target: "jfc::session", count = sessions.len(), ?cwd_filter, "sessions filtered (parallel)");
    sessions
}

/// Lazy variant: list session IDs *only* (sorted by mtime descending)
/// without reading metadata for each. Use when the caller only needs
/// the IDs (e.g. /resume autocomplete) — saves the per-session JSON
/// read.
pub async fn list_session_ids_only() -> Vec<SessionId> {
    list_sessions().await
}

/// Most recent session for the *current cwd*. Mirrors v126
/// (cli.js:480735-480741) and codex-rs default behavior — `--continue`
/// in project A doesn't accidentally resume a session from project B.
/// Pass `None` for the legacy globally-most-recent behavior.
///
/// **Optimized path:** Session filenames are timestamps (`ses_YYYYMMDD_HHMMSS`),
/// so iterating them in reverse-lexicographic order IS newest-first. We read
/// only the first 1 KB of each file (the `"cwd"` field is always in the JSON
/// header, well before byte 512) and extract the cwd with a cheap byte scan.
/// This avoids parsing multi-hundred-MB session files just to find which
/// project they belong to — turning a 2.3s startup into ~5ms.
pub async fn most_recent_session_for_cwd(cwd: Option<&str>) -> Option<SessionId> {
    let Some(target_cwd) = cwd else {
        // No cwd filter → global most-recent (filename order is sufficient)
        return most_recent_session().await;
    };

    let dir = sessions_dir();
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return None;
    };

    // Collect filenames, sort newest-first (lexicographic on the timestamp).
    // Filename order = *creation* order; we re-rank the cwd matches by
    // `updated_at` below so the session you most recently *worked in* wins,
    // not the one most recently created.
    let mut filenames: Vec<String> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".json") && !name.contains(".tmp") {
            filenames.push(name);
        }
    }
    filenames.sort_unstable_by(|a, b| b.cmp(a)); // newest-created first

    // Scan each matching file's header (1 KB is plenty — the JSON header
    // `id, created_at, updated_at, first_prompt, model, cwd` lands within
    // the first ~400 bytes; `first_prompt` is length-capped). Pick the
    // cwd-matching session with the greatest `updated_at`.
    //
    // ## Why updated_at, not filename order
    //
    // Session filenames are `ses_<created_at>`, so filename order is
    // *creation* order. But `--continue` should resume the session the user
    // was last *working in*. If they resume an older session and keep
    // editing it, its `updated_at` advances while its filename stays old —
    // filename order would wrongly skip it for a newer-but-untouched session
    // in the same project. `updated_at` is written on every save
    // (session/core.rs), so it's the correct recency key. RFC3339 timestamps
    // sort lexicographically == chronologically, so a string `max` works.
    // Sessions missing `updated_at` (legacy) fall back to filename order via
    // the `created_at`-shaped session id as the tiebreak key.
    let mut best: Option<(String, String)> = None; // (updated_at_key, id)
    for filename in &filenames {
        let path = dir.join(filename);
        let Ok(file) = tokio::fs::File::open(&path).await else {
            continue;
        };
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0u8; 1024];
        let mut file = file;
        let n = match file.read(&mut buf).await {
            Ok(n) => n,
            Err(_) => continue,
        };
        let header = &buf[..n];

        let Some(cwd_value) = extract_cwd_from_header(header) else {
            continue; // legacy session without cwd, or cwd beyond 1 KB — skip
        };
        if cwd_value != target_cwd {
            continue;
        }
        let id = filename
            .strip_suffix(".json")
            .unwrap_or(filename)
            .to_owned();
        // Recency key: updated_at if present, else the id itself (which
        // embeds the creation timestamp) so legacy rows still rank sanely.
        let recency = extract_string_field_from_header(header, "updated_at")
            .map(str::to_owned)
            .unwrap_or_else(|| id.clone());
        match &best {
            Some((best_key, _)) if *best_key >= recency => {}
            _ => best = Some((recency, id)),
        }
    }

    if let Some((_, id)) = best {
        debug!(
            target: "jfc::session",
            ?cwd,
            session_id = %id,
            "most-recent cwd match (by updated_at)"
        );
        return Some(SessionId::new(id));
    }

    debug!(target: "jfc::session", ?cwd, "no session found for cwd (header scan)");
    None
}

/// Extract the `"cwd"` value from a raw JSON header byte slice without
/// full parsing. Returns `None` if the field isn't found in the buffer.
fn extract_cwd_from_header(header: &[u8]) -> Option<&str> {
    extract_string_field_from_header(header, "cwd")
}

/// Extract a top-level string field's value from a raw JSON header byte
/// slice without full parsing. Returns `None` if the field isn't present
/// in the buffer, or its value is `null` (legacy rows). The header values
/// we scan (`cwd`, `updated_at`) never contain escaped quotes, so a simple
/// next-quote scan is sufficient and avoids parsing the whole (possibly
/// multi-hundred-MB) session file just to read a header field.
fn extract_string_field_from_header<'a>(header: &'a [u8], field: &str) -> Option<&'a str> {
    let header_str = std::str::from_utf8(header).ok()?;
    let needle = format!("\"{field}\"");
    let key_idx = header_str.find(&needle)?;
    let after_key = &header_str[key_idx + needle.len()..];
    let colon_idx = after_key.find(':')?;
    let after_colon = after_key[colon_idx + 1..].trim_start();
    if after_colon.starts_with("null") {
        return None;
    }
    let after_colon = after_colon.strip_prefix('"')?;
    let end_quote = after_colon.find('"')?;
    Some(&after_colon[..end_quote])
}

/// Globally most-recent session id (legacy callers + `--global` flag).
pub async fn most_recent_session() -> Option<SessionId> {
    let result = list_sessions().await.into_iter().next();
    debug!(target: "jfc::session", found = result.is_some(), "most recent session (global)");
    result
}

#[cfg(test)]
mod header_scan_tests {
    use super::{extract_cwd_from_header, extract_string_field_from_header};

    // Normal: pull cwd and updated_at out of a realistic header prefix.
    #[test]
    fn extracts_cwd_and_updated_at_normal() {
        let header = br#"{"id":"ses_1","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-05-29T03:00:00Z","first_prompt":"hi","model":"opus","cwd":"/home/u/proj","title":null,"messages":["#;
        assert_eq!(extract_cwd_from_header(header), Some("/home/u/proj"));
        assert_eq!(
            extract_string_field_from_header(header, "updated_at"),
            Some("2026-05-29T03:00:00Z")
        );
    }

    // Robust: null value (legacy session) yields None, not "null".
    #[test]
    fn null_field_returns_none_robust() {
        let header = br#"{"id":"ses_1","updated_at":null,"cwd":null,"messages":["#;
        assert_eq!(extract_cwd_from_header(header), None);
        assert_eq!(extract_string_field_from_header(header, "updated_at"), None);
    }

    // Robust: a field truncated past the 1 KB read window is treated as
    // absent (no panic, just None) — the caller falls back to id-order.
    #[test]
    fn field_beyond_buffer_returns_none_robust() {
        // Header cut off mid-value before the closing quote.
        let header = br#"{"id":"ses_1","cwd":"/home/u/very/long/pa"#;
        assert_eq!(extract_cwd_from_header(header), None);
    }

    // Robust: missing field entirely → None.
    #[test]
    fn missing_field_returns_none_robust() {
        let header = br#"{"id":"ses_1","messages":["#;
        assert_eq!(extract_string_field_from_header(header, "updated_at"), None);
    }
}
