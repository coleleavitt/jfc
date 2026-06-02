//! On-disk serialization types and conversion functions for session messages.
//!
//! Owns all `Serialized*` structs/enums plus the `serialize_*` /
//! `deserialize_*` helpers that convert between runtime types and their
//! JSON-friendly counterparts.

use serde::{Deserialize, Serialize};

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

// Mirrors `MessagePart`'s shape for on-disk session round-tripping. Same
// rationale as MessagePart — the Tool variant is the dominant payload, and
// this enum exists for one purpose (serde) where size doesn't drive perf.
#[allow(clippy::large_enum_variant)]
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
        /// Gemini 3.x opaque thought signature captured from the stream.
        /// Must round-trip so `--continue` doesn't lose provider
        /// continuity — without it, the first replayed functionCall on a
        /// resumed session falls back to the synthetic token (or 400s).
        /// Optional + serde(default + skip-if-None) keeps old/non-Gemini
        /// session files byte-identical.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
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
        effort: Option<String>,
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
        #[serde(default)]
        include_history: Option<bool>,
        #[serde(default)]
        history_query: Option<String>,
    },
    TaskDone {
        task_id: String,
    },
    TaskStop {
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
        // Normalized `questions` array (1-4). `serde(default)` keeps old session
        // files (which stored `question`/`options`/`multi_select`) loadable —
        // those legacy fields are ignored and `questions` defaults to Null,
        // deserializing to an empty (display-only, already-completed) call.
        #[serde(default)]
        questions: serde_json::Value,
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
