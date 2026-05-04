use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::{
    ChatMessage, DiffHunk, DiffLine, DiffLineKind, DiffView, LargeText, MessagePart,
    ReplacementMode, Role, TaskInput, TaskLifecycle, TaskStatusPart, ToolCall, ToolInput, ToolKind,
    ToolOutput, ToolStatus,
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
    /// Working directory at the time of save. Optional so older session JSONs
    /// (which predate this field) still deserialize cleanly. Used by the Ctrl+B
    /// sidebar to group sessions under "This project" vs "Other projects".
    #[serde(default)]
    pub cwd: Option<String>,
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
        #[serde(default)]
        is_collapsed: bool,
        input: SerializedToolInput,
        output: SerializedToolOutput,
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
    Generic {
        summary: String,
    },
}

/// Full tool output serialization - preserves content for proper resume
#[derive(Serialize, Deserialize)]
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

pub fn generate_session_id() -> String {
    let now = chrono::Utc::now();
    format!("ses_{}", now.format("%Y%m%d_%H%M%S"))
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
                    // Truncate long prompts for display
                    if trimmed.len() > 100 {
                        Some(format!("{}…", &trimmed[..100]))
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                _ => None,
            })
        })
}

#[tracing::instrument(target = "jfc::session", skip(messages), fields(n = messages.len()))]
pub fn save_session(session_id: &str, messages: &[ChatMessage], cwd: Option<&str>) {
    let dir = sessions_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let now = chrono::Utc::now();
    let path = dir.join(format!("{session_id}.json"));

    // Try to load existing session to preserve created_at and cwd (the cwd at
    // session-creation is the canonical project — don't overwrite it just
    // because a user `cd`'d during the session).
    let existing: Option<SerializedSession> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<SerializedSession>(&content).ok());
    let created_at = existing
        .as_ref()
        .map(|s| s.created_at.clone())
        .unwrap_or_else(|| now.to_rfc3339());
    let stored_cwd = existing
        .and_then(|s| s.cwd)
        .or_else(|| cwd.map(str::to_owned));

    let serialized = SerializedSession {
        id: session_id.to_owned(),
        created_at,
        updated_at: Some(now.to_rfc3339()),
        first_prompt: extract_first_prompt(messages),
        cwd: stored_cwd,
        messages: messages.iter().map(serialize_message).collect(),
    };

    if let Ok(json) = serde_json::to_string_pretty(&serialized) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn load_session(session_id: &str) -> Option<Vec<ChatMessage>> {
    let path = sessions_dir().join(format!("{session_id}.json"));
    let content = std::fs::read_to_string(&path).ok()?;
    let session: SerializedSession = serde_json::from_str(&content).ok()?;
    Some(
        session
            .messages
            .into_iter()
            .map(deserialize_message)
            .collect(),
    )
}

/// Load session metadata without full message deserialization
pub fn load_session_metadata(session_id: &str) -> Option<SessionMetadata> {
    let path = sessions_dir().join(format!("{session_id}.json"));
    let content = std::fs::read_to_string(&path).ok()?;
    let session: SerializedSession = serde_json::from_str(&content).ok()?;
    Some(SessionMetadata {
        id: session.id,
        created_at: session.created_at,
        updated_at: session.updated_at,
        first_prompt: session.first_prompt,
        cwd: session.cwd,
        message_count: session.messages.len(),
    })
}

#[derive(Debug, Clone)]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    pub first_prompt: Option<String>,
    /// Working directory captured the first time this session was saved.
    /// `None` for legacy sessions that predate the `cwd` field.
    pub cwd: Option<String>,
    pub message_count: usize,
}

