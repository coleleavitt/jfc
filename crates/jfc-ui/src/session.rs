use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::ids::SessionId;
use crate::types::{
    ChatMessage, DiffHunk, DiffLine, DiffLineKind, DiffView, LargeText, MessagePart,
    ReplacementMode, Role, TaskInput, TaskLifecycle, TaskStatusPart, ToolCall, ToolInput, ToolKind,
    ToolOutput, ToolStatus, validate_turn_invariants,
};

/// Session metadata stored alongside messages
#[derive(Serialize, Deserialize)]
pub struct SerializedSession {
    pub id: String,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub first_prompt: Option<String>,
    /// The model ID that was active when this session was last saved.
    /// Restored on `/continue` so the user stays on the same model.
    #[serde(default)]
    pub model: Option<String>,
    /// Working directory the session was created in. Used by
    /// `/continue` and the sidebar picker to filter sessions to those
    /// belonging to the current project. Mirrors codex-rs (cli/src/
    /// main.rs:285,311 — `--show-all` toggle) and v126 cli.js:47254
    /// (`listSessions(cwd)` filters by cwd prefix). Sessions saved
    /// before this field landed deserialize with `None` and remain
    /// visible only via `--global` / "show all" toggles. Also drives
    /// the cwd-mismatch warning on resume (codex-rs
    /// `tui/src/session_resume.rs:99-111`).
    #[serde(default)]
    pub cwd: Option<String>,
    /// User-set title (via future `/rename` slash). Falls back to
    /// `first_prompt` for display when None. Mirrors v126's title
    /// precedence: customTitle → aiTitle → firstPrompt → id-slice
    /// (cli.js:39786, 47183-47184).
    #[serde(default)]
    pub title: Option<String>,
    pub messages: Vec<SerializedMessage>,
}

#[derive(Serialize, Deserialize)]
pub struct SerializedMessage {
    pub role: String,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub cost_tier: Option<String>,
    #[serde(default)]
    pub elapsed: Option<String>,
    /// Per-message cumulative usage at the END of this assistant turn.
    /// Mirrors v126's per-message `usage` field (cli.js:416673,
    /// 197282-197294) — on resume the picker walks the messages
    /// backwards to find the last `Some(usage)` and uses that to seed
    /// the Context gauge, so the user doesn't see "0 tokens / 0%" on a
    /// resumed million-token session. Optional + serde(default) so old
    /// session files (no usage) still load.
    #[serde(default)]
    pub usage: Option<crate::types::ModelUsage>,
    pub parts: Vec<SerializedPart>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerializedPart {
    Text {
        content: String,
    },
    Reasoning {
        content: String,
    },
    Tool {
        id: String,
        kind: String,
        status: String,
        /// Legacy on-disk field — kept for backward compatibility.
        /// Old session files wrote `is_collapsed: bool`; new writes
        /// emit it as `tc.display.is_collapsed()`. Loading
        /// reconstructs the new `ToolDisplayState`: `is_collapsed=true`
        /// → `Collapsed`, otherwise → `Default { pinned: false }`.
        /// `expanded` and `pinned` were never persisted (stale on
        /// reload), so the migration here is one-way and lossless for
        /// the only state we ever stored.
        #[serde(default)]
        is_collapsed: bool,
        /// Optional + serde(default): old session files (pre-tool-input
        /// schema landed) wrote tool entries without an `input` field.
        /// The deserializer used to fail the entire session if any
        /// single Tool entry was missing this — surfacing as
        /// "missing field `input`" warnings in the log and the picker
        /// silently dropping that session. Now we tolerate the gap and
        /// reconstruct an unknown-input stub at message-rebuild time.
        #[serde(default)]
        input: Option<SerializedToolInput>,
        #[serde(default)]
        output: Option<SerializedToolOutput>,
    },
    TaskStatus {
        task_id: String,
        description: String,
        status: String,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        elapsed_ms: Option<u64>,
    },
    CompactBoundary {
        pre_tokens: usize,
    },
    /// Persisted advisor reply (see `crate::advisor`). Preserves the text so
    /// resuming a session that had `/advisor` invocations renders them with
    /// the same italic/secondary styling.
    Advisor {
        content: String,
    },
}

/// Full tool input serialization - preserves all fields for proper resume
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerializedToolInput {
    Edit {
        file_path: String,
        old_string: String,
        new_string: String,
        #[serde(default)]
        replace_all: bool,
    },
    Write {
        file_path: String,
        content: String,
    },
    Read {
        file_path: String,
        #[serde(default)]
        offset: Option<u64>,
        #[serde(default)]
        limit: Option<u64>,
    },
    Bash {
        command: String,
        #[serde(default)]
        timeout: Option<u64>,
        #[serde(default)]
        workdir: Option<String>,
    },
    Glob {
        pattern: String,
        #[serde(default)]
        path: Option<String>,
    },
    Grep {
        pattern: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        glob: Option<String>,
        #[serde(default)]
        output_mode: Option<String>,
    },
    Search {
        query: String,
        #[serde(default)]
        path: Option<String>,
    },
    ApplyPatch {
        patch: String,
    },
    Task {
        description: String,
        prompt: String,
        #[serde(default)]
        subagent_type: Option<String>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        run_in_background: bool,
        #[serde(default)]
        model: Option<String>,
    },
    TaskCreate {
        subject: String,
        description: String,
        #[serde(default)]
        active_form: Option<String>,
        #[serde(default)]
        blocked_by: Vec<String>,
    },
    TaskUpdate {
        task_id: String,
        #[serde(default)]
        status: Option<String>,
        #[serde(default)]
        subject: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        owner: Option<String>,
    },
    TaskList {
        #[serde(default)]
        status_filter: Option<String>,
        #[serde(default)]
        owner_filter: Option<String>,
    },
    TaskDone {
        task_id: String,
    },
    TaskGet {
        task_id: String,
    },
    Skill {
        name: String,
        #[serde(default)]
        args: Option<String>,
    },
    MemoryCreate {
        level: String,
        memory_type: String,
        scope: String,
        body: String,
    },
    MemoryDelete {
        path: String,
    },
    Lsp {
        kind: String,
        file: String,
        line: u32,
        column: u32,
    },
    PushNotification {
        message: String,
        #[serde(default)]
        title: Option<String>,
    },
    RemoteTrigger {
        trigger_id: String,
        #[serde(default)]
        payload: Option<serde_json::Value>,
    },
    EnterPlanMode {
        reason: String,
    },
    EnterWorktree {
        name: String,
        #[serde(default)]
        branch: Option<String>,
    },
    ExitWorktree,
    NotebookRead {
        path: String,
    },
    NotebookEdit {
        path: String,
        cell_id: String,
        new_source: String,
        #[serde(default)]
        edit_mode: Option<String>,
    },
    Generic {
        summary: String,
    },
}

/// Full tool output serialization - preserves content for proper resume
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SerializedToolOutput {
    Text {
        content: String,
    },
    LargeText {
        content: String,
        line_count: usize,
        byte_count: usize,
    },
    Diff {
        file_path: String,
        additions: usize,
        deletions: usize,
        hunks: Vec<SerializedDiffHunk>,
    },
    FileContent {
        path: String,
        content: String,
        language: String,
    },
    Command {
        stdout: String,
        stderr: String,
        #[serde(default)]
        exit_code: Option<i32>,
    },
    FileList {
        files: Vec<String>,
    },
    Empty,
}

/// Custom deserializer that handles both:
/// - The new internally-tagged enum format: `{"type": "text", "content": "..."}`
/// - The old plain-string format from May 4 sessions: `"some output text"`
/// - null values (treated as Empty)
impl<'de> serde::Deserialize<'de> for SerializedToolOutput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;
        use serde_json::Value;

