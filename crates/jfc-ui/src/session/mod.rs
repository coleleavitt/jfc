use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::ids::SessionId;
use crate::types::{
    ChatMessage, DiffHunk, DiffLine, DiffLineKind, DiffView, LargeText, MessagePart,
    ReplacementMode, Role, TaskInput, TaskLifecycle, TaskStatusPart, ToolCall, ToolInput, ToolKind,
    ToolOutput, ToolStatus, validate_turn_invariants,
};

use jfc_session::{
    SessionMetadata, cwd_mismatch_message, format_session_id_timestamp, generate_session_id,
    group_by_cwd, list_session_ids_only, list_sessions, list_sessions_filtered,
    list_sessions_with_metadata, load_session_metadata, most_recent_session,
    most_recent_session_for_cwd, relative_time, sessions_dir, shorten_cwd,
};

fn default_true() -> bool {
    true
}

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
    RedactedThinking {
        data: String,
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
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        team_name: Option<String>,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        isolation: Option<String>,
        #[serde(default)]
        parent_task_id: Option<String>,
    },
    TaskCreate {
        subject: String,
        description: String,
        #[serde(default)]
        active_form: Option<String>,
        #[serde(default)]
        blocked_by: Vec<String>,
        #[serde(default)]
        acceptance_criteria: Option<String>,
        #[serde(default)]
        verification_command: Option<String>,
        #[serde(default)]
        risk: Option<String>,
        #[serde(default)]
        parent_id: Option<String>,
        #[serde(default)]
        kind: Option<String>,
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
        #[serde(default)]
        acceptance_criteria: Option<String>,
        #[serde(default)]
        verification_command: Option<String>,
        #[serde(default)]
        risk: Option<String>,
        #[serde(default)]
        parent_id: Option<String>,
        #[serde(default)]
        kind: Option<String>,
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
    TaskValidate,
    ToolSearch {
        query: String,
        #[serde(default)]
        limit: Option<u64>,
    },
    ToolSuggest {
        intent: String,
        #[serde(default)]
        limit: Option<u64>,
    },
    TeamCreate {
        team_name: String,
        #[serde(default)]
        description: Option<String>,
    },
    TeamDelete,
    SendMessage {
        to: String,
        message: String,
        #[serde(default)]
        summary: Option<String>,
    },
    TeamMemberMode {
        member_name: String,
        mode: String,
    },
    CodeIndex {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        max_entries: Option<usize>,
    },
    GraphQuery {
        query: String,
        #[serde(default)]
        max_tokens: Option<usize>,
        #[serde(default)]
        include_handles: Option<bool>,
    },
    PostBounty {
        description: String,
        budget: u64,
        acceptance_criteria: String,
        #[serde(default)]
        max_solvers: Option<u8>,
        #[serde(default)]
        auto_dispatch: bool,
    },
    MarketStatus {
        #[serde(default)]
        bounty_id: Option<String>,
    },
    RunBounty {
        bounty_id: String,
        #[serde(default)]
        max_solvers: Option<u8>,
    },
    RunCoverage {
        #[serde(default)]
        lcov_path: Option<String>,
        #[serde(default = "default_true")]
        include_untested_list: bool,
    },
    SymbolEdit {
        handle: String,
        new_content: String,
        #[serde(default)]
        validate: bool,
        #[serde(default, rename = "dispatch_cascade")]
        dispatch_cascade: bool,
    },
    ExitPlanMode {
        plan: String,
    },
    MultiEdit {
        file_path: String,
        edits: serde_json::Value,
    },
    AskUserQuestion {
        question: String,
        options: serde_json::Value,
        #[serde(default)]
        multi_select: bool,
    },
    WebFetch {
        url: String,
        #[serde(default)]
        prompt: Option<String>,
    },
    WebSearch {
        query: String,
        #[serde(default)]
        max_results: Option<u32>,
    },
    Mcp {
        name: String,
        arguments: serde_json::Value,
    },
    CronCreate {
        schedule: String,
        command: String,
        description: String,
    },
    CronList,
    CronDelete {
        id: String,
    },
    ScheduleWakeup {
        delay_seconds: u32,
        prompt: String,
        reason: String,
    },
    Monitor {
        command: String,
        until: String,
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
    ScratchpadRead {
        key: String,
    },
    ScratchpadWrite {
        key: String,
        value: String,
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
    /// Anthropic server-side tool result (e.g. `web_search_tool_result`).
    /// Persisted as the raw JSON content + wire-type string so that
    /// reloads round-trip the result back into the runtime as a
    /// `ToolOutput::ServerToolResult`. Sessions written before this
    /// variant landed deserialize fine — the variant only appears on
    /// fresh writes that involved a server-side tool.
    ServerToolResult {
        /// Anthropic wire `type` field (e.g. "web_search_tool_result",
        /// "code_execution_tool_result"). Used to reconstruct the
        /// `ServerToolResultKind` discriminant on load.
        wire_type: String,
        /// Raw JSON content returned by the server. Preserved verbatim
        /// so the resend path can re-emit it byte-faithfully on a
        /// future user turn (cli.js v142:441375).
        content: serde_json::Value,
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
                    ServerToolResult {
                        wire_type: String,
                        content: serde_json::Value,
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
                    Inner::ServerToolResult { wire_type, content } => {
                        SerializedToolOutput::ServerToolResult { wire_type, content }
                    }
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

/// Merge consecutive same-role `ChatMessage`s into one logical turn
/// for persistence. Agentic loops push a fresh empty assistant slot
/// per sub-stream (see `setup_new_substream_slot`), so a 5-step
/// agentic turn ends up as `[user, A1, A2, A3, A4, A5, user]` on
/// disk. That:
///
///   * makes the file unreadable (one prompt → 5+ "assistant:" headers);
///   * makes resume rebuild the per-sub-stream split, with every
///     subsequent provider request `validate_turn_invariants`-warning
///     ConsecutiveAssistant in the log;
///   * confuses LLM-based summarizers that key off speaker alternation.
///
/// This helper does NOT touch the in-memory `app.messages` (sub-stream
/// boundaries are still needed at runtime for streaming-slot tracking
/// and the "this sub-stream completed at T" timestamps); it only runs
/// on the path **into** the session JSON.
///
/// Merging rules:
///   * adjacent same-role messages → one message with all parts
///     concatenated in order;
///   * `is_compact_boundary` messages stay on their own — they're a
///     semantic separator the renderer keys off;
///   * scalar fields (`agent_name`, `model_name`, `cost_tier`,
///     `elapsed`, `usage`) prefer the LAST non-None value — the most
///     recent sub-stream's metadata is the cumulative-correct one
///     (matches v126's per-message usage semantics: every assistant
///     message carries the END-of-turn cumulative count).
///   * `attachments` concatenate.
///   * `queued` messages bypass merging entirely (they're filtered
///     out before serialize anyway, but the dedup walk respects them
///     in case the filter ever moves).
fn coalesce_consecutive_same_role(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for msg in messages {
        let can_merge = out.last().is_some_and(|prev| {
            prev.role == msg.role
                && !prev.is_compact_boundary()
                && !msg.is_compact_boundary()
                && !prev.queued
                && !msg.queued
        });
        if can_merge {
            let prev = out.last_mut().expect("can_merge guarantees a tail");
            // Extend parts in order — preserves the per-sub-stream
            // interleaving (text from sub-stream 1, tool from sub-stream
            // 1, text from sub-stream 2, tool from sub-stream 2, ...)
            // so the renderer can still walk through the conversation
            // chronologically.
            prev.parts.extend(msg.parts.iter().cloned());
            prev.attachments.extend(msg.attachments.iter().cloned());
            // Scalar fields: prefer the LAST non-None — the latest
            // sub-stream's view is the cumulative-correct one.
            if msg.agent_name.is_some() {
                prev.agent_name = msg.agent_name.clone();
            }
            if msg.model_name.is_some() {
                prev.model_name = msg.model_name.clone();
            }
            if msg.cost_tier.is_some() {
                prev.cost_tier = msg.cost_tier.clone();
            }
            if msg.elapsed.is_some() {
                prev.elapsed = msg.elapsed.clone();
            }
            if msg.usage.is_some() {
                prev.usage = msg.usage.clone();
            }
        } else {
            out.push(msg.clone());
        }
    }
    out
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
    //
    // Then coalesce consecutive same-role messages (sub-stream split
    // artifacts) into one logical turn per persisted message. See
    // `coalesce_consecutive_same_role` for the rationale — the file
    // on disk becomes the "alternating user/assistant" shape that
    // `validate_turn_invariants` enforces, and the renderer stops
    // emitting one "assistant:" header per agentic sub-stream.
    let filtered: Vec<ChatMessage> = messages.iter().filter(|m| !m.queued).cloned().collect();
    let coalesced = coalesce_consecutive_same_role(&filtered);
    tracing::debug!(
        target: "jfc::session",
        session_id = session_id_str,
        runtime_messages = messages.len(),
        post_filter_messages = filtered.len(),
        coalesced_messages = coalesced.len(),
        "session save: coalescing sub-stream message splits"
    );
    let serialized = SerializedSession {
        id: session_id_str.to_owned(),
        created_at,
        updated_at: Some(now.to_rfc3339()),
        first_prompt: extract_first_prompt(messages),
        model: stored_model,
        cwd: stored_cwd,
        title,
        messages: coalesced.iter().map(serialize_message).collect(),
    };

    if let Ok(json) = serde_json::to_string_pretty(&serialized) {
        // Atomic write: a SIGKILL or power loss between writeFile()
        // chunks would otherwise leave the session JSON truncated
        // (e.g. half a `messages` array with no closing brace), and
        // every subsequent load would fail to deserialize and the
        // user would lose the whole transcript. temp + fsync + rename
        // keeps the old contents in place until the new payload is
        // fully on disk. See crate::atomic_write for the recipe.
        if let Err(e) = crate::atomic_write::write_atomic(&path, json.as_bytes()).await {
            warn!(
                target: "jfc::session",
                session_id = session_id_str,
                error = %e,
                "atomic session write failed — previous on-disk contents preserved"
            );
        } else {
            info!(target: "jfc::session", session_id = session_id_str, message_count = messages.len(), path = %path.display(), "session saved");
        }
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
        // Atomic write — see save_session() above for the rationale.
        // Title updates are cosmetic but they overwrite the entire
        // session file, so a torn write here loses the whole transcript.
        if let Err(e) = crate::atomic_write::write_atomic(&path, json.as_bytes()).await {
            warn!(
                target: "jfc::session",
                session_id = session_id_str,
                error = %e,
                "atomic title write failed — previous session preserved"
            );
        } else {
            info!(target: "jfc::session", session_id = session_id_str, "session title updated");
        }
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
        MessagePart::RedactedThinking(data) => SerializedPart::RedactedThinking {
            data: data.clone(),
        },
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
            name: ti.name.clone(),
            team_name: ti.team_name.clone(),
            mode: ti.mode.clone(),
            isolation: ti.isolation.clone(),
            parent_task_id: ti.parent_task_id.clone(),
        },
        ToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        } => SerializedToolInput::TaskCreate {
            subject: subject.clone(),
            description: description.clone(),
            active_form: active_form.clone(),
            blocked_by: blocked_by.clone(),
            acceptance_criteria: acceptance_criteria.clone(),
            verification_command: verification_command.clone(),
            risk: risk.clone(),
            parent_id: parent_id.clone(),
            kind: kind.clone(),
        },
        ToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        } => SerializedToolInput::TaskUpdate {
            task_id: task_id.clone(),
            status: status.clone(),
            subject: subject.clone(),
            description: description.clone(),
            owner: owner.clone(),
            acceptance_criteria: acceptance_criteria.clone(),
            verification_command: verification_command.clone(),
            risk: risk.clone(),
            parent_id: parent_id.clone(),
            kind: kind.clone(),
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
        ToolInput::TaskValidate => SerializedToolInput::TaskValidate,
        ToolInput::Skill { name, args } => SerializedToolInput::Skill {
            name: name.clone(),
            args: args.clone(),
        },
        ToolInput::ToolSearch { query, limit } => SerializedToolInput::ToolSearch {
            query: query.clone(),
            limit: *limit,
        },
        ToolInput::ToolSuggest { intent, limit } => SerializedToolInput::ToolSuggest {
            intent: intent.clone(),
            limit: *limit,
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
        } => SerializedToolInput::TeamCreate {
            team_name: team_name.clone(),
            description: description.clone(),
        },
        ToolInput::TeamDelete => SerializedToolInput::TeamDelete,
        ToolInput::SendMessage {
            to,
            message,
            summary,
        } => SerializedToolInput::SendMessage {
            to: to.clone(),
            message: message.clone(),
            summary: summary.clone(),
        },
        ToolInput::TeamMemberMode { member_name, mode } => SerializedToolInput::TeamMemberMode {
            member_name: member_name.clone(),
            mode: mode.clone(),
        },
        ToolInput::CodeIndex {
            path,
            query,
            kind,
            max_entries,
        } => SerializedToolInput::CodeIndex {
            path: path.clone(),
            query: query.clone(),
            kind: kind.clone(),
            max_entries: *max_entries,
        },
        ToolInput::GraphQuery {
            query,
            max_tokens,
            include_handles,
        } => SerializedToolInput::GraphQuery {
            query: query.clone(),
            max_tokens: *max_tokens,
            include_handles: *include_handles,
        },
        ToolInput::RunCoverage {
            lcov_path,
            include_untested_list,
        } => SerializedToolInput::RunCoverage {
            lcov_path: lcov_path.clone(),
            include_untested_list: *include_untested_list,
        },
        ToolInput::SymbolEdit {
            handle,
            new_content,
            validate,
            dispatch_cascade,
        } => SerializedToolInput::SymbolEdit {
            handle: handle.clone(),
            new_content: new_content.clone(),
            validate: *validate,
            dispatch_cascade: *dispatch_cascade,
        },
        ToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
        } => SerializedToolInput::PostBounty {
            description: description.clone(),
            budget: *budget,
            acceptance_criteria: acceptance_criteria.clone(),
            max_solvers: *max_solvers,
            auto_dispatch: *auto_dispatch,
        },
        ToolInput::MarketStatus { bounty_id } => SerializedToolInput::MarketStatus {
            bounty_id: bounty_id.clone(),
        },
        ToolInput::RunBounty {
            bounty_id,
            max_solvers,
        } => SerializedToolInput::RunBounty {
            bounty_id: bounty_id.clone(),
            max_solvers: *max_solvers,
        },
        ToolInput::ExitPlanMode { plan } => {
            SerializedToolInput::ExitPlanMode { plan: plan.clone() }
        }
        ToolInput::MultiEdit { file_path, edits } => SerializedToolInput::MultiEdit {
            file_path: file_path.clone(),
            edits: edits.clone(),
        },
        ToolInput::AskUserQuestion {
            question,
            options,
            multi_select,
        } => SerializedToolInput::AskUserQuestion {
            question: question.clone(),
            options: options.clone(),
            multi_select: *multi_select,
        },
        ToolInput::WebFetch { url, prompt } => SerializedToolInput::WebFetch {
            url: url.clone(),
            prompt: prompt.clone(),
        },
        ToolInput::WebSearch { query, max_results } => SerializedToolInput::WebSearch {
            query: query.clone(),
            max_results: *max_results,
        },
        ToolInput::Mcp { name, arguments } => SerializedToolInput::Mcp {
            name: name.clone(),
            arguments: arguments.clone(),
        },
        ToolInput::CronCreate {
            schedule,
            command,
            description,
        } => SerializedToolInput::CronCreate {
            schedule: schedule.clone(),
            command: command.clone(),
            description: description.clone(),
        },
        ToolInput::CronList => SerializedToolInput::CronList,
        ToolInput::CronDelete { id } => SerializedToolInput::CronDelete { id: id.clone() },
        ToolInput::ScheduleWakeup {
            delay_seconds,
            prompt,
            reason,
        } => SerializedToolInput::ScheduleWakeup {
            delay_seconds: *delay_seconds,
            prompt: prompt.clone(),
            reason: reason.clone(),
        },
        ToolInput::Monitor { command, until } => SerializedToolInput::Monitor {
            command: command.clone(),
            until: until.clone(),
        },
        ToolInput::Lsp {
            kind,
            file,
            line,
            column,
        } => SerializedToolInput::Lsp {
            kind: kind.clone(),
            file: file.clone(),
            line: *line,
            column: *column,
        },
        ToolInput::PushNotification { message, title } => SerializedToolInput::PushNotification {
            message: message.clone(),
            title: title.clone(),
        },
        ToolInput::RemoteTrigger {
            trigger_id,
            payload,
        } => SerializedToolInput::RemoteTrigger {
            trigger_id: trigger_id.clone(),
            payload: payload.clone(),
        },
        ToolInput::EnterPlanMode { reason } => SerializedToolInput::EnterPlanMode {
            reason: reason.clone(),
        },
        ToolInput::EnterWorktree { name, branch } => SerializedToolInput::EnterWorktree {
            name: name.clone(),
            branch: branch.clone(),
        },
        ToolInput::ExitWorktree => SerializedToolInput::ExitWorktree,
        ToolInput::NotebookRead { path } => {
            SerializedToolInput::NotebookRead { path: path.clone() }
        }
        ToolInput::NotebookEdit {
            path,
            cell_id,
            new_source,
            edit_mode,
        } => SerializedToolInput::NotebookEdit {
            path: path.clone(),
            cell_id: cell_id.clone(),
            new_source: new_source.clone(),
            edit_mode: edit_mode.clone(),
        },
        ToolInput::ScratchpadRead { key } => {
            SerializedToolInput::ScratchpadRead { key: key.clone() }
        }
        ToolInput::ScratchpadWrite { key, value } => SerializedToolInput::ScratchpadWrite {
            key: key.clone(),
            value: value.clone(),
        },
        ToolInput::Generic { summary } => SerializedToolInput::Generic {
            summary: summary.clone(),
        },
    }
}

fn serialize_generic_tool_input_json(input: &ToolInput) -> SerializedToolInput {
    SerializedToolInput::Generic {
        summary: input.to_value().to_string(),
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
        ToolOutput::ServerToolResult { tool_kind, content } => {
            SerializedToolOutput::ServerToolResult {
                wire_type: tool_kind.wire_type().to_owned(),
                content: content.clone(),
            }
        }
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
        } => {
            let tool_kind = ToolKind::from_name(&kind);
            MessagePart::Tool(ToolCall {
                id: crate::ids::ToolId::from(id),
                kind: tool_kind,
                status: deserialize_tool_status(&status),
                // Tolerate missing input/output on legacy session files.
                // The unknown-input fallback (a no-op Bash entry) lets the
                // resumed transcript render the tool row with whatever
                // chrome we have (id, kind, status) without panicking on a
                // missing field that older writers never produced.
                input: match input {
                    Some(i) => deserialize_tool_input_for_kind(&kind, i),
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
            })
        }
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
        SerializedPart::RedactedThinking { data } => MessagePart::RedactedThinking(data),
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

fn deserialize_tool_input_for_kind(kind: &str, input: SerializedToolInput) -> ToolInput {
    match input {
        SerializedToolInput::Generic { summary } => {
            deserialize_generic_tool_input(kind, &summary).unwrap_or(ToolInput::Generic { summary })
        }
        other => deserialize_tool_input(other),
    }
}

fn deserialize_generic_tool_input(kind: &str, summary: &str) -> Option<ToolInput> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(summary)
        && value.is_object()
        && let Ok(input) = ToolInput::from_value(kind, value)
    {
        return Some(input);
    }

    match ToolKind::from_name(kind) {
        ToolKind::GraphQuery => parse_legacy_graph_query(summary),
        ToolKind::WebSearch => {
            summary
                .strip_prefix("WebSearch: ")
                .map(|query| ToolInput::WebSearch {
                    query: query.to_owned(),
                    max_results: None,
                })
        }
        ToolKind::WebFetch => summary
            .strip_prefix("WebFetch: ")
            .map(|url| ToolInput::WebFetch {
                url: url.to_owned(),
                prompt: None,
            }),
        ToolKind::EnterPlanMode => {
            summary
                .strip_prefix("EnterPlanMode: ")
                .map(|reason| ToolInput::EnterPlanMode {
                    reason: reason.to_owned(),
                })
        }
        ToolKind::ExitPlanMode => {
            summary
                .strip_prefix("ExitPlanMode: ")
                .map(|plan| ToolInput::ExitPlanMode {
                    plan: plan.to_owned(),
                })
        }
        ToolKind::MultiEdit => parse_legacy_multi_edit(summary),
        ToolKind::RunCoverage => parse_legacy_run_coverage(summary),
        ToolKind::MarketStatus => parse_legacy_market_status(summary),
        ToolKind::RunBounty => {
            summary
                .strip_prefix("RunBounty: ")
                .map(|bounty_id| ToolInput::RunBounty {
                    bounty_id: bounty_id.to_owned(),
                    max_solvers: None,
                })
        }
        ToolKind::TeamCreate => parse_legacy_team_create(summary),
        ToolKind::TeamDelete if summary == "TeamDelete" => Some(ToolInput::TeamDelete),
        ToolKind::TeamMemberMode => parse_legacy_team_member_mode(summary),
        ToolKind::CodeIndex => parse_legacy_code_index(summary),
        ToolKind::PushNotification => parse_legacy_push_notification(summary),
        ToolKind::RemoteTrigger => {
            summary
                .strip_prefix("RemoteTrigger: ")
                .map(|trigger_id| ToolInput::RemoteTrigger {
                    trigger_id: trigger_id.to_owned(),
                    payload: None,
                })
        }
        ToolKind::AskUserQuestion => parse_legacy_ask_user_question(summary),
        ToolKind::EnterWorktree => parse_legacy_enter_worktree(summary),
        ToolKind::NotebookRead => {
            summary
                .strip_prefix("NotebookRead: ")
                .map(|path| ToolInput::NotebookRead {
                    path: path.to_owned(),
                })
        }
        ToolKind::ScratchpadRead => {
            summary
                .strip_prefix("ScratchpadRead: ")
                .map(|key| ToolInput::ScratchpadRead {
                    key: key.to_owned(),
                })
        }
        _ => None,
    }
}

fn strip_any_prefix<'a>(summary: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes
        .iter()
        .find_map(|prefix| summary.strip_prefix(prefix))
}

fn parse_legacy_graph_query(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("GraphQuery(budget=")?;
    let (budget, query) = rest.split_once("): ")?;
    Some(ToolInput::GraphQuery {
        query: query.to_owned(),
        max_tokens: budget.parse().ok(),
        include_handles: None,
    })
}

fn parse_legacy_multi_edit(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("MultiEdit: ")?;
    let file_path = rest.split_once(" (").map_or(rest, |(path, _)| path);
    Some(ToolInput::MultiEdit {
        file_path: file_path.to_owned(),
        edits: serde_json::json!([]),
    })
}

fn parse_legacy_run_coverage(summary: &str) -> Option<ToolInput> {
    let inner = summary
        .strip_prefix("RunCoverage(")?
        .strip_suffix(')')?
        .trim();
    Some(ToolInput::RunCoverage {
        lcov_path: (inner != "auto").then(|| inner.to_owned()),
        include_untested_list: true,
    })
}

fn parse_legacy_market_status(summary: &str) -> Option<ToolInput> {
    if summary == "MarketStatus" {
        return Some(ToolInput::MarketStatus { bounty_id: None });
    }
    summary
        .strip_prefix("MarketStatus: ")
        .map(|id| ToolInput::MarketStatus {
            bounty_id: Some(id.to_owned()),
        })
}

fn parse_legacy_team_create(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("TeamCreate: ")?;
    let (team_name, description) = rest
        .split_once(" — ")
        .map_or((rest, None), |(name, desc)| (name, Some(desc.to_owned())));
    Some(ToolInput::TeamCreate {
        team_name: team_name.to_owned(),
        description,
    })
}

fn parse_legacy_team_member_mode(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("TeamMemberMode ")?;
    let (member_name, mode) = rest.split_once(": ")?;
    Some(ToolInput::TeamMemberMode {
        member_name: member_name.to_owned(),
        mode: mode.to_owned(),
    })
}

fn parse_legacy_code_index(summary: &str) -> Option<ToolInput> {
    if summary == "CodeIndex" {
        return Some(ToolInput::CodeIndex {
            path: None,
            query: None,
            kind: None,
            max_entries: None,
        });
    }
    let inner = summary.strip_prefix("CodeIndex(")?.strip_suffix(')')?;
    let mut path = None;
    let mut query = None;
    let mut kind = None;
    for part in inner.split(',').map(str::trim) {
        if kind.is_none() {
            kind = Some(part.to_owned());
        } else if query.is_none() {
            query = Some(part.to_owned());
        } else if path.is_none() {
            path = Some(part.to_owned());
        }
    }
    Some(ToolInput::CodeIndex {
        path,
        query,
        kind,
        max_entries: None,
    })
}

fn parse_legacy_push_notification(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("PushNotification: ")?;
    let (title, message) = rest
        .split_once(": ")
        .map_or((None, rest), |(title, message)| {
            (Some(title.to_owned()), message)
        });
    Some(ToolInput::PushNotification {
        message: message.to_owned(),
        title,
    })
}

fn parse_legacy_enter_worktree(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("EnterWorktree: ")?;
    let (name, branch) = if let Some((name, branch)) = rest
        .strip_suffix(')')
        .and_then(|trimmed| trimmed.split_once(" ("))
    {
        (name, Some(branch.to_owned()))
    } else {
        (rest, None)
    };
    Some(ToolInput::EnterWorktree {
        name: name.to_owned(),
        branch,
    })
}

fn parse_legacy_ask_user_question(summary: &str) -> Option<ToolInput> {
    let question = strip_any_prefix(summary, &["AskUserQuestion: ", "ask: "])?;
    Some(ToolInput::AskUserQuestion {
        question: question.to_owned(),
        options: serde_json::json!([]),
        multi_select: false,
    })
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
            name,
            team_name,
            mode,
            isolation,
            parent_task_id,
        } => ToolInput::Task(TaskInput {
            description,
            prompt,
            subagent_type,
            category,
            run_in_background,
            model,
            name,
            team_name,
            mode,
            isolation,
            parent_task_id,
        }),
        SerializedToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        } => ToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        },
        SerializedToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        } => ToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
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
        SerializedToolInput::TaskValidate => ToolInput::TaskValidate,
        SerializedToolInput::Skill { name, args } => ToolInput::Skill { name, args },
        SerializedToolInput::ToolSearch { query, limit } => ToolInput::ToolSearch { query, limit },
        SerializedToolInput::ToolSuggest { intent, limit } => {
            ToolInput::ToolSuggest { intent, limit }
        }
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
        SerializedToolInput::TeamCreate {
            team_name,
            description,
        } => ToolInput::TeamCreate {
            team_name,
            description,
        },
        SerializedToolInput::TeamDelete => ToolInput::TeamDelete,
        SerializedToolInput::SendMessage {
            to,
            message,
            summary,
        } => ToolInput::SendMessage {
            to,
            message,
            summary,
        },
        SerializedToolInput::TeamMemberMode { member_name, mode } => {
            ToolInput::TeamMemberMode { member_name, mode }
        }
        SerializedToolInput::CodeIndex {
            path,
            query,
            kind,
            max_entries,
        } => ToolInput::CodeIndex {
            path,
            query,
            kind,
            max_entries,
        },
        SerializedToolInput::GraphQuery {
            query,
            max_tokens,
            include_handles,
        } => ToolInput::GraphQuery {
            query,
            max_tokens,
            include_handles,
        },
        SerializedToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
        } => ToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
        },
        SerializedToolInput::MarketStatus { bounty_id } => ToolInput::MarketStatus { bounty_id },
        SerializedToolInput::RunBounty {
            bounty_id,
            max_solvers,
        } => ToolInput::RunBounty {
            bounty_id,
            max_solvers,
        },
        SerializedToolInput::RunCoverage {
            lcov_path,
            include_untested_list,
        } => ToolInput::RunCoverage {
            lcov_path,
            include_untested_list,
        },
        SerializedToolInput::SymbolEdit {
            handle,
            new_content,
            validate,
            dispatch_cascade,
        } => ToolInput::SymbolEdit {
            handle,
            new_content,
            validate,
            dispatch_cascade,
        },
        SerializedToolInput::ExitPlanMode { plan } => ToolInput::ExitPlanMode { plan },
        SerializedToolInput::MultiEdit { file_path, edits } => {
            ToolInput::MultiEdit { file_path, edits }
        }
        SerializedToolInput::AskUserQuestion {
            question,
            options,
            multi_select,
        } => ToolInput::AskUserQuestion {
            question,
            options,
            multi_select,
        },
        SerializedToolInput::WebFetch { url, prompt } => ToolInput::WebFetch { url, prompt },
        SerializedToolInput::WebSearch { query, max_results } => {
            ToolInput::WebSearch { query, max_results }
        }
        SerializedToolInput::Mcp { name, arguments } => ToolInput::Mcp { name, arguments },
        SerializedToolInput::CronCreate {
            schedule,
            command,
            description,
        } => ToolInput::CronCreate {
            schedule,
            command,
            description,
        },
        SerializedToolInput::CronList => ToolInput::CronList,
        SerializedToolInput::CronDelete { id } => ToolInput::CronDelete { id },
        SerializedToolInput::ScheduleWakeup {
            delay_seconds,
            prompt,
            reason,
        } => ToolInput::ScheduleWakeup {
            delay_seconds,
            prompt,
            reason,
        },
        SerializedToolInput::Monitor { command, until } => ToolInput::Monitor { command, until },
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
        SerializedToolInput::ScratchpadRead { key } => ToolInput::ScratchpadRead { key },
        SerializedToolInput::ScratchpadWrite { key, value } => {
            ToolInput::ScratchpadWrite { key, value }
        }
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
        SerializedToolOutput::ServerToolResult { wire_type, content } => {
            ToolOutput::ServerToolResult {
                tool_kind: jfc_provider::ServerToolResultKind::from_wire_type(&wire_type),
                content,
            }
        }
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
mod coalesce_tests {
    //! Pins the on-disk shape of agentic-loop transcripts. Sub-stream
    //! splits (one `ChatMessage::assistant("")` per sub-stream from
    //! `setup_new_substream_slot`) must collapse into a single
    //! assistant message on save so the file is human-readable and the
    //! resume path doesn't get 50+ assistant rows for a single user
    //! turn (the original `ses_20260515_175208.json` symptom).
    use super::*;
    use crate::ids::ToolId;
    use crate::types::{
        ChatMessage, MessagePart, ModelUsage, Role, ToolCall, ToolDisplayState, ToolInput,
        ToolKind, ToolOutput, ToolStatus,
    };