impl SessionMetadata {
    /// Best-effort human title for the sidebar/picker. Falls back to the
    /// formatted timestamp encoded in the session id when `first_prompt`
    /// is missing or empty (e.g. brand-new sessions, hand-edited files).
    /// Truncates very-long prompts so a single sidebar row stays readable.
    pub fn display_title(&self) -> String {
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
        format_session_id_timestamp(&self.id)
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

pub fn list_sessions() -> Vec<String> {
    let dir = sessions_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    ids.sort_by(|a, b| b.cmp(a)); // newest first
    ids
}

/// List sessions with metadata, sorted by most recent update
pub fn list_sessions_with_metadata() -> Vec<SessionMetadata> {
    let ids = list_sessions();
    let mut sessions: Vec<SessionMetadata> = ids
        .into_iter()
        .filter_map(|id| load_session_metadata(&id))
        .collect();
    // Sort by updated_at or created_at, newest first
    sessions.sort_by(|a, b| {
        let a_time = a.updated_at.as_ref().unwrap_or(&a.created_at);
        let b_time = b.updated_at.as_ref().unwrap_or(&b.created_at);
        b_time.cmp(a_time)
    });
    sessions
}

/// Get the most recent session id (for --continue)
pub fn most_recent_session() -> Option<String> {
    list_sessions().into_iter().next()
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
        parts: msg.parts.iter().map(serialize_part).collect(),
    }
}

fn serialize_part(part: &MessagePart) -> SerializedPart {
    match part {
        MessagePart::Text(t) => SerializedPart::Text { content: t.clone() },
        MessagePart::Reasoning(t) => SerializedPart::Reasoning { content: t.clone() },
        MessagePart::Tool(tc) => SerializedPart::Tool {
            id: tc.id.clone(),
            kind: tc.kind.label().to_owned(),
            status: serialize_tool_status(tc.status),
            is_collapsed: tc.is_collapsed,
            input: serialize_tool_input(&tc.input),
            output: serialize_tool_output(&tc.output),
        },
        MessagePart::TaskStatus(ts) => SerializedPart::TaskStatus {
            task_id: ts.task_id.clone(),
            description: ts.description.clone(),
            status: serialize_task_lifecycle(ts.status),
            summary: ts.summary.clone(),
            error: ts.error.clone(),
            elapsed_ms: ts.elapsed_ms,
        },
        MessagePart::CompactBoundary { pre_tokens } => SerializedPart::CompactBoundary {
            pre_tokens: *pre_tokens,
        },
    }
}

fn serialize_tool_status(status: ToolStatus) -> String {
    match status {
        ToolStatus::Pending => "pending".into(),
        ToolStatus::Running => "running".into(),
        ToolStatus::Complete => "complete".into(),
        ToolStatus::Failed => "failed".into(),
    }
}

fn serialize_task_lifecycle(status: TaskLifecycle) -> String {
    match status {
        TaskLifecycle::Pending => "pending".into(),
        TaskLifecycle::Running => "running".into(),
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
            id,
            kind: ToolKind::from_name(&kind),
            status: deserialize_tool_status(&status),
            input: deserialize_tool_input(input),
            output: deserialize_tool_output(output),
            is_collapsed,
            // Loaded sessions always come back in preview mode — the user
            // can re-expand whatever they need with Ctrl+O. Storing the
            // expanded flag in the on-disk format would persist UI
            // chrome state we don't want to roundtrip.
            expanded: false,
        }),
        SerializedPart::TaskStatus {
            task_id,
            description,
            status,
            summary,
            error,
            elapsed_ms,
        } => MessagePart::TaskStatus(TaskStatusPart {
            task_id,
            description,
            status: deserialize_task_lifecycle(&status),
            summary,
            error,
            elapsed_ms,
        }),
        SerializedPart::CompactBoundary { pre_tokens } => {
            MessagePart::CompactBoundary { pre_tokens }
        }
    }
}

fn deserialize_tool_status(status: &str) -> ToolStatus {
    match status {
        "pending" => ToolStatus::Pending,
        "running" => ToolStatus::Running,
        "complete" | "Complete" => ToolStatus::Complete,
        "failed" | "Failed" => ToolStatus::Failed,
        _ => ToolStatus::Complete,
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
        },
        SerializedToolInput::TaskList {
            status_filter,
            owner_filter,
        } => ToolInput::TaskList {
            status_filter,
            owner_filter,
        },
        SerializedToolInput::TaskDone { task_id } => ToolInput::TaskDone { task_id },
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
            id: id.to_owned(),
            created_at: "2026-05-04T19:46:49Z".to_owned(),
            updated_at: Some("2026-05-04T19:46:49Z".to_owned()),
            first_prompt: prompt.map(str::to_owned),
            cwd: cwd.map(str::to_owned),
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