        let value = Value::deserialize(deserializer)?;
        match &value {
            // Old format: plain string → Text
            Value::String(s) => Ok(SerializedToolOutput::Text { content: s.clone() }),
            // null → Empty
            Value::Null => Ok(SerializedToolOutput::Empty),
            // New format: object with "type" tag
            Value::Object(_) => {
                // Re-deserialize using the tagged enum logic
                #[derive(Deserialize)]
                #[serde(tag = "type", rename_all = "snake_case")]
                enum Inner {
                    Text {
                        content: String,
                    },
                    LargeText {
                        content: String,
                        line_count: usize,
                        byte_count: usize,
                    },
                    Diff {
                        file_path: String,
                        additions: usize,
                        deletions: usize,
                        hunks: Vec<SerializedDiffHunk>,
                    },
                    FileContent {
                        path: String,
                        content: String,
                        language: String,
                    },
                    Command {
                        stdout: String,
                        stderr: String,
                        #[serde(default)]
                        exit_code: Option<i32>,
                    },
                    FileList {
                        files: Vec<String>,
                    },
                    Empty,
                }
                let inner: Inner = serde_json::from_value(value).map_err(de::Error::custom)?;
                Ok(match inner {
                    Inner::Text { content } => SerializedToolOutput::Text { content },
                    Inner::LargeText {
                        content,
                        line_count,
                        byte_count,
                    } => SerializedToolOutput::LargeText {
                        content,
                        line_count,
                        byte_count,
                    },
                    Inner::Diff {
                        file_path,
                        additions,
                        deletions,
                        hunks,
                    } => SerializedToolOutput::Diff {
                        file_path,
                        additions,
                        deletions,
                        hunks,
                    },
                    Inner::FileContent {
                        path,
                        content,
                        language,
                    } => SerializedToolOutput::FileContent {
                        path,
                        content,
                        language,
                    },
                    Inner::Command {
                        stdout,
                        stderr,
                        exit_code,
                    } => SerializedToolOutput::Command {
                        stdout,
                        stderr,
                        exit_code,
                    },
                    Inner::FileList { files } => SerializedToolOutput::FileList { files },
                    Inner::Empty => SerializedToolOutput::Empty,
                })
            }
            // Anything else → treat as Empty
            _ => Ok(SerializedToolOutput::Empty),
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SerializedDiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub header: String,
    pub lines: Vec<SerializedDiffLine>,
}

#[derive(Serialize, Deserialize)]
pub struct SerializedDiffLine {
    pub kind: String,
    #[serde(default)]
    pub old_line: Option<usize>,
    #[serde(default)]
    pub new_line: Option<usize>,
    pub content: String,
}

pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("sessions")
}

pub fn generate_session_id() -> SessionId {
    let now = chrono::Utc::now();
    let id = SessionId::new(format!("ses_{}", now.format("%Y%m%d_%H%M%S")));
    debug!(target: "jfc::session", %id, "generated session id");
    id
}

/// Extract the first meaningful user prompt from messages for display in session list
fn extract_first_prompt(messages: &[ChatMessage]) -> Option<String> {
    messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| {
            m.parts.iter().find_map(|p| match p {
                MessagePart::Text(t) if !t.trim().is_empty() => {
                    let trimmed = t.trim();
                    // Truncate long prompts for display (floor to char boundary)
                    if trimmed.len() > 100 {
                        let boundary = trimmed.floor_char_boundary(100);
                        Some(format!("{}…", &trimmed[..boundary]))
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                _ => None,
            })
        })
}

#[tracing::instrument(target = "jfc::session", skip(messages), fields(n = messages.len()))]
pub async fn save_session(
    session_id: &SessionId,
    messages: &[ChatMessage],
    cwd: Option<&str>,
    model: Option<&str>,
) {
    // Surface invariant breakage at the save boundary. We deliberately
    // do NOT block the save — corrupt state is itself debugging signal,
    // and silently dropping the write would hide the very symptom we
    // want to study post-mortem. The warn lands in the trace log with
    // enough context (session id, error variant) to reconstruct what
    // shape went wrong.
    let session_id_str = session_id.as_str();
    if let Err(err) = validate_turn_invariants(messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count = messages.len(),
            "save_session: turn-invariant violation (saving anyway for forensics)"
        );
    }
    let dir = sessions_dir();
    if tokio::fs::create_dir_all(&dir).await.is_err() {
        warn!(target: "jfc::session", "failed to create sessions directory");
        return;
    }

    let now = chrono::Utc::now();
    let path = dir.join(format!("{session_id_str}.json"));

    // Try to load existing session to preserve created_at + cwd + title
    // (so resaving doesn't reset them on every turn). cwd is pinned at
    // first save; subsequent saves don't migrate the session even if the
    // user `cd`s elsewhere — that would conflate two projects' work into
    // one session, and would also defeat the cwd-mismatch warning on
    // resume (codex-rs `tui/src/session_resume.rs:99-111`).
    let prior = tokio::fs::read_to_string(&path)
        .await
        .ok()
        .and_then(|content| serde_json::from_str::<SerializedSession>(&content).ok());
    let created_at = prior
        .as_ref()
        .map(|s| s.created_at.clone())
        .unwrap_or_else(|| now.to_rfc3339());
    // Precedence: prior session's cwd (immutable for session lifetime) →
    // explicit `cwd` arg from caller → current_dir() fallback.
    let stored_cwd = prior
        .as_ref()
        .and_then(|s| s.cwd.clone())
        .or_else(|| cwd.map(str::to_owned))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        });
    let title = prior.as_ref().and_then(|s| s.title.clone());
    let stored_model = model
        .map(str::to_owned)
        .or_else(|| prior.as_ref().and_then(|s| s.model.clone()));

    // Drop any queued-prompt placeholders before serializing. They're a
    // runtime-only construct used to render "⏳ I queued this" in the
    // transcript; persisting them would make resume re-display unsent
    // prompts and (worse) `recompute_token_estimate` count their bytes
    // against the context budget on the next launch.
    let serialized = SerializedSession {
        id: session_id_str.to_owned(),
        created_at,
        updated_at: Some(now.to_rfc3339()),
        first_prompt: extract_first_prompt(messages),
        model: stored_model,
        cwd: stored_cwd,
        title,
        messages: messages
            .iter()
            .filter(|m| !m.queued)
            .map(serialize_message)
            .collect(),
    };

    if let Ok(json) = serde_json::to_string_pretty(&serialized) {
        let _ = tokio::fs::write(&path, json).await;
        info!(target: "jfc::session", session_id = session_id_str, message_count = messages.len(), path = %path.display(), "session saved");
    } else {
        warn!(target: "jfc::session", session_id = session_id_str, "failed to serialize session");
    }
}

pub async fn load_session(session_id: &SessionId) -> Option<Vec<ChatMessage>> {
    let session_id_str = session_id.as_str();
    debug!(target: "jfc::session", session_id = session_id_str, "loading session");
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let session: SerializedSession = match serde_json::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            warn!(target: "jfc::session", session_id = session_id_str, error = %e, "failed to parse session file");
            return None;
        }
    };
    let message_count = session.messages.len();
    let messages: Vec<ChatMessage> = session
        .messages
        .into_iter()
        .map(deserialize_message)
        .collect();
    // Record any pre-existing invariant violation BEFORE callers run
    // their own sanitizers. The plan-continuation phantom-assistant
    // bug only surfaced after the renderer composed two layers of
    // truth — the validator gives us a single tracing line that says
    // "this session arrived broken from disk."
    if let Err(err) = validate_turn_invariants(&messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count,
            "load_session: persisted transcript violates turn invariants"
        );
    }
    debug!(target: "jfc::session", session_id = session_id_str, message_count, "session loaded");
    Some(messages)
}

/// Load session messages AND the model that was active. Used by `/continue`
/// to restore the model selection.
pub async fn load_session_with_model(
    session_id: &SessionId,
) -> Option<(Vec<ChatMessage>, Option<String>)> {
    let session_id_str = session_id.as_str();
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let session: SerializedSession = serde_json::from_str(&content).ok()?;
    let model = session.model.clone();
    let messages: Vec<ChatMessage> = session
        .messages
        .into_iter()
        .map(deserialize_message)
        .collect();
    if let Err(err) = validate_turn_invariants(&messages) {
        warn!(
            target: "jfc::session::invariants",
            session_id = session_id_str,
            error = %err,
            message_count = messages.len(),
            "load_session_with_model: persisted transcript violates turn invariants"
        );
    }
    Some((messages, model))
}

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
            warn!(target: "jfc::session", session_id = session_id_str, error = %e, "failed to parse session metadata");
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
    let metas = futures::future::join_all(ids.iter().map(|id| load_session_metadata(id))).await;
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
pub async fn most_recent_session_for_cwd(cwd: Option<&str>) -> Option<SessionId> {
    let result = list_sessions_filtered(cwd)
        .await
        .into_iter()
        .next()
        .map(|s| s.id);
    debug!(target: "jfc::session", ?cwd, found = result.is_some(), "most recent session for cwd");
    result
}

/// Globally most-recent session id (legacy callers + `--global` flag).
pub async fn most_recent_session() -> Option<SessionId> {
    let result = list_sessions().await.into_iter().next();
    debug!(target: "jfc::session", found = result.is_some(), "most recent session (global)");
    result
}

/// Set the user-defined title on a session (`/rename` slash). Returns
/// silently on I/O failures — title is cosmetic, shouldn't block the
/// chat. Mirrors v126's `customTitle` field (cli.js:39786) which sits
/// atop the title precedence chain.
pub async fn set_session_title(session_id: &SessionId, title: &str) {
    let session_id_str = session_id.as_str();
    debug!(target: "jfc::session", session_id = session_id_str, "setting session title");
    let path = sessions_dir().join(format!("{session_id_str}.json"));
    let Ok(content) = tokio::fs::read_to_string(&path).await else {
        warn!(target: "jfc::session", session_id = session_id_str, "cannot read session file for title update");
        return;
    };
    let Ok(mut session) = serde_json::from_str::<SerializedSession>(&content) else {
        warn!(target: "jfc::session", session_id = session_id_str, "cannot parse session file for title update");
        return;
    };
    session.title = Some(title.to_owned());
    if let Ok(json) = serde_json::to_string_pretty(&session) {
        let _ = tokio::fs::write(&path, json).await;
        info!(target: "jfc::session", session_id = session_id_str, "session title updated");
    }
}