    fn user_text(s: &str) -> ChatMessage {
        ChatMessage::user(s.to_owned())
    }

    fn assistant_text(s: &str) -> ChatMessage {
        ChatMessage::assistant(s.to_owned())
    }

    fn tool_part(id: &str) -> MessagePart {
        MessagePart::Tool(ToolCall {
            id: ToolId::from(id),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Text("ok".into()),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        })
    }

    // Normal: a 5-step agentic loop (user → A,A,A,A,A → user) collapses
    // to user → A → user. The persisted JSON shape matches the
    // alternating-role invariant `validate_turn_invariants` enforces.
    #[test]
    fn coalesces_five_assistant_substreams_to_one_normal() {
        let input = vec![
            user_text("do the thing"),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 1".into()), tool_part("t1")]),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 2".into()), tool_part("t2")]),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 3".into()), tool_part("t3")]),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 4".into()), tool_part("t4")]),
            assistant_text("done"),
            user_text("next prompt"),
        ];
        let out = coalesce_consecutive_same_role(&input);
        assert_eq!(out.len(), 3, "must collapse the 5 sub-streams into one");
        assert_eq!(out[0].role, Role::User);
        assert_eq!(out[1].role, Role::Assistant);
        assert_eq!(out[2].role, Role::User);
        // Parts preserved in order across all 5 sub-streams:
        // 4 (text+tool) + 1 (text) = 9.
        assert_eq!(out[1].parts.len(), 9);
        // Validate that the alternating-role invariant holds on the
        // coalesced output (this is what the on-disk file should
        // satisfy).
        validate_turn_invariants(&out).expect("coalesced session must satisfy invariants");
    }

    // Normal: an empty input produces an empty output (no synthetic
    // injection, no panic on the no-tail branch).
    #[test]
    fn coalesce_empty_input_normal() {
        let out = coalesce_consecutive_same_role(&[]);
        assert!(out.is_empty());
    }

    // Robust: an already-alternating transcript is a fixed point.
    // Coalescing twice produces the same shape.
    #[test]
    fn coalesce_already_alternating_is_fixed_point_robust() {
        let input = vec![
            user_text("a"),
            assistant_text("b"),
            user_text("c"),
            assistant_text("d"),
        ];
        let first_pass = coalesce_consecutive_same_role(&input);
        let second_pass = coalesce_consecutive_same_role(&first_pass);
        assert_eq!(first_pass.len(), 4);
        assert_eq!(first_pass.len(), second_pass.len());
        for (a, b) in first_pass.iter().zip(second_pass.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.parts.len(), b.parts.len());
        }
    }

    // Robust: queued-prompt placeholders never participate in
    // merging. They're filtered out of save_session before coalesce
    // runs, but if a future caller hands them in directly the
    // dedup walk must respect them so user-typed text isn't
    // accidentally promoted into a sent prompt.
    #[test]
    fn coalesce_skips_queued_messages_robust() {
        let mut queued = user_text("queued");
        queued.queued = true;
        let input = vec![user_text("first"), queued, user_text("second")];
        let out = coalesce_consecutive_same_role(&input);
        // Queued is preserved as its own entry — never merged into a
        // sibling user message.
        assert_eq!(out.len(), 3);
        assert!(out[1].queued);
    }

    // Robust: usage from the LAST sub-stream wins on merge. v126
    // semantics: each assistant message carries the END-of-turn
    // cumulative usage, so the final sub-stream's usage IS the
    // post-merge correct value. If we picked the first or summed
    // them, the Context gauge would over- or under-count.
    #[test]
    fn coalesce_picks_last_usage_robust() {
        let mut first = ChatMessage::assistant("step 1".into());
        first.usage = Some(ModelUsage {
            input_tokens: 100,
            output_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        let mut last = ChatMessage::assistant("step 2".into());
        last.usage = Some(ModelUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        let input = vec![user_text("hi"), first, last];
        let out = coalesce_consecutive_same_role(&input);
        assert_eq!(out.len(), 2);
        let usage = out[1].usage.as_ref().expect("usage must survive merge");
        assert_eq!(
            usage.output_tokens, 200,
            "merged usage must be the LAST sub-stream's value (cumulative end-of-turn count)"
        );
    }

    // Robust: a compact_boundary message stays on its own — it's a
    // semantic separator the renderer keys off, and merging it into a
    // sibling assistant would teach the model that the summary IS the
    // assistant's reply.
    #[test]
    fn coalesce_preserves_compact_boundary_robust() {
        let boundary =
            ChatMessage::assistant_parts(vec![MessagePart::CompactBoundary { pre_tokens: 100 }]);
        let input = vec![
            user_text("first"),
            assistant_text("step 1"),
            boundary,
            assistant_text("step 2"),
        ];
        let out = coalesce_consecutive_same_role(&input);
        // The boundary stays on its own message; "step 1" and
        // "step 2" do NOT merge across it because the boundary
        // breaks the same-role-merge chain.
        assert_eq!(out.len(), 4);
        assert!(
            out[2].is_compact_boundary(),
            "boundary must survive on its own message"
        );
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
    use crate::types::{
        ChatMessage, TaskInput, ToolCall, ToolInput, ToolKind, ToolOutput, ToolStatus,
    };
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

    // Robust: legacy Generic summaries like `GraphQuery(...): ...` are
    // repaired on load so resumed history replays structured tool_use
    // inputs instead of `{ "input": "GraphQuery(...): ..." }`.
    #[test]
    fn generic_tool_input_legacy_graph_query_rehydrates_robust() {
        let input = deserialize_tool_input_for_kind(
            "GraphQuery",
            SerializedToolInput::Generic {
                summary: r#"GraphQuery(budget=3000): fn("main") | callees"#.into(),
            },
        );
        match input {
            ToolInput::GraphQuery {
                query, max_tokens, ..
            } => {
                assert_eq!(query, r#"fn("main") | callees"#);
                assert_eq!(max_tokens, Some(3000));
            }
            other => panic!("expected GraphQuery, got {}", other.summary()),
        }
    }

    #[test]
    fn generic_tool_input_json_rehydrates_by_kind_normal() {
        let input = deserialize_tool_input_for_kind(
            "WebSearch",
            SerializedToolInput::Generic {
                summary: serde_json::json!({
                    "query": "streaming parser",
                    "max_results": 3,
                })
                .to_string(),
            },
        );
        match input {
            ToolInput::WebSearch { query, max_results } => {
                assert_eq!(query, "streaming parser");
                assert_eq!(max_results, Some(3));
            }
            other => panic!("expected WebSearch, got {}", other.summary()),
        }
    }

    #[test]
    fn generic_tool_input_legacy_multi_edit_uses_valid_shape_robust() {
        let input = deserialize_tool_input_for_kind(
            "MultiEdit",
            SerializedToolInput::Generic {
                summary: "MultiEdit: /tmp/file.rs (2 edits)".into(),
            },
        );
        match input {
            ToolInput::MultiEdit { file_path, edits } => {
                assert_eq!(file_path, "/tmp/file.rs");
                assert_eq!(edits, serde_json::json!([]));
            }
            other => panic!("expected MultiEdit, got {}", other.summary()),
        }
    }

    #[test]
    fn serialized_task_input_preserves_teammate_fields_normal() {
        let input = ToolInput::Task(TaskInput {
            description: "review auth".into(),
            prompt: "review auth carefully".into(),
            subagent_type: Some("reviewer".into()),
            category: Some("code".into()),
            run_in_background: true,
            model: Some("anthropic/claude-sonnet-4-7".into()),
            name: Some("alice".into()),
            team_name: Some("core".into()),
            mode: Some("plan".into()),
            isolation: Some("worktree".into()),
            parent_task_id: Some("t20".into()),
        });

        let encoded = serialize_tool_input(&input);
        assert!(matches!(
            encoded,
            SerializedToolInput::Task {
                ref name,
                ref team_name,
                ref mode,
                ref isolation,
                ..
            } if name.as_deref() == Some("alice")
                && team_name.as_deref() == Some("core")
                && mode.as_deref() == Some("plan")
                && isolation.as_deref() == Some("worktree")
        ));

        let decoded = deserialize_tool_input(encoded);
        match decoded {
            ToolInput::Task(task) => {
                assert_eq!(task.name.as_deref(), Some("alice"));
                assert_eq!(task.team_name.as_deref(), Some("core"));
                assert_eq!(task.mode.as_deref(), Some("plan"));
                assert_eq!(task.isolation.as_deref(), Some("worktree"));
                assert_eq!(task.parent_task_id.as_deref(), Some("t20"));
            }
            other => panic!("expected Task, got {}", other.summary()),
        }
    }

    #[test]
    fn serialized_task_metadata_preserves_extended_fields_normal() {
        let create = ToolInput::TaskCreate {
            subject: "map parser".into(),
            description: "write the parser".into(),
            active_form: Some("parsing".into()),
            blocked_by: vec!["t1".into()],
            acceptance_criteria: Some("round-trip fixtures".into()),
            verification_command: Some("cargo test parser".into()),
            risk: Some("medium".into()),
            parent_id: Some("t0".into()),
            kind: Some("implementation".into()),
        };
        let decoded = deserialize_tool_input(serialize_tool_input(&create));
        match decoded {
            ToolInput::TaskCreate {
                acceptance_criteria,
                verification_command,
                risk,
                parent_id,
                kind,
                ..
            } => {
                assert_eq!(acceptance_criteria.as_deref(), Some("round-trip fixtures"));
                assert_eq!(verification_command.as_deref(), Some("cargo test parser"));
                assert_eq!(risk.as_deref(), Some("medium"));
                assert_eq!(parent_id.as_deref(), Some("t0"));
                assert_eq!(kind.as_deref(), Some("implementation"));
            }
            other => panic!("expected TaskCreate, got {}", other.summary()),
        }
    }

    #[test]
    fn previously_generic_tool_inputs_serialize_as_typed_variants_normal() {
        let samples = vec![
            ToolInput::WebSearch {
                query: "stream parser".into(),
                max_results: Some(3),
            },
            ToolInput::WebFetch {
                url: "https://example.invalid".into(),
                prompt: Some("summarize".into()),
            },
            ToolInput::AskUserQuestion {
                question: "choose one".into(),
                options: serde_json::json!(["a", "b"]),
                multi_select: false,
            },
            ToolInput::CodeIndex {
                path: Some("src".into()),
                query: Some("parser".into()),
                kind: Some("function".into()),
                max_entries: Some(10),
            },
            ToolInput::GraphQuery {
                query: "entrypoints".into(),
                max_tokens: Some(4000),
                include_handles: Some(true),
            },
            ToolInput::Mcp {
                name: "mcp__fs__read".into(),
                arguments: serde_json::json!({"path": "Cargo.toml"}),
            },
            ToolInput::ScratchpadWrite {
                key: "note".into(),
                value: "body".into(),
            },
        ];

        for input in samples {
            assert!(
                !matches!(
                    serialize_tool_input(&input),
                    SerializedToolInput::Generic { .. }
                ),
                "{} should not fall back to Generic session input",
                input.summary()
            );
        }
    }

    #[test]
    fn generic_tool_input_legacy_ask_user_question_rehydrates_robust() {
        let input = deserialize_tool_input_for_kind(
            "AskUserQuestion",
            SerializedToolInput::Generic {
                summary: "AskUserQuestion: Pick a target: prod or staging?".into(),
            },
        );

        match input {
            ToolInput::AskUserQuestion {
                question,
                options,
                multi_select,
            } => {
                assert_eq!(question, "Pick a target: prod or staging?");
                assert_eq!(options, serde_json::json!([]));
                assert!(!multi_select);
            }
            other => panic!("expected AskUserQuestion, got {}", other.summary()),
        }
    }
}