fn serialize_message(msg: &ChatMessage) -> SerializedMessage {
    SerializedMessage {
        role: match msg.role {
            Role::User => "user".into(),
            Role::Assistant => "assistant".into(),
        },
        agent_name: msg.agent_name.clone(),
        model_name: msg.model_name.clone(),
        cost_tier: msg.cost_tier.clone(),
        elapsed: msg.elapsed.clone(),
        usage: msg.usage.clone(),
        parts: msg.parts.iter().map(serialize_part).collect(),
    }
}

fn serialize_part(part: &MessagePart) -> SerializedPart {
    match part {
        MessagePart::Text(t) => SerializedPart::Text { content: t.clone() },
        MessagePart::Reasoning(t) => SerializedPart::Reasoning { content: t.clone() },
        MessagePart::Tool(tc) => SerializedPart::Tool {
            id: tc.id.as_str().to_owned(),
            kind: tc.kind.label().to_owned(),
            status: serialize_tool_status(tc.status),
            // Persist only the teaser bit — the only display state
            // worth surviving a session reload (see
            // `SerializedPart::Tool::is_collapsed` doc comment).
            is_collapsed: tc.display.is_collapsed(),
            input: Some(serialize_tool_input(&tc.input)),
            output: Some(serialize_tool_output(&tc.output)),
        },
        MessagePart::TaskStatus(ts) => SerializedPart::TaskStatus {
            task_id: ts.task_id.as_str().to_owned(),
            description: ts.description.clone(),
            status: serialize_task_lifecycle(ts.status),
            summary: ts.summary.clone(),
            error: ts.error.clone(),
            elapsed_ms: ts.elapsed_ms,
        },
        MessagePart::CompactBoundary { pre_tokens } => SerializedPart::CompactBoundary {
            pre_tokens: *pre_tokens,
        },
        MessagePart::Advisor(t) => SerializedPart::Advisor { content: t.clone() },
    }
}

fn serialize_tool_status(status: ToolStatus) -> String {
    // ToolStatus is now an alias for ExecutionStatus, which has two
    // extra variants (Idle, Cancelled) that tools didn't historically
    // produce. Map them to the closest tool-shaped value so legacy
    // session readers (which only know about pending/running/complete/
    // failed) still see something sensible:
    //   - Idle → "running" (the tool is still in flight, just quiet)
    //   - Cancelled → "failed" (denied / abandoned tools surface as
    //     failures from the model's perspective)
    // Wire format remains "complete" for Completed (NOT "completed")
    // — preserves backward compatibility with on-disk session JSON.
    match status {
        ToolStatus::Pending => "pending".into(),
        ToolStatus::Running | ToolStatus::Idle => "running".into(),
        ToolStatus::Completed => "complete".into(),
        ToolStatus::Failed | ToolStatus::Cancelled => "failed".into(),
    }
}

fn serialize_task_lifecycle(status: TaskLifecycle) -> String {
    match status {
        TaskLifecycle::Pending => "pending".into(),
        TaskLifecycle::Running => "running".into(),
        TaskLifecycle::Idle => "idle".into(),
        TaskLifecycle::Completed => "completed".into(),
        TaskLifecycle::Failed => "failed".into(),
        TaskLifecycle::Cancelled => "cancelled".into(),
    }
}

fn serialize_tool_input(input: &ToolInput) -> SerializedToolInput {
    match input {
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            replacement,
        } => SerializedToolInput::Edit {
            file_path: file_path.clone(),
            old_string: old_string.clone(),
            new_string: new_string.clone(),
            replace_all: replacement.replace_all(),
        },
        ToolInput::Write { file_path, content } => SerializedToolInput::Write {
            file_path: file_path.clone(),
            content: content.clone(),
        },
        ToolInput::Read {
            file_path,
            offset,
            limit,
        } => SerializedToolInput::Read {
            file_path: file_path.clone(),
            offset: *offset,
            limit: *limit,
        },
        ToolInput::Bash {
            command,
            timeout,
            workdir,
        } => SerializedToolInput::Bash {
            command: command.clone(),
            timeout: *timeout,
            workdir: workdir.clone(),
        },
        ToolInput::Glob { pattern, path } => SerializedToolInput::Glob {
            pattern: pattern.clone(),
            path: path.clone(),
        },
        ToolInput::Grep {
            pattern,
            path,
            glob,
            output_mode,
        } => SerializedToolInput::Grep {
            pattern: pattern.clone(),
            path: path.clone(),
            glob: glob.clone(),
            output_mode: output_mode.clone(),
        },
        ToolInput::Search { query, path } => SerializedToolInput::Search {
            query: query.clone(),
            path: path.clone(),
        },
        ToolInput::ApplyPatch { patch } => SerializedToolInput::ApplyPatch {
            patch: patch.clone(),
        },
        ToolInput::Task(ti) => SerializedToolInput::Task {
            description: ti.description.clone(),
            prompt: ti.prompt.clone(),
            subagent_type: ti.subagent_type.clone(),
            category: ti.category.clone(),
            run_in_background: ti.run_in_background,
            model: ti.model.clone(),
        },
        ToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            ..
        } => SerializedToolInput::TaskCreate {
            subject: subject.clone(),
            description: description.clone(),
            active_form: active_form.clone(),
            blocked_by: blocked_by.clone(),
        },
        ToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            ..
        } => SerializedToolInput::TaskUpdate {
            task_id: task_id.clone(),
            status: status.clone(),
            subject: subject.clone(),
            description: description.clone(),
            owner: owner.clone(),
        },
        ToolInput::TaskList {
            status_filter,
            owner_filter,
        } => SerializedToolInput::TaskList {
            status_filter: status_filter.clone(),
            owner_filter: owner_filter.clone(),
        },
        ToolInput::TaskDone { task_id } => SerializedToolInput::TaskDone {
            task_id: task_id.clone(),
        },
        ToolInput::TaskGet { task_id } => SerializedToolInput::TaskGet {
            task_id: task_id.clone(),
        },
        ToolInput::TaskValidate => SerializedToolInput::Generic {
            summary: "TaskValidate".to_string(),
        },
        ToolInput::Skill { name, args } => SerializedToolInput::Skill {
            name: name.clone(),
            args: args.clone(),
        },
        ToolInput::ToolSearch { query, limit } => SerializedToolInput::Generic {
            summary: serde_json::json!({
                "query": query,
                "limit": limit,
            })
            .to_string(),
        },
        ToolInput::ToolSuggest { intent, limit } => SerializedToolInput::Generic {
            summary: serde_json::json!({
                "intent": intent,
                "limit": limit,
            })
            .to_string(),
        },
        ToolInput::MemoryCreate {
            level,
            memory_type,
            scope,
            body,
        } => SerializedToolInput::MemoryCreate {
            level: level.clone(),
            memory_type: memory_type.clone(),
            scope: scope.clone(),
            body: body.clone(),
        },
        ToolInput::MemoryDelete { path } => {
            SerializedToolInput::MemoryDelete { path: path.clone() }
        }
        ToolInput::TeamCreate {
            team_name,
            description,
        } => SerializedToolInput::Generic {
            // Preserve the description in the summary so a resumed
            // session shows what the team was created for, not just
            // its name. Previously `description` was destructured but
            // never used — silent data loss on resume.
            summary: match description.as_deref().filter(|s| !s.is_empty()) {
                Some(d) => format!("TeamCreate: {team_name} — {d}"),
                None => format!("TeamCreate: {team_name}"),
            },
        },
        ToolInput::TeamDelete => SerializedToolInput::Generic {
            summary: "TeamDelete".to_owned(),
        },
        ToolInput::SendMessage { to, summary, .. } => SerializedToolInput::Generic {
            summary: format!(
                "SendMessage to {to}: {}",
                summary.as_deref().unwrap_or("(message)")
            ),
        },
        ToolInput::TeamMemberMode { member_name, mode } => SerializedToolInput::Generic {
            summary: format!("TeamMemberMode {member_name}: {mode}"),
        },
        ToolInput::GraphQuery {
            query, max_tokens, ..
        } => SerializedToolInput::Generic {
            summary: format!(
                "GraphQuery(budget={}): {}",
                max_tokens.unwrap_or(4000),
                query
            ),
        },
        ToolInput::RunCoverage { lcov_path, .. } => SerializedToolInput::Generic {
            summary: format!("RunCoverage({})", lcov_path.as_deref().unwrap_or("auto")),
        },
        ToolInput::SymbolEdit { handle, .. } => SerializedToolInput::Generic {
            summary: format!("SymbolEdit: {handle}"),
        },
        ToolInput::PostBounty {
            description,
            budget,
            ..
        } => SerializedToolInput::Generic {
            summary: format!(
                "PostBounty({budget} tok): {}",
                description.chars().take(60).collect::<String>()
            ),
        },
        ToolInput::MarketStatus { bounty_id } => SerializedToolInput::Generic {
            summary: match bounty_id {
                Some(id) => format!("MarketStatus: {id}"),
                None => "MarketStatus".into(),
            },
        },
        ToolInput::RunBounty { bounty_id, .. } => SerializedToolInput::Generic {
            summary: format!("RunBounty: {bounty_id}"),
        },
        ToolInput::ExitPlanMode { plan } => SerializedToolInput::Generic {
            summary: format!("ExitPlanMode: {plan}"),
        },
        ToolInput::MultiEdit { file_path, edits } => SerializedToolInput::Generic {
            summary: format!(
                "MultiEdit: {file_path} ({} edits)",
                edits.as_array().map(|a| a.len()).unwrap_or(0)
            ),
        },
        ToolInput::AskUserQuestion { question, .. } => SerializedToolInput::Generic {
            summary: format!("AskUserQuestion: {question}"),
        },
        ToolInput::WebFetch { url, .. } => SerializedToolInput::Generic {
            summary: format!("WebFetch: {url}"),
        },
        ToolInput::WebSearch { query, .. } => SerializedToolInput::Generic {
            summary: format!("WebSearch: {query}"),
        },
        ToolInput::Mcp { name, arguments } => SerializedToolInput::Generic {
            summary: format!("{name}: {arguments}"),
        },
        ToolInput::CronCreate {
            schedule,
            description,
            ..
        } => SerializedToolInput::Generic {
            summary: format!("CronCreate({schedule}): {description}"),
        },
        ToolInput::CronList => SerializedToolInput::Generic {
            summary: "CronList".into(),
        },
        ToolInput::CronDelete { id } => SerializedToolInput::Generic {
            summary: format!("CronDelete: {id}"),
        },
        ToolInput::ScheduleWakeup {
            delay_seconds,
            reason,
            ..
        } => SerializedToolInput::Generic {
            summary: format!("ScheduleWakeup({delay_seconds}s): {reason}"),
        },
        ToolInput::Monitor { command, until } => SerializedToolInput::Generic {
            summary: format!(
                "Monitor `{}` until /{until}/",
                command.chars().take(40).collect::<String>()
            ),
        },
        ToolInput::Lsp {
            kind, file, line, ..
        } => SerializedToolInput::Generic {
            summary: format!("LSP {kind} {file}:{line}"),
        },
        ToolInput::PushNotification { message, title } => SerializedToolInput::Generic {
            summary: match title {
                Some(t) => format!("PushNotification: {t}: {message}"),
                None => format!("PushNotification: {message}"),
            },
        },
        ToolInput::RemoteTrigger { trigger_id, .. } => SerializedToolInput::Generic {
            summary: format!("RemoteTrigger: {trigger_id}"),
        },
        ToolInput::EnterPlanMode { reason } => SerializedToolInput::Generic {
            summary: format!("EnterPlanMode: {reason}"),
        },
        ToolInput::EnterWorktree { name, branch } => SerializedToolInput::Generic {
            summary: match branch {
                Some(b) => format!("EnterWorktree: {name} ({b})"),
                None => format!("EnterWorktree: {name}"),
            },
        },
        ToolInput::ExitWorktree => SerializedToolInput::Generic {
            summary: "ExitWorktree".into(),
        },
        ToolInput::NotebookRead { path } => SerializedToolInput::Generic {
            summary: format!("NotebookRead: {path}"),
        },
        ToolInput::NotebookEdit {
            path,
            cell_id,
            edit_mode,
            ..
        } => SerializedToolInput::Generic {
            summary: format!(
                "NotebookEdit({}): {path}#{cell_id}",
                edit_mode.as_deref().unwrap_or("replace"),
            ),
        },
        ToolInput::ScratchpadRead { key } => SerializedToolInput::Generic {
            summary: format!("ScratchpadRead: {key}"),
        },
        ToolInput::ScratchpadWrite { key, .. } => SerializedToolInput::Generic {
            summary: format!("ScratchpadWrite: {key}"),
        },
        ToolInput::Generic { summary } => SerializedToolInput::Generic {
            summary: summary.clone(),
        },
    }
}

fn serialize_tool_output(output: &ToolOutput) -> SerializedToolOutput {
    match output {
        ToolOutput::Text(content) => SerializedToolOutput::Text {
            content: content.clone(),
        },
        ToolOutput::LargeText(lt) => SerializedToolOutput::LargeText {
            content: lt.content.clone(),
            line_count: lt.line_count,
            byte_count: lt.byte_count,
        },
        ToolOutput::Diff(d) => SerializedToolOutput::Diff {
            file_path: d.file_path.clone(),
            additions: d.additions,
            deletions: d.deletions,
            hunks: d.hunks.iter().map(serialize_diff_hunk).collect(),
        },
        ToolOutput::FileContent {
            path,
            content,
            language,
        } => SerializedToolOutput::FileContent {
            path: path.clone(),
            content: content.clone(),
            language: language.clone(),
        },
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => SerializedToolOutput::Command {
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            exit_code: *exit_code,
        },
        ToolOutput::FileList(files) => SerializedToolOutput::FileList {
            files: files.clone(),
        },
        ToolOutput::Empty => SerializedToolOutput::Empty,
    }
}

fn serialize_diff_hunk(hunk: &DiffHunk) -> SerializedDiffHunk {
    SerializedDiffHunk {
        old_start: hunk.old_start,
        new_start: hunk.new_start,
        header: hunk.header.clone(),
        lines: hunk.lines.iter().map(serialize_diff_line).collect(),
    }
}

fn serialize_diff_line(line: &DiffLine) -> SerializedDiffLine {
    SerializedDiffLine {
        kind: match line.kind {
            DiffLineKind::Context => "context".into(),
            DiffLineKind::Added => "added".into(),
            DiffLineKind::Removed => "removed".into(),
        },
        old_line: line.old_line,
        new_line: line.new_line,
        content: line.content.clone(),
    }
}

fn deserialize_message(msg: SerializedMessage) -> ChatMessage {
    let role = if msg.role == "user" {
        Role::User
    } else {
        Role::Assistant
    };
    let parts: Vec<MessagePart> = msg.parts.into_iter().map(deserialize_part).collect();
    ChatMessage {
        role,
        parts,
        agent_name: msg.agent_name,
        model_name: msg.model_name,
        cost_tier: msg.cost_tier,
        elapsed: msg.elapsed,
        usage: msg.usage,
        // Queued is a runtime-only marker — resumed sessions never have
        // unsent queued prompts because drain_queued_prompts runs as
        // part of the turn lifecycle before save_session ever fires.
        queued: false,
        // Attachments (images) are not persisted in session files — they
        // would bloat JSON to hundreds of MB. Default to empty on load.
        attachments: Vec::new(),
    }
}

fn deserialize_part(part: SerializedPart) -> MessagePart {
    match part {
        SerializedPart::Text { content } => MessagePart::Text(content),
        SerializedPart::Reasoning { content } => MessagePart::Reasoning(content),
        SerializedPart::Tool {
            id,
            kind,
            status,
            is_collapsed,
            input,
            output,
        } => MessagePart::Tool(ToolCall {
            id: crate::ids::ToolId::from(id),
            kind: ToolKind::from_name(&kind),
            status: deserialize_tool_status(&status),
            // Tolerate missing input/output on legacy session files.
            // The unknown-input fallback (a no-op Bash entry) lets the
            // resumed transcript render the tool row with whatever
            // chrome we have (id, kind, status) without panicking on a
            // missing field that older writers never produced.
            input: match input {
                Some(i) => deserialize_tool_input(i),
                None => ToolInput::Bash {
                    command: String::new(),
                    timeout: None,
                    workdir: None,
                },
            },
            output: match output {
                Some(o) => deserialize_tool_output(o),
                None => ToolOutput::Empty,
            },
            // Reconstruct the tri-state from the legacy on-disk
            // `is_collapsed` bool. Expanded + pinned were never
            // persisted (storing UI chrome state in the on-disk
            // format would round-trip stale state), so loaded sessions
            // always come back as either Collapsed (huge teaser
            // preserved) or Default. The user can re-expand or re-pin
            // with `o` / Ctrl+O / double-click.
            display: if is_collapsed {
                crate::types::ToolDisplayState::Collapsed
            } else {
                crate::types::ToolDisplayState::DEFAULT
            },
            // elapsed_ms could in principle round-trip, but it's
            // cosmetic — leave None on resume so we don't lock in a
            // stale duration. started_at is meaningless after a
            // reload (would always say "elapsed since session-load").
            elapsed_ms: None,
            started_at: None,
        }),
        SerializedPart::TaskStatus {
            task_id,
            description,
            status,
            summary,
            error,
            elapsed_ms,
        } => MessagePart::TaskStatus(TaskStatusPart {
            task_id: crate::ids::TaskId::from(task_id),
            description,
            status: deserialize_task_lifecycle(&status),
            summary,
            error,
            elapsed_ms,
        }),
        SerializedPart::CompactBoundary { pre_tokens } => {
            MessagePart::CompactBoundary { pre_tokens }
        }
        SerializedPart::Advisor { content } => MessagePart::Advisor(content),
    }
}

fn deserialize_tool_status(status: &str) -> ToolStatus {
    // Backward-compat: legacy sessions wrote "complete" (Tool's
    // pre-unification spelling). Also accept "completed" / "idle" /
    // "cancelled" so a future serializer that emits the canonical
    // ExecutionStatus names stays readable. Falls back to Completed
    // (rather than Pending) on unknown — a tool that landed on disk
    // without a recognized state is almost certainly done by the
    // time a session reload reads it.
    match status {
        "pending" => ToolStatus::Pending,
        "running" => ToolStatus::Running,
        "idle" => ToolStatus::Idle,
        "complete" | "Complete" | "completed" | "Completed" => ToolStatus::Completed,
        "failed" | "Failed" => ToolStatus::Failed,
        "cancelled" | "Cancelled" => ToolStatus::Cancelled,
        _ => ToolStatus::Completed,
    }
}

fn deserialize_task_lifecycle(status: &str) -> TaskLifecycle {
    match status {
        "pending" => TaskLifecycle::Pending,
        "running" => TaskLifecycle::Running,
        "completed" => TaskLifecycle::Completed,
        "failed" => TaskLifecycle::Failed,
        "cancelled" => TaskLifecycle::Cancelled,
        _ => TaskLifecycle::Pending,
    }
}

fn deserialize_tool_input(input: SerializedToolInput) -> ToolInput {
    match input {
        SerializedToolInput::Edit {
            file_path,
            old_string,
            new_string,
            replace_all,
        } => ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            replacement: ReplacementMode::from_replace_all(replace_all),
        },
        SerializedToolInput::Write { file_path, content } => {
            ToolInput::Write { file_path, content }
        }
        SerializedToolInput::Read {
            file_path,
            offset,
            limit,
        } => ToolInput::Read {
            file_path,
            offset,
            limit,
        },
        SerializedToolInput::Bash {
            command,
            timeout,
            workdir,
        } => ToolInput::Bash {
            command,
            timeout,
            workdir,
        },
        SerializedToolInput::Glob { pattern, path } => ToolInput::Glob { pattern, path },
        SerializedToolInput::Grep {
            pattern,
            path,
            glob,
            output_mode,
        } => ToolInput::Grep {
            pattern,
            path,
            glob,
            output_mode,
        },
        SerializedToolInput::Search { query, path } => ToolInput::Search { query, path },
        SerializedToolInput::ApplyPatch { patch } => ToolInput::ApplyPatch { patch },
        SerializedToolInput::Task {
            description,
            prompt,
            subagent_type,
            category,
            run_in_background,
            model,
        } => ToolInput::Task(TaskInput {
            description,
            prompt,
            subagent_type,
            category,
            run_in_background,
            model,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
        }),
        SerializedToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
        } => ToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria: None,
            verification_command: None,
            risk: None,
            parent_id: None,
            kind: None,
        },
        SerializedToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
        } => ToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            acceptance_criteria: None,
            verification_command: None,
            risk: None,
            parent_id: None,
            kind: None,
        },
        SerializedToolInput::TaskList {
            status_filter,
            owner_filter,
        } => ToolInput::TaskList {
            status_filter,
            owner_filter,
        },
        SerializedToolInput::TaskDone { task_id } => ToolInput::TaskDone { task_id },
        SerializedToolInput::TaskGet { task_id } => ToolInput::TaskGet { task_id },
        SerializedToolInput::Skill { name, args } => ToolInput::Skill { name, args },
        SerializedToolInput::MemoryCreate {
            level,
            memory_type,
            scope,
            body,
        } => ToolInput::MemoryCreate {
            level,
            memory_type,
            scope,
            body,
        },
        SerializedToolInput::MemoryDelete { path } => ToolInput::MemoryDelete { path },
        SerializedToolInput::Lsp {
            kind,
            file,
            line,
            column,
        } => ToolInput::Lsp {
            kind,
            file,
            line,
            column,
        },
        SerializedToolInput::PushNotification { message, title } => {
            ToolInput::PushNotification { message, title }
        }
        SerializedToolInput::RemoteTrigger {
            trigger_id,
            payload,
        } => ToolInput::RemoteTrigger {
            trigger_id,
            payload,
        },
        SerializedToolInput::EnterPlanMode { reason } => ToolInput::EnterPlanMode { reason },
        SerializedToolInput::EnterWorktree { name, branch } => {
            ToolInput::EnterWorktree { name, branch }
        }
        SerializedToolInput::ExitWorktree => ToolInput::ExitWorktree,
        SerializedToolInput::NotebookRead { path } => ToolInput::NotebookRead { path },
        SerializedToolInput::NotebookEdit {
            path,
            cell_id,
            new_source,
            edit_mode,
        } => ToolInput::NotebookEdit {
            path,
            cell_id,
            new_source,
            edit_mode,
        },
        SerializedToolInput::Generic { summary } => ToolInput::Generic { summary },
    }
}

fn deserialize_tool_output(output: SerializedToolOutput) -> ToolOutput {
    match output {
        SerializedToolOutput::Text { content } => ToolOutput::Text(content),
        SerializedToolOutput::LargeText {
            content,
            line_count,
            byte_count,
        } => ToolOutput::LargeText(LargeText {
            content,
            line_count,
            byte_count,
        }),
        SerializedToolOutput::Diff {
            file_path,
            additions,
            deletions,
            hunks,
        } => ToolOutput::Diff(DiffView {
            file_path,
            additions,
            deletions,
            hunks: hunks.into_iter().map(deserialize_diff_hunk).collect(),
        }),
        SerializedToolOutput::FileContent {
            path,
            content,
            language,
        } => ToolOutput::FileContent {
            path,
            content,
            language,
        },
        SerializedToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        },
        SerializedToolOutput::FileList { files } => ToolOutput::FileList(files),
        SerializedToolOutput::Empty => ToolOutput::Empty,
    }
}

fn deserialize_diff_hunk(hunk: SerializedDiffHunk) -> DiffHunk {
    DiffHunk {
        old_start: hunk.old_start,
        new_start: hunk.new_start,
        header: hunk.header,
        lines: hunk.lines.into_iter().map(deserialize_diff_line).collect(),
    }
}

fn deserialize_diff_line(line: SerializedDiffLine) -> DiffLine {
    DiffLine {
        kind: match line.kind.as_str() {
            "added" => DiffLineKind::Added,
            "removed" => DiffLineKind::Removed,
            _ => DiffLineKind::Context,
        },
        old_line: line.old_line,
        new_line: line.new_line,
        content: line.content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_tool_input_edit() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::All,
        };
        let serialized = serialize_tool_input(&input);
        let deserialized = deserialize_tool_input(serialized);
        match deserialized {
            ToolInput::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            } => {
                assert_eq!(file_path, "src/main.rs");
                assert_eq!(old_string, "old");
                assert_eq!(new_string, "new");
                assert!(replacement.replace_all());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn roundtrip_tool_output_diff() {
        let output = ToolOutput::Diff(DiffView {
            file_path: "test.rs".into(),
            additions: 5,
            deletions: 3,
            hunks: vec![DiffHunk {
                old_start: 10,
                new_start: 10,
                header: "@@ -10,5 +10,7 @@".into(),
                lines: vec![
                    DiffLine {
                        kind: DiffLineKind::Removed,
                        old_line: Some(10),
                        new_line: None,
                        content: "old line".into(),
                    },
                    DiffLine {
                        kind: DiffLineKind::Added,
                        old_line: None,
                        new_line: Some(10),
                        content: "new line".into(),
                    },
                ],
            }],
        });
        let serialized = serialize_tool_output(&output);
        let deserialized = deserialize_tool_output(serialized);
        match deserialized {
            ToolOutput::Diff(d) => {
                assert_eq!(d.file_path, "test.rs");
                assert_eq!(d.additions, 5);
                assert_eq!(d.deletions, 3);
                assert_eq!(d.hunks.len(), 1);
                assert_eq!(d.hunks[0].lines.len(), 2);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn cwd_mismatch_returns_none_when_match_normal() {
        // Same paths -> no warning. The happy case for resume in the
        // same project the session was created in.
        let same = "/home/user/project";
        assert_eq!(cwd_mismatch_message(Some(same), same), None);
    }

    #[test]
    fn cwd_mismatch_returns_message_when_different_normal() {
        // Different paths -> Some, message contains both. Mirrors
        // codex-rs `session_resume.rs:99-111`.
        let session_cwd = "/home/user/project-a";
        let current_cwd = "/home/user/project-b";
        let msg = cwd_mismatch_message(Some(session_cwd), current_cwd)
            .expect("differing paths should produce a warning");
        assert!(
            msg.contains(session_cwd),
            "message should contain session cwd: {msg}"
        );
        assert!(
            msg.contains(current_cwd),
            "message should contain current cwd: {msg}"
        );
    }

    #[test]
    fn cwd_mismatch_returns_none_for_legacy_unset_robust() {
        // Legacy sessions written before the cwd field existed have
        // session_cwd=None. We must NOT warn — there's nothing to
        // compare against.
        assert_eq!(cwd_mismatch_message(None, "/anywhere"), None);
    }

    #[test]
    fn cwd_mismatch_returns_none_for_empty_current_robust() {
        // current_cwd="" means `std::env::current_dir()` failed (e.g.
        // the cwd was deleted). We don't have a real path to compare
        // to, so suppress the warning rather than surface noise.
        assert_eq!(cwd_mismatch_message(Some("/home/user/project"), ""), None);
    }

    #[test]
    fn roundtrip_task_status_part() {
        let part = MessagePart::TaskStatus(TaskStatusPart {
            task_id: "t1".into(),
            description: "Test task".into(),
            status: TaskLifecycle::Running,
            summary: Some("Working on it".into()),
            error: None,
            elapsed_ms: Some(1500),
        });
        let serialized = serialize_part(&part);
        let deserialized = deserialize_part(serialized);
        match deserialized {
            MessagePart::TaskStatus(ts) => {
                assert_eq!(ts.task_id, "t1");
                assert_eq!(ts.description, "Test task");
                assert_eq!(ts.status, TaskLifecycle::Running);
                assert_eq!(ts.summary, Some("Working on it".into()));
                assert_eq!(ts.elapsed_ms, Some(1500));
            }
            _ => panic!("wrong variant"),
        }
    }

    fn make_session(id: &str, cwd: Option<&str>, prompt: Option<&str>) -> SessionMetadata {
        SessionMetadata {
            id: SessionId::new(id),
            created_at: "2026-05-04T19:46:49Z".to_owned(),
            updated_at: Some("2026-05-04T19:46:49Z".to_owned()),
            first_prompt: prompt.map(str::to_owned),
            cwd: cwd.map(str::to_owned),
            title: None,
            message_count: 1,
        }
    }

    #[test]
    fn group_splits_current_cwd_first_normal() {
        let sessions = vec![
            make_session("ses_1", Some("/home/c/jfc"), None),
            make_session("ses_2", Some("/home/c/other"), None),
            make_session("ses_3", Some("/home/c/jfc"), None),
            make_session("ses_4", Some("/home/c/other"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, Some("/home/c/jfc"));
        assert_eq!(this_proj.len(), 2);
        assert_eq!(other.len(), 2);
        assert_eq!(this_proj[0].id, "ses_1");
        assert_eq!(this_proj[1].id, "ses_3");
        assert_eq!(other[0].id, "ses_2");
        assert_eq!(other[1].id, "ses_4");
    }

    #[test]
    fn group_legacy_none_cwd_goes_to_other_robust() {
        let sessions = vec![
            make_session("ses_1", None, None),
            make_session("ses_2", Some("/home/c/jfc"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, Some("/home/c/jfc"));
        assert_eq!(this_proj.len(), 1);
        assert_eq!(this_proj[0].id, "ses_2");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].id, "ses_1");
    }

    #[test]
    fn group_no_current_cwd_all_other_robust() {
        let sessions = vec![
            make_session("ses_1", Some("/home/c/jfc"), None),
            make_session("ses_2", None, None),
            make_session("ses_3", Some("/home/c/other"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, None);
        assert!(this_proj.is_empty());
        assert_eq!(other.len(), 3);
    }

    #[test]
    fn group_empty_input_normal() {
        let (this_proj, other) = group_by_cwd(Vec::new(), Some("/home/c/jfc"));
        assert!(this_proj.is_empty());
        assert!(other.is_empty());
    }

    #[test]
    fn group_preserves_order_within_group_normal() {
        let sessions = vec![
            make_session("ses_a", Some("/p1"), None),
            make_session("ses_b", Some("/p2"), None),
            make_session("ses_c", Some("/p1"), None),
            make_session("ses_d", Some("/p2"), None),
            make_session("ses_e", Some("/p1"), None),
        ];
        let (this_proj, other) = group_by_cwd(sessions, Some("/p1"));
        let this_ids: Vec<&str> = this_proj.iter().map(|s| s.id.as_str()).collect();
        let other_ids: Vec<&str> = other.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(this_ids, vec!["ses_a", "ses_c", "ses_e"]);
        assert_eq!(other_ids, vec!["ses_b", "ses_d"]);
    }

    #[test]
    fn picker_row_text_uses_display_title_normal() {
        // Pin: title comes from `first_prompt` when present...
        let with_prompt = make_session("ses_1", None, Some("Refactor compaction"));
        assert_eq!(with_prompt.display_title(), "Refactor compaction");

        // ...and falls back to a formatted timestamp from the id when missing.
        let without_prompt = make_session("ses_20260504_194649", None, None);
        assert_eq!(without_prompt.display_title(), "2026-05-04 19:46");

        // Empty / whitespace prompt → fallback (not an empty title).
        let blank = make_session("ses_20260504_194649", None, Some("   \n  "));
        assert_eq!(blank.display_title(), "2026-05-04 19:46");

        // Long single-line prompts get truncated with an ellipsis so the
        // sidebar row never wraps unpredictably.
        let long = "a".repeat(200);
        let long_session = make_session("ses_1", None, Some(&long));
        let title = long_session.display_title();
        assert!(title.ends_with('…'));
        assert!(title.chars().count() <= 61); // 60 + '…'

        // Multi-line prompts only show the first line.
        let multi = make_session("ses_1", None, Some("first line\nsecond line"));
        assert_eq!(multi.display_title(), "first line");
    }

    #[test]
    fn shorten_cwd_handles_home_basename_and_none() {
        // None → placeholder, never panics.
        assert_eq!(shorten_cwd(None), "—");

        // Non-home absolute → basename (so narrow sidebars stay readable).
        assert_eq!(shorten_cwd(Some("/var/log/something")), "something");
        assert_eq!(shorten_cwd(Some("/var/log/something/")), "something");
        assert_eq!(shorten_cwd(Some("/")), "/");
    }

    #[test]
    fn relative_time_buckets() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-05-04T20:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        // Future / clock skew.
        assert_eq!(relative_time("2026-05-04T20:00:30Z", now), "now");
        // Sub-minute.
        assert_eq!(relative_time("2026-05-04T19:59:30Z", now), "just now");
        // Minutes.
        assert_eq!(relative_time("2026-05-04T19:46:00Z", now), "14m ago");
        // Hours.
        assert_eq!(relative_time("2026-05-04T17:00:00Z", now), "3h ago");
        // Days.
        assert_eq!(relative_time("2026-05-02T20:00:00Z", now), "2d ago");
        // Garbage input → placeholder.
        assert_eq!(relative_time("not a timestamp", now), "—");
    }
}

#[cfg(test)]
mod cwd_filter_tests {
    use super::*;

    fn meta(
        id: &str,
        cwd: Option<&str>,
        title: Option<&str>,
        prompt: Option<&str>,
    ) -> SessionMetadata {
        SessionMetadata {
            id: SessionId::new(id),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: None,
            first_prompt: prompt.map(str::to_owned),
            cwd: cwd.map(str::to_owned),
            title: title.map(str::to_owned),
            message_count: 1,
        }
    }

    #[test]
    fn display_title_prefers_custom_title_normal() {
        // Title precedence (v126 cli.js:39786): customTitle wins.
        let m = meta("s1", None, Some("My session"), Some("hello world"));
        assert_eq!(m.display_title(), "My session");
    }

    #[test]
    fn display_title_falls_through_to_first_prompt_normal() {
        let m = meta("s1", None, None, Some("hello world"));
        assert_eq!(m.display_title(), "hello world");
    }

    #[test]
    fn display_title_truncates_long_first_prompt_normal() {
        // Long prompts get truncated with ellipsis so the picker doesn't blow out.
        let long_prompt: String = "x".repeat(80);
        let m = meta("s1", None, None, Some(&long_prompt));
        let title = m.display_title();
        assert!(title.ends_with('…'), "got: {title}");
        assert_eq!(title.chars().count(), 61);
    }

    #[test]
    fn display_title_empty_prompt_falls_to_id_robust() {
        // Both title + first_prompt empty/None → fall back to
        // format_session_id_timestamp(id) which pretty-prints
        // `ses_YYYYMMDD_HHMMSS`. Non-matching ids pass through verbatim.
        let m = meta("ses_20260504_194649", None, None, None);
        assert_eq!(m.display_title(), "2026-05-04 19:46");

        // Verbatim passthrough for ids that don't match the ses_ pattern.
        let m = meta("abcdef1234567890", None, None, None);
        assert_eq!(m.display_title(), "abcdef1234567890");
    }

    #[test]
    fn display_title_empty_string_title_uses_first_prompt_robust() {
        // Empty-string title should still fall through, not display blank.
        let m = meta("s1", Some(""), Some("hello"), None);
        assert_eq!(m.display_title(), "hello");
    }

    /// Match-logic helper for the cwd filter (extracted for testability).
    fn matches_filter(session_cwd: Option<&str>, target: Option<&str>) -> bool {
        match target {
            None => true,
            Some(t) => session_cwd.is_none_or(|c| c == t),
        }
    }

    #[test]
    fn cwd_filter_no_filter_lets_all_through_normal() {
        assert!(matches_filter(Some("/a"), None));
        assert!(matches_filter(None, None));
    }

    #[test]
    fn cwd_filter_matches_exact_path_normal() {
        assert!(matches_filter(Some("/a"), Some("/a")));
        assert!(!matches_filter(Some("/b"), Some("/a")));
    }

    #[test]
    fn cwd_filter_lets_legacy_unset_cwd_through_robust() {
        // Sessions saved before the cwd field existed have cwd=None.
        // We surface them in any cwd's listing so the user doesn't lose
        // history — they can still `/continue all` to find them.
        assert!(matches_filter(None, Some("/a")));
    }

    // Round-trip: usage attached to an assistant message survives
    // serde → JSON → serde. Without serde wiring on `ModelUsage` the
    // resume gauge would always read 0.
    #[test]
    fn message_usage_round_trips_through_serde_normal() {
        use crate::types::{ChatMessage, MessagePart, ModelUsage, Role};
        let mut msg = ChatMessage::assistant("hi".into());
        msg.usage = Some(ModelUsage {
            input_tokens: 12_345,
            output_tokens: 678,
            cache_read_tokens: 9_000,
            cache_write_tokens: 100,
            cost_usd: None,
        });
        let serialized = serialize_message(&msg);
        let json = serde_json::to_string(&serialized).expect("ser");
        let parsed: SerializedMessage = serde_json::from_str(&json).expect("de");
        let round = deserialize_message(parsed);
        let u = round.usage.expect("usage preserved");
        assert_eq!(u.input_tokens, 12_345);
        assert_eq!(u.output_tokens, 678);
        assert_eq!(u.cache_read_tokens, 9_000);
        assert_eq!(u.cache_write_tokens, 100);
        // Total context tokens = sum of all four (matches v126 W_$).
        assert_eq!(u.total_context_tokens(), 12_345 + 678 + 9_000 + 100);
        // Suppress unused-variant warnings via discriminant check.
        match round.role {
            Role::Assistant => {}
            Role::User => panic!("role should round-trip"),
        }
        assert!(matches!(round.parts.first(), Some(MessagePart::Text(_))));
    }

    // Robust: legacy session JSON without `usage` field still loads,
    // with `usage = None`. Old session files must keep working.
    #[test]
    fn message_without_usage_field_loads_with_none_robust() {
        let legacy = r#"{
            "role": "assistant",
            "parts": [{ "type": "text", "content": "hi" }]
        }"#;
        let parsed: SerializedMessage = serde_json::from_str(legacy).expect("legacy load");
        assert!(parsed.usage.is_none());
        let round = deserialize_message(parsed);
        assert!(round.usage.is_none());
    }
}

// ───────────────────────────────────────────────────────────────────────
// Disk-I/O coverage. Tests in this module mutate `XDG_CONFIG_HOME` so
// `sessions_dir()` points at a per-test tempdir. Serialized through
// ENV_LOCK because cargo test runs them in parallel by default and
// process-global env var state can't be split across threads.
// ───────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod disk_io_tests {
    use super::*;
    use crate::types::{ChatMessage, ToolCall, ToolInput, ToolKind, ToolOutput, ToolStatus};
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// RAII guard that points `XDG_CONFIG_HOME` at a tempdir for the
    /// lifetime of one test. Restores the previous value on drop so a
    /// later test in the same process doesn't see a dangling override.
    struct TempConfigHome {
        _dir: TempDir,
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempConfigHome {
        fn new() -> Self {
            // Poison-tolerant lock: a panic in one test shouldn't take
            // out every subsequent disk-I/O test.
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior = std::env::var("XDG_CONFIG_HOME").ok();
            // Safety: env mutation is serialized through ENV_LOCK.
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", dir.path());
            }
            Self {
                _dir: dir,
                prior,
                _guard: guard,
            }
        }
    }

    impl Drop for TempConfigHome {
        fn drop(&mut self) {
            // Safety: env mutation is serialized through the held guard.
            unsafe {
                match self.prior.take() {
                    Some(prev) => std::env::set_var("XDG_CONFIG_HOME", prev),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    // Normal: round-trip a session through save/load with a few common
    // message variants. Verifies the file lands under sessions_dir() and
    // load_session reconstructs the messages with the same shape.
    #[tokio::test]
    async fn save_load_roundtrip_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![
            ChatMessage::user("first user prompt".into()),
            ChatMessage::assistant("first reply".into()),
        ];
        let id = SessionId::new("ses_20260506_120000");
        save_session(&id, &messages, Some("/tmp/test"), Some("test-model")).await;
        // The file should exist on disk now.
        let path = sessions_dir().join(format!("{}.json", id.as_str()));
        assert!(path.exists(), "session file written");

        let loaded = load_session(&id).await.expect("loadable");
        assert_eq!(loaded.len(), 2);
        assert!(loaded[0].role_is_user());
    }

    // Normal: load_session_with_model returns the persisted model id.
    #[tokio::test]
    async fn load_session_with_model_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![ChatMessage::user("hi".into())];
        let id = SessionId::new("ses_20260506_120100");
        save_session(&id, &messages, Some("/tmp/proj"), Some("opus-4-7")).await;
        let (loaded, model) = load_session_with_model(&id).await.expect("loadable");
        assert_eq!(loaded.len(), 1);
        assert_eq!(model.as_deref(), Some("opus-4-7"));
    }

    // Robust: load_session for a non-existent id returns None instead of
    // panicking.
    #[tokio::test]
    async fn load_session_missing_returns_none_robust() {
        let _g = TempConfigHome::new();
        let missing = SessionId::new("ses_does_not_exist");
        assert!(load_session(&missing).await.is_none());
        assert!(load_session_with_model(&missing).await.is_none());
        assert!(load_session_metadata(&missing).await.is_none());
    }

    // Normal: load_session_metadata reports the same first_prompt and
    // message_count we saved.
    #[tokio::test]
    async fn load_session_metadata_picks_up_first_prompt_normal() {
        let _g = TempConfigHome::new();
        let messages = vec![
            ChatMessage::user("Refactor the renderer".into()),
            ChatMessage::assistant("Plan: …".into()),
        ];
        let id = SessionId::new("ses_20260506_120200");
        save_session(&id, &messages, Some("/tmp/proj"), None).await;
        let meta = load_session_metadata(&id).await.expect("metadata loads");
        assert_eq!(meta.id, id);
        assert_eq!(meta.first_prompt.as_deref(), Some("Refactor the renderer"));
        assert_eq!(meta.message_count, 2);
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/proj"));
    }

    // Robust: corrupted JSON in a session file makes load_session_metadata
    // return None without aborting (parse errors are logged and swallowed).
    #[tokio::test]
    async fn load_session_metadata_handles_corrupted_robust() {
        let _g = TempConfigHome::new();
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("ses_corrupted.json");
        std::fs::write(&path, "{ this is not json").expect("write garbage");
        assert!(
            load_session_metadata(&SessionId::new("ses_corrupted"))
                .await
                .is_none()
        );
    }

    // Normal: list_sessions returns all known ids, newest-first by id sort
    // (which is also chronological for the `ses_YYYYMMDD_HHMMSS` shape).
    #[tokio::test]
    async fn list_sessions_returns_all_sorted_newest_first_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(&SessionId::new("ses_20260101_000000"), &m, None, None).await;
        save_session(&SessionId::new("ses_20260601_000000"), &m, None, None).await;
        save_session(&SessionId::new("ses_20260301_000000"), &m, None, None).await;
        let ids = list_sessions().await;
        assert_eq!(
            ids,
            vec![
                SessionId::new("ses_20260601_000000"),
                SessionId::new("ses_20260301_000000"),
                SessionId::new("ses_20260101_000000"),
            ],
        );
    }

    // Robust: list_sessions on a non-existent sessions directory returns
    // an empty vec rather than panicking.
    #[tokio::test]
    async fn list_sessions_missing_dir_is_empty_robust() {
        let _g = TempConfigHome::new();
        // No save_session calls — directory doesn't even exist yet.
        assert!(list_sessions().await.is_empty());
    }

    // Normal: list_sessions_filtered with a cwd filter returns only that
    // project's sessions plus any legacy (cwd=None) entries.
    #[tokio::test]
    async fn list_sessions_filtered_includes_matching_and_legacy_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(
            &SessionId::new("ses_20260101_000000"),
            &m,
            Some("/projA"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260201_000000"),
            &m,
            Some("/projB"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260301_000000"),
            &m,
            Some("/projA"),
            None,
        )
        .await;

        let only_a = list_sessions_filtered(Some("/projA")).await;
        let ids: Vec<&str> = only_a.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["ses_20260301_000000", "ses_20260101_000000"]);

        // No filter (None) returns all sessions sorted newest-first by
        // updated_at.
        let all = list_sessions_filtered(None).await;
        assert_eq!(all.len(), 3);
    }

    // Normal: most_recent_session_for_cwd returns the newest in the matching
    // project bucket.
    #[tokio::test]
    async fn most_recent_session_for_cwd_returns_top_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(
            &SessionId::new("ses_20260101_000000"),
            &m,
            Some("/proj"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260301_000000"),
            &m,
            Some("/proj"),
            None,
        )
        .await;
        save_session(
            &SessionId::new("ses_20260201_000000"),
            &m,
            Some("/other"),
            None,
        )
        .await;
        let top = most_recent_session_for_cwd(Some("/proj")).await;
        assert_eq!(
            top.as_ref().map(|s| s.as_str()),
            Some("ses_20260301_000000")
        );
    }

    // Robust: most_recent_session (global) returns the newest id regardless
    // of cwd.
    #[tokio::test]
    async fn most_recent_session_global_robust() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("hi".into())];
        save_session(&SessionId::new("ses_20260101_000000"), &m, None, None).await;
        save_session(&SessionId::new("ses_20260601_000000"), &m, None, None).await;
        let top = most_recent_session().await;
        assert_eq!(
            top.as_ref().map(|s| s.as_str()),
            Some("ses_20260601_000000")
        );
    }

    // Normal: set_session_title writes a custom title that overrides
    // first_prompt in display.
    #[tokio::test]
    async fn set_session_title_persists_and_overrides_first_prompt_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("Original prompt".into())];
        let id = SessionId::new("ses_20260506_140000");
        save_session(&id, &m, Some("/tmp"), None).await;
        set_session_title(&id, "My custom title").await;
        let meta = load_session_metadata(&id).await.expect("loaded");
        assert_eq!(meta.title.as_deref(), Some("My custom title"));
        assert_eq!(meta.display_title(), "My custom title");
    }

    // Robust: set_session_title on a non-existent id is a no-op (does not
    // panic, does not create files).
    #[tokio::test]
    async fn set_session_title_missing_session_is_noop_robust() {
        let _g = TempConfigHome::new();
        // Don't save — target doesn't exist.
        let nope = SessionId::new("ses_nope");
        set_session_title(&nope, "ignored").await;
        assert!(load_session_metadata(&nope).await.is_none());
    }

    // Normal: when re-saving an existing session, the original created_at
    // and cwd are preserved (cwd is pinned at first save).
    #[tokio::test]
    async fn save_session_preserves_created_at_and_cwd_normal() {
        let _g = TempConfigHome::new();
        let m = vec![ChatMessage::user("first".into())];
        let id = SessionId::new("ses_20260506_141500");
        save_session(&id, &m, Some("/orig"), None).await;
        let meta1 = load_session_metadata(&id).await.expect("first save");
        let created_at = meta1.created_at.clone();

        // Re-save with a different cwd — should NOT migrate.
        let m2 = vec![
            ChatMessage::user("first".into()),
            ChatMessage::assistant("reply".into()),
        ];
        save_session(&id, &m2, Some("/elsewhere"), None).await;
        let meta2 = load_session_metadata(&id).await.expect("second save");
        assert_eq!(meta2.created_at, created_at);
        assert_eq!(meta2.cwd.as_deref(), Some("/orig"));
        assert_eq!(meta2.message_count, 2);
    }

    // Normal: round-trip a tool message with full input + output content.
    // Exercises the serialize_part / deserialize_part / serialize_tool_input
    // / deserialize_tool_input paths for a non-trivial tool variant.
    #[tokio::test]
    async fn save_load_with_tool_message_round_trips_normal() {
        let _g = TempConfigHome::new();
        let tool = ToolCall {
            id: "tool-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "echo hi".into(),
                timeout: Some(30_000),
                workdir: Some("/tmp".into()),
            },
            output: ToolOutput::Command {
                stdout: "hi\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            display: crate::types::ToolDisplayState::Collapsed,
            elapsed_ms: Some(123),
            started_at: None,
        };
        let messages = vec![
            ChatMessage::user("run a command".into()),
            ChatMessage::assistant_parts(vec![crate::types::MessagePart::Tool(tool)]),
        ];
        let id = SessionId::new("ses_20260506_142000");
        save_session(&id, &messages, Some("/tmp"), Some("opus")).await;
        let loaded = load_session(&id).await.expect("loaded");
        assert_eq!(loaded.len(), 2);
        let tool_part = loaded[1]
            .parts
            .iter()
            .find(|p| matches!(p, crate::types::MessagePart::Tool(_)))
            .expect("tool part");
        match tool_part {
            crate::types::MessagePart::Tool(tc) => {
                assert_eq!(tc.kind, ToolKind::Bash);
                match &tc.input {
                    ToolInput::Bash {
                        command,
                        timeout,
                        workdir,
                    } => {
                        assert_eq!(command, "echo hi");
                        assert_eq!(*timeout, Some(30_000));
                        assert_eq!(workdir.as_deref(), Some("/tmp"));
                    }
                    other => panic!("expected Bash input, got {other:?}"),
                }
                match &tc.output {
                    ToolOutput::Command {
                        stdout, exit_code, ..
                    } => {
                        assert_eq!(stdout, "hi\n");
                        assert_eq!(*exit_code, Some(0));
                    }
                    _ => panic!("expected Command output"),
                }
                // Collapsed survives, expanded/pinned do not (per design).
                assert!(tc.display.is_collapsed());
                assert!(!tc.display.is_expanded());
                assert!(!tc.display.is_pinned());
            }
            _ => unreachable!(),
        }
    }

    // Robust: deserialize_tool_status maps unknown statuses to Complete
    // (graceful fallback when an old/foreign status string lands).
    #[test]
    fn deserialize_tool_status_unknown_falls_back_robust() {
        // The function is private, but we can exercise it through
        // deserializing a SerializedPart::Tool with an unknown status.
        let part = SerializedPart::Tool {
            id: "x".into(),
            kind: "bash".into(),
            status: "exotic".into(),
            is_collapsed: false,
            input: None,
            output: None,
        };
        let mp = deserialize_part(part);
        match mp {
            crate::types::MessagePart::Tool(tc) => {
                assert_eq!(tc.status, ToolStatus::Completed);
                // Default reconstructed Bash stub — empty command.
                assert!(matches!(tc.input, ToolInput::Bash { .. }));
                assert!(matches!(tc.output, ToolOutput::Empty));
            }
            _ => panic!("expected Tool"),
        }
    }

    // Robust: deserialize_task_lifecycle maps unknown variants to Pending.
    #[test]
    fn deserialize_task_lifecycle_unknown_falls_back_robust() {
        let part = SerializedPart::TaskStatus {
            task_id: "t1".into(),
            description: "x".into(),
            status: "wat".into(),
            summary: None,
            error: None,
            elapsed_ms: None,
        };
        let mp = deserialize_part(part);
        match mp {
            crate::types::MessagePart::TaskStatus(ts) => {
                assert_eq!(ts.status, crate::types::TaskLifecycle::Pending);
            }
            _ => panic!("expected TaskStatus"),
        }
    }

    // Normal: SerializedToolOutput's custom deserializer accepts a plain
    // string (legacy v0 format) and produces a Text variant.
    #[test]
    fn serialized_tool_output_accepts_legacy_string_normal() {
        let parsed: SerializedToolOutput =
            serde_json::from_str(r#""legacy plaintext output""#).expect("ok");
        assert!(matches!(parsed, SerializedToolOutput::Text { .. }));
    }

    // Robust: a null in the output slot deserializes to Empty (not error).
    #[test]
    fn serialized_tool_output_null_to_empty_robust() {
        let parsed: SerializedToolOutput = serde_json::from_str("null").expect("ok");
        assert!(matches!(parsed, SerializedToolOutput::Empty));
    }
}
