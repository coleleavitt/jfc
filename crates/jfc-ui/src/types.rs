#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum McpStatus {
    Connected,
    Disabled,
    Error,
}

impl McpStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::Disabled => "Disabled",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct McpServerInfo {
    pub name: String,
    pub status: McpStatus,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LspStatus {
    Active,
    Inactive,
}

#[derive(Clone, Debug)]
pub struct LspServerInfo {
    pub name: String,
    pub status: LspStatus,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

impl ModelUsage {
    /// v126's `W_$()` (cli.js:197281): the visible "context tokens used"
    /// for the gauge — input + cache_creation + cache_read + output.
    /// All four count against the prompt/completion limit.
    pub fn total_context_tokens(&self) -> u64 {
        self.input_tokens + self.cache_write_tokens + self.cache_read_tokens + self.output_tokens
    }

    pub fn add_delta(&mut self, input: u32, output: u32, cache_read: u32, cache_write: u32) {
        self.input_tokens += input as u64;
        self.output_tokens += output as u64;
        self.cache_read_tokens += cache_read as u64;
        self.cache_write_tokens += cache_write as u64;
    }

    /// Apply a cumulative reading from a streaming provider that emits
    /// running totals (Anthropic `message_delta`) — subtract the previous
    /// in-turn baseline before adding to per-model totals so we don't
    /// triple-count. Returns the new baseline so the caller can persist
    /// it for the next event.
    ///
    /// Example: deltas {output: 10}, {output: 20}, {output: 30} arrive.
    /// Naive `add_delta` would give 60 total; this function gives 30.
    pub fn apply_cumulative(
        &mut self,
        cumulative: (u32, u32, u32, u32),
        baseline: (u32, u32, u32, u32),
    ) -> (u32, u32, u32, u32) {
        let (c_in, c_out, c_cr, c_cw) = cumulative;
        let (b_in, b_out, b_cr, b_cw) = baseline;
        let d_in = c_in.saturating_sub(b_in);
        let d_out = c_out.saturating_sub(b_out);
        let d_cr = c_cr.saturating_sub(b_cr);
        let d_cw = c_cw.saturating_sub(b_cw);
        if d_in > 0 || d_out > 0 || d_cr > 0 || d_cw > 0 {
            self.add_delta(d_in, d_out, d_cr, d_cw);
            cumulative
        } else {
            baseline
        }
    }

    pub fn cache_hit_pct(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        (self.cache_read_tokens as f64 / self.input_tokens as f64 * 100.0).min(100.0)
    }
}

#[cfg(test)]
mod cumulative_usage_tests {
    use super::ModelUsage;

    #[test]
    fn cumulative_deltas_dont_triple_count_normal() {
        // Anthropic streams 5 message_delta events for a single turn,
        // each carrying the running output_tokens count. Naive add_delta
        // produces 1+5+15+50+200 = 271; correct answer is the final 200.
        let mut u = ModelUsage::default();
        let mut baseline = (0u32, 0, 0, 0);
        for cum in [
            (100, 1, 0, 0),
            (100, 5, 0, 0),
            (100, 15, 0, 0),
            (100, 50, 0, 0),
            (100, 200, 0, 0),
        ] {
            baseline = u.apply_cumulative(cum, baseline);
        }
        assert_eq!(u.input_tokens, 100, "input shouldn't double-count");
        assert_eq!(u.output_tokens, 200, "output should be final cumulative");
    }

    #[test]
    fn second_turn_resets_baseline_normal() {
        // Each new turn the caller resets baseline to (0,0,0,0); the
        // function then correctly attributes the full new turn's count.
        let mut u = ModelUsage::default();
        let _ = u.apply_cumulative((100, 50, 0, 0), (0, 0, 0, 0));
        // Turn 2: caller passes baseline = (0,0,0,0) again
        let _ = u.apply_cumulative((80, 30, 0, 0), (0, 0, 0, 0));
        assert_eq!(u.input_tokens, 180, "two turns add: 100 + 80");
        assert_eq!(u.output_tokens, 80, "two turns add: 50 + 30");
    }

    #[test]
    fn no_op_when_cumulative_unchanged_robust() {
        // Some providers emit redundant usage events with the same count.
        // The apply should be a no-op (no double-charge, baseline unchanged).
        let mut u = ModelUsage::default();
        let b1 = u.apply_cumulative((100, 50, 0, 0), (0, 0, 0, 0));
        let b2 = u.apply_cumulative((100, 50, 0, 0), b1);
        assert_eq!(b1, b2, "baseline shouldn't move on duplicate event");
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 50);
    }

    #[test]
    fn saturating_handles_decreasing_cumulative_robust() {
        // If a provider misbehaves and reports a lower cumulative than
        // last time, saturating_sub yields zero — we don't underflow or
        // negatively adjust. The next higher reading recovers.
        let mut u = ModelUsage::default();
        let b1 = u.apply_cumulative((100, 50, 0, 0), (0, 0, 0, 0));
        let b2 = u.apply_cumulative((90, 30, 0, 0), b1); // bogus regression
        assert_eq!(b1, b2, "regression event must not move baseline");
        assert_eq!(u.output_tokens, 50, "no negative or wraparound charge");
        let _ = u.apply_cumulative((100, 80, 0, 0), b2);
        assert_eq!(u.output_tokens, 80, "next valid reading still works");
    }

    #[test]
    fn cache_tokens_apply_independently_robust() {
        let mut u = ModelUsage::default();
        let mut baseline = (0u32, 0, 0, 0);
        baseline = u.apply_cumulative((100, 0, 50, 0), baseline);
        baseline = u.apply_cumulative((100, 0, 75, 25), baseline);
        let _ = u.apply_cumulative((100, 0, 75, 100), baseline);
        assert_eq!(u.cache_read_tokens, 75);
        assert_eq!(u.cache_write_tokens, 100);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
pub enum MessagePart {
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
    TaskStatus(TaskStatusPart),
    CompactBoundary { pre_tokens: usize },
}

impl MessagePart {
    pub fn approx_text_len(&self) -> usize {
        match self {
            Self::Text(s) | Self::Reasoning(s) => s.len(),
            Self::Tool(tc) => tc.input.summary().len() + tc.output.approx_text_len(),
            Self::TaskStatus(ts) => {
                ts.description.len() + ts.summary.as_deref().map_or(0, |s| s.len())
            }
            Self::CompactBoundary { .. } => 0,
        }
    }

    pub fn text_only(&self) -> String {
        match self {
            Self::Text(s) | Self::Reasoning(s) => s.clone(),
            Self::Tool(tc) => {
                format!("[Tool: {} → {}]", tc.kind.label(), tc.output.text_only())
            }
            Self::TaskStatus(ts) => {
                format!("[Task {}: {}]", ts.task_id, ts.description)
            }
            Self::CompactBoundary { pre_tokens } => {
                format!("[Compact boundary, pre={pre_tokens} tokens]")
            }
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Reasoning(s) => format!("[Reasoning: {}]", s),
            Self::Tool(tc) => {
                format!(
                    "[Tool: {} | Input: {} | Output: {}]",
                    tc.kind.label(),
                    tc.input.summary(),
                    tc.output.to_display_string(),
                )
            }
            Self::TaskStatus(ts) => {
                format!(
                    "[Task {} | {} | {:?}]",
                    ts.task_id, ts.description, ts.status
                )
            }
            Self::CompactBoundary { pre_tokens } => {
                format!("[Compact boundary, pre={pre_tokens} tokens]")
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    pub input: ToolInput,
    pub output: ToolOutput,
    /// True when the tool block renders as a single-line collapsed
    /// header (set on huge outputs that would otherwise dominate the
    /// chat). Distinct from `expanded`: this is the *minimal* state.
    pub is_collapsed: bool,
    /// True when the user has explicitly expanded the tool to full
    /// content via `Ctrl+O` (or click). False = preview-cap state
    /// (first `TOOL_PREVIEW_LINES` rows + a `… N more` truncation
    /// row). Mirrors v126's per-tool expand/collapse affordance —
    /// long stdout, multi-hunk diffs, and big file reads all start
    /// preview-only so they don't drown out the rest of the chat.
    pub expanded: bool,
    /// Wall-clock millis between the tool's dispatch and its result
    /// landing. `None` while the tool is in flight. Set by the
    /// `ToolResult` handler in `main.rs`. Surfaced in the title as
    /// a muted `[2.3s]` badge so the user can spot slow operations
    /// at a glance.
    pub elapsed_ms: Option<u64>,
    /// Wall-clock instant when the tool transitioned into flight —
    /// captured at construction and used to compute `elapsed_ms` on
    /// completion. Not persisted (recomputing the duration after a
    /// session reload is meaningless), so this isn't serialized.
    pub started_at: Option<std::time::Instant>,
    /// True when the user has double-clicked the tool to pin it
    /// expanded. The renderer adds a small 📌 glyph to the title
    /// and the click handler resists toggling it off — only an
    /// explicit double-click can unpin. Without this, a long Read
    /// the user wants to keep visible while scrolling around can
    /// silently re-collapse.
    pub pinned: bool,
}

/// The lifecycle state of a spawned sub-agent task.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TaskLifecycle {
    Pending,
    Running,
    /// Teammate finished its turn but is still alive — waiting for the next
    /// inbound message before it picks up again. Distinct from Running so
    /// the task panel can stop its "Receiving output…" spinner without
    /// having to mark the task terminal (the agent could resume on the
    /// next SendMessage).
    Idle,
    Completed,
    Failed,
    Cancelled,
}

impl TaskLifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Counts as "alive" for fan-out / agent-count purposes. Running and
    /// Idle teammates both still belong on the agent fan even though
    /// only Running ones are actively producing output.
    pub fn is_alive(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Idle)
    }
}

#[derive(Clone, Debug)]
pub struct TaskStatusPart {
    pub task_id: String,
    pub description: String,
    pub status: TaskLifecycle,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TaskInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: Option<String>,
    pub category: Option<String>,
    pub run_in_background: bool,
    pub model: Option<String>,
    /// Name for the spawned agent — makes it addressable via SendMessage.
    /// When set along with `team_name`, spawns a persistent teammate instead
    /// of a one-shot subagent.
    pub name: Option<String>,
    /// Team to spawn the agent into. Uses current team context if omitted.
    pub team_name: Option<String>,
    /// Permission mode for the spawned teammate (e.g., "plan" to require approval).
    pub mode: Option<String>,
    /// Isolation mode: "worktree" creates a temp git worktree for the agent.
    pub isolation: Option<String>,
}

impl TaskInput {
    pub fn summary(&self) -> String {
        if let Some(ref name) = self.name {
            format!("spawn teammate: {name} — {}", self.description)
        } else {
            format!(
                "{} ({})",
                self.description,
                if self.run_in_background {
                    "background"
                } else {
                    "foreground"
                }
            )
        }
    }

    /// Whether this Task invocation should spawn a persistent teammate
    /// rather than a one-shot subagent.
    pub fn is_teammate_spawn(&self) -> bool {
        self.name.is_some() && self.team_name.is_some()
    }

    /// Whether this is a fork (no subagent_type specified). Forks inherit
    /// the parent's full conversation context and share the prompt cache.
    /// This is the cheapest delegation path.
    pub fn is_fork(&self) -> bool {
        self.subagent_type.is_none() && !self.is_teammate_spawn()
    }
}

#[derive(Clone, Debug)]
pub struct LargeText {
    pub content: String,
    pub line_count: usize,
    pub byte_count: usize,
}

impl LargeText {
    pub const COLLAPSE_LINES: usize = 500;
    pub const COLLAPSE_BYTES: usize = 30_720;

    pub fn new(content: String) -> Self {
        let line_count = content.lines().count();
        let byte_count = content.len();
        Self {
            content,
            line_count,
            byte_count,
        }
    }

    pub fn should_collapse(text: &str) -> bool {
        text.len() > Self::COLLAPSE_BYTES || text.lines().count() > Self::COLLAPSE_LINES
    }

    pub fn size_label(&self) -> String {
        let kb = self.byte_count as f64 / 1024.0;
        format!("{} lines · {:.1} KB", self.line_count, kb)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolKind {
    Edit,
    Write,
    Read,
    Bash,
    Glob,
    Grep,
    Search,
    ApplyPatch,
    TaskCreate,
    TaskUpdate,
    TaskList,
    TaskDone,
    Task,
    Skill,
    MemoryCreate,
    MemoryDelete,
    TeamCreate,
    TeamDelete,
    SendMessage,
    /// Change a teammate's permission mode at runtime — wraps
    /// `swarm::team_helpers::set_member_mode`. Lets the leader
    /// promote/demote a teammate (e.g. plan → default) without
    /// respawning it.
    TeamMemberMode,
    Generic(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolStatus {
    Pending,
    Running,
    Complete,
    Failed,
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum ToolInput {
    Edit {
        file_path: String,
        old_string: String,
        new_string: String,
        replacement: ReplacementMode,
    },
    Write {
        file_path: String,
        content: String,
    },
    Read {
        file_path: String,
        offset: Option<u64>,
        limit: Option<u64>,
    },
    Bash {
        command: String,
        timeout: Option<u64>,
        workdir: Option<String>,
    },
    Glob {
        pattern: String,
        path: Option<String>,
    },
    Grep {
        pattern: String,
        path: Option<String>,
        glob: Option<String>,
        output_mode: Option<String>,
    },
    Search {
        query: String,
        path: Option<String>,
    },
    ApplyPatch {
        patch: String,
    },
    Task(TaskInput),
    TaskCreate {
        subject: String,
        description: String,
        active_form: Option<String>,
        blocked_by: Vec<String>,
    },
    TaskUpdate {
        task_id: String,
        status: Option<String>,
        subject: Option<String>,
        description: Option<String>,
        owner: Option<String>,
    },
    TaskList {
        status_filter: Option<String>,
        owner_filter: Option<String>,
    },
    TaskDone {
        task_id: String,
    },
    Skill {
        name: String,
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
    TeamCreate {
        team_name: String,
        description: Option<String>,
    },
    TeamDelete,
    SendMessage {
        to: String,
        message: String,
        summary: Option<String>,
    },
    TeamMemberMode {
        member_name: String,
        mode: String,
    },
    Generic {
        summary: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum ReplacementMode {
    FirstOnly,
    All,
}

impl ReplacementMode {
    pub fn from_replace_all(replace_all: bool) -> Self {
        if replace_all {
            Self::All
        } else {
            Self::FirstOnly
        }
    }

    pub fn replace_all(self) -> bool {
        matches!(self, Self::All)
    }
}

#[derive(Clone, Debug)]
pub enum ToolOutput {
    Text(String),
    LargeText(LargeText),
    Diff(DiffView),
    FileContent {
        path: String,
        content: String,
        language: String,
    },
    Command {
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    FileList(Vec<String>),
    Empty,
}

impl ToolOutput {
    /// Mirror of the wire-format truncation cap in `stream.rs`
    /// (`MAX_TOOL_RESULT_CHARS`). The API only ever sees a tool result
    /// shortened to this many bytes, so the local token estimate must cap
    /// here too — otherwise a 500KB Read output makes `compact_level` think
    /// the context is full when the API only received 30KB of it. That
    /// mismatch is what made compaction trigger on every tool batch with a
    /// large file in it.
    pub const APPROX_LEN_CAP: usize = 30_000;

    pub fn approx_text_len(&self) -> usize {
        let raw = match self {
            Self::Text(s) => s.len(),
            Self::LargeText(lt) => lt.byte_count,
            Self::Diff(d) => d
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .map(|l| l.content.len())
                .sum(),
            Self::FileContent { content, .. } => content.len(),
            Self::Command { stdout, stderr, .. } => stdout.len() + stderr.len(),
            Self::FileList(files) => files.iter().map(|f| f.len()).sum(),
            Self::Empty => 0,
        };
        raw.min(Self::APPROX_LEN_CAP)
    }

    pub fn text_only(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::LargeText(lt) => format!("[large: {}]", lt.size_label()),
            Self::Diff(d) => format!("{} (+{}/-{})", d.file_path, d.additions, d.deletions),
            Self::FileContent { path, .. } => format!("[file: {}]", path),
            Self::Command {
                stdout,
                stderr,
                exit_code,
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                format!(
                    "exit={} stdout={}B stderr={}B",
                    code,
                    stdout.len(),
                    stderr.len()
                )
            }
            Self::FileList(files) => format!("{} files", files.len()),
            Self::Empty => String::new(),
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::LargeText(lt) => lt.content.clone(),
            Self::Diff(d) => format!("{} (+{}/-{})", d.file_path, d.additions, d.deletions),
            Self::FileContent { path, content, .. } => {
                format!("{} ({} chars)", path, content.len())
            }
            Self::Command {
                stdout, exit_code, ..
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                let preview = if stdout.len() > 100 {
                    format!("{}...", &stdout[..100])
                } else {
                    stdout.clone()
                };
                format!("exit={}: {}", code, preview)
            }
            Self::FileList(files) => format!("{} files", files.len()),
            Self::Empty => "[empty]".into(),
        }
    }

    pub fn to_api_text(&self) -> String {
        match self {
            Self::LargeText(lt) => lt.content.clone(),
            other => other.to_display_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiffView {
    pub file_path: String,
    pub hunks: Vec<DiffHunk>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub old_start: usize,
    pub new_start: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
}

#[derive(Clone, Copy, Debug)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug)]
pub struct ChatMessage {
    pub role: Role,
    pub parts: Vec<MessagePart>,
    pub agent_name: Option<String>,
    pub model_name: Option<String>,
    pub cost_tier: Option<String>,
    pub elapsed: Option<String>,
    /// Token usage as of the END of this assistant turn. Set on
    /// `StreamUsage` (via `apply_to_last_assistant`) so when the
    /// session is later resumed, `App::recompute_token_estimate` can
    /// walk backwards to the last assistant message with usage and
    /// re-seat the Context gauge at the correct value. Mirrors v126's
    /// `Wd(messages)` (cli.js:197282-197294) which finds the last
    /// usage block and totals input + cache_read + cache_write +
    /// output.
    pub usage: Option<ModelUsage>,
}

impl ChatMessage {
    pub fn user(content: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        }
    }

    pub fn assistant(content: String) -> Self {
        // No placeholder values — fields are set authentically by the
        // stream pipeline (`elapsed` at StreamDone via `Cooked for Xs`,
        // `model_name` from the active provider). Earlier hardcoded
        // strings ("Sisyphus - Ultraworker", "$$$$", "3.9s") leaked into
        // session.json files and showed up under loaded sessions before
        // the next turn could overwrite them.
        Self {
            role: Role::Assistant,
            parts: vec![MessagePart::Text(content)],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        }
    }

    pub fn assistant_parts(parts: Vec<MessagePart>) -> Self {
        Self {
            role: Role::Assistant,
            parts,
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        }
    }

    pub fn compact_boundary(summary: &str, pre_tokens: usize) -> Self {
        Self {
            role: Role::User,
            parts: vec![
                MessagePart::CompactBoundary { pre_tokens },
                MessagePart::Text(format!(
                    "This session is being continued from a previous conversation that ran out of context. \
                     The summary below covers the earlier portion of the conversation.\n\n\
                     {summary}\n\n\
                     Continue the conversation from where it left off without asking further questions. \
                     Resume directly — do not acknowledge the summary, do not recap what was happening, \
                     do not preface with \"I'll continue\" or similar. Pick up the last task as if the break never happened."
                )),
            ],
            agent_name: Some("system".into()),
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        }
    }

    pub fn role_is_user(&self) -> bool {
        self.role == Role::User
    }

    pub fn is_compact_boundary(&self) -> bool {
        self.parts
            .iter()
            .any(|p| matches!(p, MessagePart::CompactBoundary { .. }))
    }
}

impl ToolKind {
    pub fn from_name(name: &str) -> Self {
        // Normalize: lowercase + strip underscores. v126's native names
        // are PascalCase ("TaskCreate"), Anthropic's structured-tools are
        // snake_case ("apply_patch"), and OWUI/LiteLLM cross-provider
        // proxies sometimes flatten to lowercase-concatenated
        // ("taskcreate"). Without this, a Bedrock/OWUI session sees
        // `taskcreate` arrive as a Generic tool and the dispatcher
        // returns "not yet implemented" — exactly what the user hit.
        let norm = name.to_ascii_lowercase().replace('_', "");
        match norm.as_str() {
            "edit" | "strreplacebasededittool" => Self::Edit,
            "write" | "writefile" => Self::Write,
            "read" | "readfile" => Self::Read,
            "bash" | "runbash" => Self::Bash,
            "glob" => Self::Glob,
            "grep" => Self::Grep,
            "codebasesearch" | "search" => Self::Search,
            "applypatch" => Self::ApplyPatch,
            "taskcreate" => Self::TaskCreate,
            "taskupdate" => Self::TaskUpdate,
            "tasklist" => Self::TaskList,
            "taskdone" => Self::TaskDone,
            "task" => Self::Task,
            "skill" => Self::Skill,
            "memorycreate" => Self::MemoryCreate,
            "memorydelete" => Self::MemoryDelete,
            "teamcreate" => Self::TeamCreate,
            "teamdelete" => Self::TeamDelete,
            "sendmessage" => Self::SendMessage,
            "teammembermode" => Self::TeamMemberMode,
            _ => Self::Generic(name.to_owned()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "Search",
            Self::ApplyPatch => "Patch",
            Self::TaskCreate => "TaskCreate",
            Self::TaskUpdate => "TaskUpdate",
            Self::TaskList => "TaskList",
            Self::TaskDone => "TaskDone",
            Self::Task => "Task",
            Self::Skill => "Skill",
            Self::MemoryCreate => "MemoryCreate",
            Self::MemoryDelete => "MemoryDelete",
            Self::TeamCreate => "TeamCreate",
            Self::TeamDelete => "TeamDelete",
            Self::SendMessage => "SendMessage",
            Self::TeamMemberMode => "TeamMemberMode",
            Self::Generic(name) => name.as_str(),
        }
    }

    pub fn api_name(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "codebase_search",
            Self::ApplyPatch => "apply_patch",
            Self::TaskCreate => "TaskCreate",
            Self::TaskUpdate => "TaskUpdate",
            Self::TaskList => "TaskList",
            Self::TaskDone => "TaskDone",
            Self::Task => "Task",
            Self::Skill => "Skill",
            Self::MemoryCreate => "MemoryCreate",
            Self::MemoryDelete => "MemoryDelete",
            Self::TeamCreate => "TeamCreate",
            Self::TeamDelete => "TeamDelete",
            Self::SendMessage => "SendMessage",
            Self::TeamMemberMode => "TeamMemberMode",
            Self::Generic(name) => name.as_str(),
        }
    }
}

impl ToolStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Complete => "done",
            Self::Failed => "failed",
        }
    }
}

impl ToolInput {
    pub fn summary(&self) -> String {
        match self {
            Self::Edit { file_path, .. } => file_path.clone(),
            Self::Write { file_path, .. } => file_path.clone(),
            Self::Read { file_path, .. } => file_path.clone(),
            Self::Bash {
                command, workdir, ..
            } => match workdir {
                Some(workdir) => format!("{command} in {workdir}"),
                None => command.clone(),
            },
            Self::Glob { pattern, path } => match path {
                Some(path) => format!("{pattern} in {path}"),
                None => pattern.clone(),
            },
            Self::Grep { pattern, path, .. } => match path {
                Some(path) => format!("{pattern} in {path}"),
                None => pattern.clone(),
            },
            Self::Search { query, path } => match path {
                Some(path) => format!("{query} in {path}"),
                None => query.clone(),
            },
            Self::ApplyPatch { patch } => format!("apply patch ({} bytes)", patch.len()),
            Self::TaskCreate { subject, .. } => format!("create: {subject}"),
            Self::TaskUpdate { task_id, .. } => format!("update: {task_id}"),
            Self::TaskList { status_filter, .. } => match status_filter {
                Some(f) => format!("list tasks ({f})"),
                None => "list tasks".into(),
            },
            Self::TaskDone { task_id } => format!("done: {task_id}"),
            Self::Task(ti) => ti.summary(),
            Self::Skill { name, args } => match args.as_deref().filter(|s| !s.is_empty()) {
                Some(a) => format!("{name}: {a}"),
                None => name.clone(),
            },
            Self::MemoryCreate { body, level, .. } => {
                let preview: String = body.chars().take(50).collect();
                format!("remember ({level}): {preview}")
            }
            Self::MemoryDelete { path } => format!("forget: {path}"),
            Self::TeamCreate { team_name, .. } => format!("create team: {team_name}"),
            Self::TeamDelete => "cleanup team".into(),
            Self::SendMessage { to, summary, .. } => match summary {
                Some(s) => format!("→ {to}: {s}"),
                None => format!("→ {to}"),
            },
            Self::TeamMemberMode { member_name, mode } => {
                format!("set {member_name} → {mode}")
            }
            Self::Generic { summary } => summary.clone(),
        }
    }

    pub fn from_value(tool_name: &str, v: serde_json::Value) -> Self {
        let obj = match &v {
            serde_json::Value::Object(m) => Some(m),
            _ => None,
        };
        let str_field = |key: &str| -> String {
            obj.and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned()
        };
        let opt_str_field = |key: &str| -> Option<String> {
            obj.and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        };
        let opt_u64_field =
            |key: &str| -> Option<u64> { obj.and_then(|m| m.get(key)).and_then(|v| v.as_u64()) };
        let bool_field = |key: &str| -> bool {
            obj.and_then(|m| m.get(key))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };
        match ToolKind::from_name(tool_name) {
            ToolKind::Edit => Self::Edit {
                file_path: str_field("file_path"),
                old_string: str_field("old_string"),
                new_string: str_field("new_string"),
                replacement: ReplacementMode::from_replace_all(bool_field("replace_all")),
            },
            ToolKind::Write => Self::Write {
                file_path: str_field("file_path"),
                content: str_field("content"),
            },
            ToolKind::Read => Self::Read {
                file_path: str_field("file_path"),
                offset: opt_u64_field("offset"),
                limit: opt_u64_field("limit"),
            },
            ToolKind::Bash => Self::Bash {
                command: str_field("command"),
                timeout: opt_u64_field("timeout"),
                workdir: opt_str_field("workdir"),
            },
            ToolKind::Glob => Self::Glob {
                pattern: str_field("pattern"),
                path: opt_str_field("path"),
            },
            ToolKind::Grep => Self::Grep {
                pattern: str_field("pattern"),
                path: opt_str_field("path"),
                glob: opt_str_field("glob"),
                output_mode: opt_str_field("output_mode"),
            },
            ToolKind::Search => Self::Search {
                query: str_field("query"),
                path: opt_str_field("path"),
            },
            ToolKind::ApplyPatch => Self::ApplyPatch {
                patch: str_field("patch"),
            },
            ToolKind::TaskCreate => {
                let blocked_by = obj
                    .and_then(|m| m.get("blocked_by"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                Self::TaskCreate {
                    subject: str_field("subject"),
                    description: str_field("description"),
                    active_form: opt_str_field("active_form"),
                    blocked_by,
                }
            }
            ToolKind::TaskUpdate => Self::TaskUpdate {
                task_id: str_field("task_id"),
                status: opt_str_field("status"),
                subject: opt_str_field("subject"),
                description: opt_str_field("description"),
                owner: opt_str_field("owner"),
            },
            ToolKind::TaskList => Self::TaskList {
                status_filter: opt_str_field("status_filter"),
                owner_filter: opt_str_field("owner_filter"),
            },
            ToolKind::TaskDone => Self::TaskDone {
                task_id: str_field("task_id"),
            },
            ToolKind::Task => Self::Task(TaskInput {
                description: str_field("description"),
                prompt: str_field("prompt"),
                subagent_type: opt_str_field("subagent_type"),
                category: opt_str_field("category"),
                run_in_background: bool_field("run_in_background"),
                model: opt_str_field("model"),
                name: opt_str_field("name"),
                team_name: opt_str_field("team_name"),
                mode: opt_str_field("mode"),
                isolation: opt_str_field("isolation"),
            }),
            ToolKind::Skill => Self::Skill {
                name: str_field("name"),
                args: opt_str_field("args"),
            },
            ToolKind::MemoryCreate => Self::MemoryCreate {
                level: str_field("level"),
                memory_type: str_field("memory_type"),
                scope: str_field("scope"),
                body: str_field("body"),
            },
            ToolKind::MemoryDelete => Self::MemoryDelete {
                path: str_field("path"),
            },
            ToolKind::TeamCreate => Self::TeamCreate {
                team_name: str_field("team_name"),
                description: v.get("description").and_then(|d| d.as_str()).map(str::to_owned),
            },
            ToolKind::TeamDelete => Self::TeamDelete,
            ToolKind::SendMessage => Self::SendMessage {
                to: str_field("to"),
                message: v
                    .get("message")
                    .map(|m| {
                        if let Some(s) = m.as_str() {
                            s.to_owned()
                        } else {
                            m.to_string()
                        }
                    })
                    .unwrap_or_default(),
                summary: v.get("summary").and_then(|s| s.as_str()).map(str::to_owned),
            },
            ToolKind::TeamMemberMode => Self::TeamMemberMode {
                member_name: str_field("member_name"),
                mode: str_field("mode"),
            },
            ToolKind::Generic(_) => Self::Generic {
                summary: v.to_string(),
            },
        }
    }

    pub fn to_value(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Self::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            } => {
                let mut v = json!({ "file_path": file_path, "old_string": old_string, "new_string": new_string });
                if replacement.replace_all() {
                    v["replace_all"] = json!(true);
                }
                v
            }
            Self::Write { file_path, content } => {
                json!({ "file_path": file_path, "content": content })
            }
            Self::Read {
                file_path,
                offset,
                limit,
            } => {
                let mut v = json!({ "file_path": file_path });
                if let Some(o) = offset {
                    v["offset"] = json!(o);
                }
                if let Some(l) = limit {
                    v["limit"] = json!(l);
                }
                v
            }
            Self::Bash {
                command,
                timeout,
                workdir,
            } => {
                let mut v = json!({ "command": command });
                if let Some(t) = timeout {
                    v["timeout"] = json!(t);
                }
                if let Some(w) = workdir {
                    v["workdir"] = json!(w);
                }
                v
            }
            Self::Glob { pattern, path } => {
                let mut v = json!({ "pattern": pattern });
                if let Some(p) = path {
                    v["path"] = json!(p);
                }
                v
            }
            Self::Grep {
                pattern,
                path,
                glob,
                output_mode,
            } => {
                let mut v = json!({ "pattern": pattern });
                if let Some(p) = path {
                    v["path"] = json!(p);
                }
                if let Some(g) = glob {
                    v["glob"] = json!(g);
                }
                if let Some(m) = output_mode {
                    v["output_mode"] = json!(m);
                }
                v
            }
            Self::Search { query, path } => {
                let mut v = json!({ "query": query });
                if let Some(p) = path {
                    v["path"] = json!(p);
                }
                v
            }
            Self::ApplyPatch { patch } => json!({ "patch": patch }),
            Self::TaskCreate {
                subject,
                description,
                active_form,
                blocked_by,
            } => {
                let mut v = json!({ "subject": subject, "description": description });
                if let Some(af) = active_form {
                    v["active_form"] = json!(af);
                }
                if !blocked_by.is_empty() {
                    v["blocked_by"] = json!(blocked_by);
                }
                v
            }
            Self::TaskUpdate {
                task_id,
                status,
                subject,
                description,
                owner,
            } => {
                let mut v = json!({ "task_id": task_id });
                if let Some(s) = status {
                    v["status"] = json!(s);
                }
                if let Some(s) = subject {
                    v["subject"] = json!(s);
                }
                if let Some(d) = description {
                    v["description"] = json!(d);
                }
                if let Some(o) = owner {
                    v["owner"] = json!(o);
                }
                v
            }
            Self::TaskList {
                status_filter,
                owner_filter,
            } => {
                let mut v = json!({});
                if let Some(f) = status_filter {
                    v["status_filter"] = json!(f);
                }
                if let Some(f) = owner_filter {
                    v["owner_filter"] = json!(f);
                }
                v
            }
            Self::TaskDone { task_id } => json!({ "task_id": task_id }),
            Self::Task(ti) => {
                let mut v = json!({
                    "description": ti.description,
                    "prompt": ti.prompt,
                    "run_in_background": ti.run_in_background,
                });
                if let Some(s) = &ti.subagent_type {
                    v["subagent_type"] = json!(s);
                }
                if let Some(c) = &ti.category {
                    v["category"] = json!(c);
                }
                if let Some(m) = &ti.model {
                    v["model"] = json!(m);
                }
                v
            }
            Self::Skill { name, args } => {
                let mut v = json!({ "name": name });
                if let Some(a) = args {
                    v["args"] = json!(a);
                }
                v
            }
            Self::MemoryCreate {
                level,
                memory_type,
                scope,
                body,
            } => json!({
                "level": level,
                "memory_type": memory_type,
                "scope": scope,
                "body": body,
            }),
            Self::MemoryDelete { path } => json!({ "path": path }),
            Self::TeamCreate {
                team_name,
                description,
            } => {
                let mut v = json!({ "team_name": team_name });
                if let Some(d) = description {
                    v["description"] = json!(d);
                }
                v
            }
            Self::TeamDelete => json!({}),
            Self::SendMessage {
                to,
                message,
                summary,
            } => {
                let mut v = json!({ "to": to, "message": message });
                if let Some(s) = summary {
                    v["summary"] = json!(s);
                }
                v
            }
            Self::TeamMemberMode { member_name, mode } => {
                json!({ "member_name": member_name, "mode": mode })
            }
            Self::Generic { summary } => {
                serde_json::from_str(summary).unwrap_or(json!({ "input": summary }))
            }
        }
    }
}

pub fn sample_tool_harness_message() -> ChatMessage {
    let diff = parse_unified_diff(
        "crates/jfc-ui/src/tools.rs",
        r#"@@ -180,2 +180,2 @@
-async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
-    let timeout = timeout_ms.unwrap_or(120_000);
+async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
+    let timeout = timeout_ms.unwrap_or(300_000);
"#,
    );

    ChatMessage::assistant_parts(vec![
        MessagePart::Reasoning("Increase default bash timeout from 2min to 5min.".into()),
        MessagePart::Tool(ToolCall {
            id: "edit-1".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Complete,
            input: ToolInput::Edit {
                file_path: "crates/jfc-ui/src/tools.rs".into(),
                old_string: "let timeout = timeout_ms.unwrap_or(120_000);".into(),
                new_string: "let timeout = timeout_ms.unwrap_or(300_000);".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "bash-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Complete,
            input: ToolInput::Bash {
                command: "cargo check -p jfc-ui".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Command {
                stdout: "Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.38s"
                    .into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "read-1".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Complete,
            input: ToolInput::Read {
                file_path: "crates/jfc-ui/src/main.rs".into(),
                offset: Some(1),
                limit: Some(80),
            },
            output: ToolOutput::FileContent {
                path: "crates/jfc-ui/src/main.rs".into(),
                language: "rust".into(),
                content: "mod app;\nmod context;\n\nuse std::sync::Arc;\nuse tokio::sync::mpsc;"
                    .into(),
            },
            is_collapsed: true,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "write-1".into(),
            kind: ToolKind::Write,
            status: ToolStatus::Pending,
            input: ToolInput::Write {
                file_path: "crates/jfc-ui/src/tool_harness.rs".into(),
                content: "pub enum MessagePart { Text(String), Tool(ToolCall) }".into(),
            },
            output: ToolOutput::Text("Waiting for approval".into()),
            is_collapsed: true,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "search-1".into(),
            kind: ToolKind::Search,
            status: ToolStatus::Running,
            input: ToolInput::Search {
                query: "ToolRegistry|DiffChanges|tool_result".into(),
                path: Some("research/opencode".into()),
            },
            output: ToolOutput::FileList(vec![
                "packages/ui/src/components/message-part.tsx".into(),
                "packages/ui/src/components/diff-changes.tsx".into(),
                "packages/opencode/src/tool/edit.ts".into(),
            ]),
            is_collapsed: true,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "patch-1".into(),
            kind: ToolKind::ApplyPatch,
            status: ToolStatus::Complete,
            input: ToolInput::ApplyPatch {
                patch: "*** Begin Patch\n*** Update File: crates/jfc-ui/src/main.rs".into(),
            },
            output: ToolOutput::Diff(parse_unified_diff(
                "crates/jfc-ui/src/main.rs",
                r#"@@ -10,1 +10,1 @@
-struct ChatMessage;
+enum MessagePart;
"#,
            )),
            is_collapsed: true,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
        MessagePart::Tool(ToolCall {
            id: "generic-1".into(),
            kind: ToolKind::Generic("Delegate".into()),
            status: ToolStatus::Failed,
            input: ToolInput::Generic {
                summary: "OpenClaude remote lookup".into(),
            },
            output: ToolOutput::Empty,
            is_collapsed: true,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }),
    ])
}

pub fn parse_unified_diff(file_path: &str, patch: &str) -> DiffView {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut additions = 0usize;
    let mut deletions = 0usize;

    for raw_line in patch.lines() {
        if raw_line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }

            let (old_start, new_start, header) = parse_hunk_header(raw_line);
            old_line = old_start;
            new_line = new_start;
            current = Some(DiffHunk {
                old_start,
                new_start,
                header,
                lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            continue;
        };

        let (kind, content) = match raw_line.chars().next() {
            Some('+') => (DiffLineKind::Added, &raw_line[1..]),
            Some('-') => (DiffLineKind::Removed, &raw_line[1..]),
            Some(' ') => (DiffLineKind::Context, &raw_line[1..]),
            _ => (DiffLineKind::Context, raw_line),
        };

        match kind {
            DiffLineKind::Added => {
                additions += 1;
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: None,
                    new_line: Some(new_line),
                    content: content.into(),
                });
                new_line += 1;
            }
            DiffLineKind::Removed => {
                deletions += 1;
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: Some(old_line),
                    new_line: None,
                    content: content.into(),
                });
                old_line += 1;
            }
            DiffLineKind::Context => {
                hunk.lines.push(DiffLine {
                    kind,
                    old_line: Some(old_line),
                    new_line: Some(new_line),
                    content: content.into(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }

    if let Some(hunk) = current {
        hunks.push(hunk);
    }

    DiffView {
        file_path: file_path.into(),
        hunks,
        additions,
        deletions,
    }
}

pub fn parse_hunk_header(header: &str) -> (usize, usize, String) {
    let mut parts = header.split_whitespace();
    let _at = parts.next();
    let old = parts.next().unwrap_or("-1");
    let new = parts.next().unwrap_or("+1");
    let tail = parts.collect::<Vec<_>>().join(" ");
    (parse_hunk_start(old), parse_hunk_start(new), tail)
}

pub fn parse_hunk_start(token: &str) -> usize {
    token
        .trim_start_matches(['-', '+'])
        .split(',')
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
}

pub fn truncate_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<_> = text.lines().collect();
    let mut result = lines
        .iter()
        .take(max_lines)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > max_lines {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("… {} more lines", lines.len() - max_lines));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_input_json_snapshot_omits_default_replacement_mode() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::FirstOnly,
        };

        assert_eq!(
            input.to_value().to_string(),
            r#"{"file_path":"src/main.rs","old_string":"old","new_string":"new"}"#
        );
    }

    #[test]
    fn edit_input_json_snapshot_preserves_replace_all_wire_shape() {
        let input = ToolInput::Edit {
            file_path: "src/main.rs".into(),
            old_string: "old".into(),
            new_string: "new".into(),
            replacement: ReplacementMode::All,
        };

        assert_eq!(
            input.to_value().to_string(),
            r#"{"file_path":"src/main.rs","old_string":"old","new_string":"new","replace_all":true}"#
        );
    }

    #[test]
    fn large_text_collapses_above_threshold() {
        let short = "line\n".repeat(10);
        assert!(!LargeText::should_collapse(&short));

        let tall = "line\n".repeat(LargeText::COLLAPSE_LINES + 1);
        assert!(LargeText::should_collapse(&tall));

        let fat = "x".repeat(LargeText::COLLAPSE_BYTES + 1);
        assert!(LargeText::should_collapse(&fat));
    }

    #[test]
    fn large_text_size_label_formats_correctly() {
        let lt = LargeText::new("hello\nworld\n".into());
        assert_eq!(lt.line_count, 2);
        assert!(lt.size_label().contains("lines"));
        assert!(lt.size_label().contains("KB"));
    }

    #[test]
    fn task_lifecycle_is_terminal() {
        assert!(TaskLifecycle::Completed.is_terminal());
        assert!(TaskLifecycle::Failed.is_terminal());
        assert!(TaskLifecycle::Cancelled.is_terminal());
        assert!(!TaskLifecycle::Running.is_terminal());
        assert!(!TaskLifecycle::Pending.is_terminal());
    }

    #[test]
    fn task_input_summary_background_flag() {
        let fg = TaskInput {
            description: "do thing".into(),
            prompt: "please do it".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
        };
        assert!(fg.summary().contains("foreground"));

        let bg = TaskInput {
            run_in_background: true,
            ..fg
        };
        assert!(bg.summary().contains("background"));
    }

    #[test]
    fn task_input_to_value_roundtrip() {
        let input = ToolInput::Task(TaskInput {
            description: "research".into(),
            prompt: "find patterns".into(),
            subagent_type: Some("explore".into()),
            category: None,
            run_in_background: true,
            model: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
        });
        let v = input.to_value();
        assert_eq!(v["description"], "research");
        assert_eq!(v["subagent_type"], "explore");
        assert_eq!(v["run_in_background"], true);
        assert!(v.get("category").is_none() || v["category"].is_null());
    }

    #[test]
    fn tool_kind_task_parses_from_string() {
        assert_eq!(ToolKind::from_name("Task"), ToolKind::Task);
        assert_eq!(ToolKind::from_name("task"), ToolKind::Task);
    }

    #[test]
    fn tool_output_large_text_api_text_returns_full_content() {
        let lt = LargeText::new("abc\ndef\n".into());
        let out = ToolOutput::LargeText(lt);
        assert_eq!(out.to_api_text(), "abc\ndef\n");
    }

    // OWUI/LiteLLM cross-provider proxies sometimes flatten tool names
    // to lowercase-no-separator (`taskcreate` instead of `TaskCreate`).
    // Without normalization the dispatcher routes them to
    // `Generic("taskcreate")` and the user sees "not yet implemented"
    // even though we have a perfectly good handler. Mirrors v126's
    // case-insensitive tool routing behind setStreamMode("tool-input").
    #[test]
    fn from_name_handles_lowercase_concat_robust() {
        assert!(matches!(
            ToolKind::from_name("taskcreate"),
            ToolKind::TaskCreate
        ));
        assert!(matches!(
            ToolKind::from_name("taskupdate"),
            ToolKind::TaskUpdate
        ));
        assert!(matches!(
            ToolKind::from_name("tasklist"),
            ToolKind::TaskList
        ));
        assert!(matches!(
            ToolKind::from_name("taskdone"),
            ToolKind::TaskDone
        ));
        assert!(matches!(
            ToolKind::from_name("applypatch"),
            ToolKind::ApplyPatch
        ));
    }

    // The PascalCase, snake_case, and lowercase-concat variants must all
    // resolve to the same kind so a session that switched providers
    // mid-conversation doesn't fragment tool history.
    #[test]
    fn from_name_normalizes_across_separators_normal() {
        for n in ["TaskCreate", "task_create", "taskcreate", "TASKCREATE"] {
            assert!(
                matches!(ToolKind::from_name(n), ToolKind::TaskCreate),
                "expected TaskCreate for {n}"
            );
        }
    }

    // Truly unknown names still fall through to Generic — we don't want
    // to silently swallow a typo and dispatch the wrong tool.
    #[test]
    fn from_name_unknown_falls_through_to_generic_robust() {
        match ToolKind::from_name("not_a_real_tool") {
            ToolKind::Generic(s) => assert_eq!(s, "not_a_real_tool"),
            other => panic!("expected Generic, got {other:?}"),
        }
    }

    // ─── TaskLifecycle ────────────────────────────────────────────────────

    #[test]
    fn task_lifecycle_label_normal() {
        assert_eq!(TaskLifecycle::Pending.label(), "pending");
        assert_eq!(TaskLifecycle::Running.label(), "running");
        assert_eq!(TaskLifecycle::Idle.label(), "idle");
        assert_eq!(TaskLifecycle::Completed.label(), "completed");
        assert_eq!(TaskLifecycle::Failed.label(), "failed");
        assert_eq!(TaskLifecycle::Cancelled.label(), "cancelled");
    }

    #[test]
    fn task_lifecycle_is_alive_normal() {
        assert!(TaskLifecycle::Pending.is_alive());
        assert!(TaskLifecycle::Running.is_alive());
        assert!(TaskLifecycle::Idle.is_alive());
        assert!(!TaskLifecycle::Completed.is_alive());
        assert!(!TaskLifecycle::Failed.is_alive());
        assert!(!TaskLifecycle::Cancelled.is_alive());
    }

    #[test]
    fn task_lifecycle_terminal_and_alive_partition_robust() {
        // Every variant must be exactly one of: alive XOR terminal.
        // If a refactor adds a Limbo variant that's neither, this test
        // catches it before we ship a state the agent fan can't display.
        for state in [
            TaskLifecycle::Pending,
            TaskLifecycle::Running,
            TaskLifecycle::Idle,
            TaskLifecycle::Completed,
            TaskLifecycle::Failed,
            TaskLifecycle::Cancelled,
        ] {
            assert_ne!(
                state.is_alive(),
                state.is_terminal(),
                "{state:?} must be exactly one of alive/terminal",
            );
        }
    }

    // ─── McpStatus / LspStatus ────────────────────────────────────────────

    #[test]
    fn mcp_status_labels_normal() {
        assert_eq!(McpStatus::Connected.label(), "Connected");
        assert_eq!(McpStatus::Disabled.label(), "Disabled");
        assert_eq!(McpStatus::Error.label(), "Error");
    }

    // ─── ToolStatus ───────────────────────────────────────────────────────

    #[test]
    fn tool_status_labels_normal() {
        assert_eq!(ToolStatus::Pending.label(), "pending");
        assert_eq!(ToolStatus::Running.label(), "running");
        assert_eq!(ToolStatus::Complete.label(), "done");
        assert_eq!(ToolStatus::Failed.label(), "failed");
    }

    // ─── ReplacementMode ──────────────────────────────────────────────────

    #[test]
    fn replacement_mode_from_replace_all_normal() {
        assert_eq!(
            ReplacementMode::from_replace_all(true),
            ReplacementMode::All
        );
        assert_eq!(
            ReplacementMode::from_replace_all(false),
            ReplacementMode::FirstOnly
        );
    }

    #[test]
    fn replacement_mode_replace_all_normal() {
        assert!(ReplacementMode::All.replace_all());
        assert!(!ReplacementMode::FirstOnly.replace_all());
    }

    // ─── ToolKind labels & API names ──────────────────────────────────────

    #[test]
    fn tool_kind_label_returns_pascal_case_normal() {
        assert_eq!(ToolKind::Edit.label(), "Edit");
        assert_eq!(ToolKind::Write.label(), "Write");
        assert_eq!(ToolKind::Bash.label(), "Bash");
        assert_eq!(ToolKind::ApplyPatch.label(), "Patch");
        assert_eq!(ToolKind::Generic("Foo".into()).label(), "Foo");
    }

    #[test]
    fn tool_kind_api_name_for_search_uses_snake_case_normal() {
        // Search and ApplyPatch use snake_case on the wire even though
        // their display label is PascalCase. Mirrors v126's tool table.
        assert_eq!(ToolKind::Search.api_name(), "codebase_search");
        assert_eq!(ToolKind::ApplyPatch.api_name(), "apply_patch");
        assert_eq!(ToolKind::Edit.api_name(), "Edit");
    }

    // ─── TaskInput::is_teammate_spawn / is_fork ───────────────────────────

    fn make_task_input() -> TaskInput {
        TaskInput {
            description: "task".into(),
            prompt: "do it".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
        }
    }

    #[test]
    fn task_input_is_fork_when_no_subagent_or_team_normal() {
        let ti = make_task_input();
        assert!(ti.is_fork());
        assert!(!ti.is_teammate_spawn());
    }

    #[test]
    fn task_input_with_subagent_type_is_not_fork_normal() {
        let ti = TaskInput {
            subagent_type: Some("explore".into()),
            ..make_task_input()
        };
        assert!(!ti.is_fork());
        assert!(!ti.is_teammate_spawn());
    }

    #[test]
    fn task_input_teammate_spawn_requires_both_name_and_team_normal() {
        // name alone or team alone is not a teammate spawn.
        let only_name = TaskInput {
            name: Some("alice".into()),
            ..make_task_input()
        };
        assert!(!only_name.is_teammate_spawn());

        let only_team = TaskInput {
            team_name: Some("alpha".into()),
            ..make_task_input()
        };
        assert!(!only_team.is_teammate_spawn());

        let both = TaskInput {
            name: Some("alice".into()),
            team_name: Some("alpha".into()),
            ..make_task_input()
        };
        assert!(both.is_teammate_spawn());
    }

    #[test]
    fn task_input_teammate_spawn_excludes_fork_robust() {
        // is_fork() must return false for teammate spawns even though
        // subagent_type is None — otherwise the dispatcher would try the
        // fork path on a teammate.
        let teammate = TaskInput {
            name: Some("alice".into()),
            team_name: Some("alpha".into()),
            ..make_task_input()
        };
        assert!(!teammate.is_fork());
    }

    #[test]
    fn task_input_summary_teammate_format_normal() {
        let ti = TaskInput {
            name: Some("alice".into()),
            team_name: Some("alpha".into()),
            description: "deploy".into(),
            ..make_task_input()
        };
        let s = ti.summary();
        assert!(s.contains("spawn teammate: alice"), "{s}");
        assert!(s.contains("deploy"), "{s}");
    }

    // ─── LargeText ────────────────────────────────────────────────────────

    #[test]
    fn large_text_new_counts_lines_and_bytes_normal() {
        let lt = LargeText::new("a\nb\nc\n".into());
        assert_eq!(lt.line_count, 3);
        assert_eq!(lt.byte_count, 6);
    }

    #[test]
    fn large_text_should_not_collapse_below_thresholds_normal() {
        let s = "x".repeat(LargeText::COLLAPSE_BYTES);
        // Exactly at byte limit shouldn't collapse — the check is `>` not `>=`.
        assert!(!LargeText::should_collapse(&s));
    }

    #[test]
    fn large_text_size_label_includes_kilobytes_normal() {
        let lt = LargeText::new("x".repeat(2048));
        let label = lt.size_label();
        assert!(label.contains("KB"), "{label}");
        assert!(label.contains("lines"), "{label}");
    }

    // ─── ToolOutput::approx_text_len & APPROX_LEN_CAP ─────────────────────

    #[test]
    fn tool_output_approx_text_len_caps_at_30k_robust() {
        // Even a megabyte of text reports cap value — important for token
        // estimation against the truncated wire result.
        let huge = "x".repeat(2_000_000);
        let out = ToolOutput::Text(huge);
        assert_eq!(out.approx_text_len(), ToolOutput::APPROX_LEN_CAP);
    }

    #[test]
    fn tool_output_approx_text_len_command_combines_streams_normal() {
        let out = ToolOutput::Command {
            stdout: "abc".into(),
            stderr: "de".into(),
            exit_code: Some(0),
        };
        assert_eq!(out.approx_text_len(), 5);
    }

    #[test]
    fn tool_output_approx_text_len_empty_is_zero_normal() {
        assert_eq!(ToolOutput::Empty.approx_text_len(), 0);
    }

    #[test]
    fn tool_output_approx_text_len_filelist_sums_path_lens_normal() {
        let out = ToolOutput::FileList(vec!["abc".into(), "de".into()]);
        assert_eq!(out.approx_text_len(), 5);
    }

    #[test]
    fn tool_output_approx_text_len_diff_sums_line_content_normal() {
        let view = parse_unified_diff(
            "x.rs",
            "@@ -1,1 +1,1 @@\n-abc\n+abcd\n",
        );
        let out = ToolOutput::Diff(view);
        // "abc" (3) + "abcd" (4) = 7
        assert_eq!(out.approx_text_len(), 7);
    }

    #[test]
    fn tool_output_text_only_diff_includes_counts_normal() {
        let view = parse_unified_diff(
            "x.rs",
            "@@ -1,1 +1,1 @@\n-old\n+new\n",
        );
        let s = ToolOutput::Diff(view).text_only();
        assert!(s.contains("x.rs"), "{s}");
        assert!(s.contains("+1"), "{s}");
        assert!(s.contains("-1"), "{s}");
    }

    #[test]
    fn tool_output_text_only_command_renders_exit_code_normal() {
        let s = ToolOutput::Command {
            stdout: "ok".into(),
            stderr: String::new(),
            exit_code: Some(2),
        }
        .text_only();
        assert!(s.contains("exit=2"), "{s}");
        assert!(s.contains("stdout=2B"), "{s}");
    }

    #[test]
    fn tool_output_text_only_command_renders_question_mark_when_no_code_robust() {
        // exit_code: None (kill via signal, etc.) renders "?".
        let s = ToolOutput::Command {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
        }
        .text_only();
        assert!(s.contains("exit=?"), "{s}");
    }

    #[test]
    fn tool_output_text_only_filecontent_includes_path_normal() {
        let s = ToolOutput::FileContent {
            path: "src/main.rs".into(),
            content: "fn main() {}".into(),
            language: "rust".into(),
        }
        .text_only();
        assert!(s.contains("src/main.rs"), "{s}");
    }

    #[test]
    fn tool_output_text_only_filelist_count_normal() {
        let s = ToolOutput::FileList(vec!["a".into(), "b".into(), "c".into()])
            .text_only();
        assert_eq!(s, "3 files");
    }

    #[test]
    fn tool_output_to_display_string_command_truncates_at_100_chars_robust() {
        let huge = "x".repeat(200);
        let s = ToolOutput::Command {
            stdout: huge,
            stderr: String::new(),
            exit_code: Some(0),
        }
        .to_display_string();
        assert!(s.contains("..."), "expected ellipsis on truncation: {s}");
    }

    #[test]
    fn tool_output_to_display_string_empty_renders_marker_normal() {
        assert_eq!(ToolOutput::Empty.to_display_string(), "[empty]");
    }

    #[test]
    fn tool_output_to_api_text_falls_back_to_display_robust() {
        // Non-LargeText variants delegate to to_display_string.
        let t = ToolOutput::Text("hello".into());
        assert_eq!(t.to_api_text(), "hello");
    }

    // ─── ToolInput::summary ───────────────────────────────────────────────

    #[test]
    fn tool_input_summary_bash_with_workdir_appends_in_dir_normal() {
        let i = ToolInput::Bash {
            command: "ls".into(),
            timeout: None,
            workdir: Some("/tmp".into()),
        };
        assert_eq!(i.summary(), "ls in /tmp");
    }

    #[test]
    fn tool_input_summary_bash_without_workdir_is_command_only_normal() {
        let i = ToolInput::Bash {
            command: "ls -la".into(),
            timeout: None,
            workdir: None,
        };
        assert_eq!(i.summary(), "ls -la");
    }

    #[test]
    fn tool_input_summary_glob_grep_search_format_normal() {
        let g = ToolInput::Glob {
            pattern: "**/*.rs".into(),
            path: Some("crates".into()),
        };
        assert_eq!(g.summary(), "**/*.rs in crates");

        let gg = ToolInput::Grep {
            pattern: "todo".into(),
            path: None,
            glob: None,
            output_mode: None,
        };
        assert_eq!(gg.summary(), "todo");

        let s = ToolInput::Search {
            query: "auth".into(),
            path: Some("src".into()),
        };
        assert_eq!(s.summary(), "auth in src");
    }

    #[test]
    fn tool_input_summary_apply_patch_includes_byte_count_normal() {
        let i = ToolInput::ApplyPatch {
            patch: "*** Begin Patch\n*** End Patch\n".into(),
        };
        let s = i.summary();
        assert!(s.contains("apply patch"));
        assert!(s.contains("bytes"));
    }

    #[test]
    fn tool_input_summary_skill_renders_args_when_present_normal() {
        let with = ToolInput::Skill {
            name: "review".into(),
            args: Some("the PR".into()),
        };
        assert_eq!(with.summary(), "review: the PR");

        let without = ToolInput::Skill {
            name: "review".into(),
            args: None,
        };
        assert_eq!(without.summary(), "review");

        // Empty-string args is treated as "no args".
        let empty_args = ToolInput::Skill {
            name: "review".into(),
            args: Some(String::new()),
        };
        assert_eq!(empty_args.summary(), "review");
    }

    #[test]
    fn tool_input_summary_memory_create_truncates_body_at_50_robust() {
        let body = "x".repeat(200);
        let i = ToolInput::MemoryCreate {
            level: "user".into(),
            memory_type: "context".into(),
            scope: "private".into(),
            body,
        };
        let s = i.summary();
        // Format: "remember (user): xxxxx..." — count of x's is capped.
        let x_count = s.chars().filter(|c| *c == 'x').count();
        assert_eq!(x_count, 50, "body should truncate to 50 chars: {s}");
    }

    #[test]
    fn tool_input_summary_send_message_with_and_without_summary_normal() {
        let with = ToolInput::SendMessage {
            to: "alice".into(),
            message: "hi".into(),
            summary: Some("greeting".into()),
        };
        assert!(with.summary().contains("→ alice"));
        assert!(with.summary().contains("greeting"));

        let without = ToolInput::SendMessage {
            to: "bob".into(),
            message: "hi".into(),
            summary: None,
        };
        assert_eq!(without.summary(), "→ bob");
    }

    #[test]
    fn tool_input_summary_team_member_mode_format_normal() {
        let i = ToolInput::TeamMemberMode {
            member_name: "alice".into(),
            mode: "default".into(),
        };
        assert_eq!(i.summary(), "set alice → default");
    }

    #[test]
    fn tool_input_summary_team_create_includes_team_name_normal() {
        let i = ToolInput::TeamCreate {
            team_name: "frontend".into(),
            description: None,
        };
        assert_eq!(i.summary(), "create team: frontend");
    }

    #[test]
    fn tool_input_summary_task_list_with_and_without_filter_normal() {
        let with = ToolInput::TaskList {
            status_filter: Some("pending".into()),
            owner_filter: None,
        };
        assert_eq!(with.summary(), "list tasks (pending)");

        let without = ToolInput::TaskList {
            status_filter: None,
            owner_filter: None,
        };
        assert_eq!(without.summary(), "list tasks");
    }

    // ─── ToolInput::from_value ────────────────────────────────────────────

    #[test]
    fn tool_input_from_value_edit_normal() {
        let v = serde_json::json!({
            "file_path": "src/main.rs",
            "old_string": "fn old",
            "new_string": "fn new",
            "replace_all": true,
        });
        let input = ToolInput::from_value("Edit", v);
        match input {
            ToolInput::Edit {
                file_path,
                replacement,
                ..
            } => {
                assert_eq!(file_path, "src/main.rs");
                assert!(replacement.replace_all());
            }
            other => panic!("expected Edit, got {:?}", other.summary()),
        }
    }

    #[test]
    fn tool_input_from_value_read_optional_fields_normal() {
        let v = serde_json::json!({"file_path": "x", "offset": 10, "limit": 50});
        let input = ToolInput::from_value("Read", v);
        match input {
            ToolInput::Read {
                file_path,
                offset,
                limit,
            } => {
                assert_eq!(file_path, "x");
                assert_eq!(offset, Some(10));
                assert_eq!(limit, Some(50));
            }
            _ => panic!("expected Read"),
        }
    }

    #[test]
    fn tool_input_from_value_task_complete_payload_normal() {
        let v = serde_json::json!({
            "description": "deploy",
            "prompt": "ship it",
            "subagent_type": "ops",
            "run_in_background": true,
            "name": "alice",
            "team_name": "alpha",
            "mode": "plan",
            "isolation": "worktree",
        });
        let input = ToolInput::from_value("Task", v);
        match input {
            ToolInput::Task(ti) => {
                assert_eq!(ti.description, "deploy");
                assert_eq!(ti.prompt, "ship it");
                assert_eq!(ti.subagent_type.as_deref(), Some("ops"));
                assert!(ti.run_in_background);
                assert_eq!(ti.name.as_deref(), Some("alice"));
                assert_eq!(ti.team_name.as_deref(), Some("alpha"));
                assert_eq!(ti.mode.as_deref(), Some("plan"));
                assert_eq!(ti.isolation.as_deref(), Some("worktree"));
            }
            _ => panic!("expected Task"),
        }
    }

    #[test]
    fn tool_input_from_value_task_create_with_blocked_by_array_normal() {
        let v = serde_json::json!({
            "subject": "ship",
            "description": "release v1",
            "blocked_by": ["t1", "t2"],
        });
        let input = ToolInput::from_value("TaskCreate", v);
        match input {
            ToolInput::TaskCreate { blocked_by, .. } => {
                assert_eq!(blocked_by.len(), 2);
                assert!(blocked_by.contains(&"t1".into()));
            }
            _ => panic!("expected TaskCreate"),
        }
    }

    #[test]
    fn tool_input_from_value_send_message_object_payload_robust() {
        // SendMessage's `message` field accepts string OR object — when an
        // object arrives we serialize it to a JSON string for the body.
        let v = serde_json::json!({
            "to": "alice",
            "message": {"kind": "ping", "n": 42},
            "summary": "ping",
        });
        let input = ToolInput::from_value("SendMessage", v);
        match input {
            ToolInput::SendMessage { to, message, .. } => {
                assert_eq!(to, "alice");
                // Object-form should be serialized — must contain both keys.
                assert!(message.contains("ping"), "{message}");
                assert!(message.contains("42"), "{message}");
            }
            _ => panic!("expected SendMessage"),
        }
    }

    #[test]
    fn tool_input_from_value_unknown_kind_falls_through_to_generic_robust() {
        let v = serde_json::json!({"foo": "bar"});
        let input = ToolInput::from_value("not_a_real_tool", v);
        match input {
            ToolInput::Generic { summary } => {
                // Generic stores the original JSON as a string.
                assert!(summary.contains("foo"), "{summary}");
                assert!(summary.contains("bar"), "{summary}");
            }
            _ => panic!("expected Generic"),
        }
    }

    #[test]
    fn tool_input_from_value_handles_missing_fields_robust() {
        // Required fields missing default to empty strings — the executor
        // surfaces an error later, but parsing must not panic.
        let v = serde_json::json!({});
        let input = ToolInput::from_value("Edit", v);
        match input {
            ToolInput::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            } => {
                assert!(file_path.is_empty());
                assert!(old_string.is_empty());
                assert!(new_string.is_empty());
                assert!(!replacement.replace_all());
            }
            _ => panic!("expected Edit"),
        }
    }

    // ─── ToolInput::to_value (round-trip-ish) ─────────────────────────────

    #[test]
    fn tool_input_to_value_bash_with_optional_fields_normal() {
        let i = ToolInput::Bash {
            command: "echo hi".into(),
            timeout: Some(5_000),
            workdir: Some("/tmp".into()),
        };
        let v = i.to_value();
        assert_eq!(v["command"], "echo hi");
        assert_eq!(v["timeout"], 5_000);
        assert_eq!(v["workdir"], "/tmp");
    }

    #[test]
    fn tool_input_to_value_bash_omits_unset_optionals_normal() {
        let i = ToolInput::Bash {
            command: "ls".into(),
            timeout: None,
            workdir: None,
        };
        let v = i.to_value();
        assert_eq!(v["command"], "ls");
        assert!(v.get("timeout").is_none());
        assert!(v.get("workdir").is_none());
    }

    #[test]
    fn tool_input_to_value_grep_omits_unset_optionals_normal() {
        let i = ToolInput::Grep {
            pattern: "todo".into(),
            path: None,
            glob: None,
            output_mode: None,
        };
        let v = i.to_value();
        assert_eq!(v["pattern"], "todo");
        assert!(v.get("path").is_none());
        assert!(v.get("glob").is_none());
        assert!(v.get("output_mode").is_none());
    }

    #[test]
    fn tool_input_to_value_team_create_with_description_normal() {
        let i = ToolInput::TeamCreate {
            team_name: "ops".into(),
            description: Some("operations".into()),
        };
        let v = i.to_value();
        assert_eq!(v["team_name"], "ops");
        assert_eq!(v["description"], "operations");
    }

    #[test]
    fn tool_input_to_value_send_message_omits_summary_when_none_robust() {
        let i = ToolInput::SendMessage {
            to: "alice".into(),
            message: "hi".into(),
            summary: None,
        };
        let v = i.to_value();
        assert_eq!(v["to"], "alice");
        assert!(v.get("summary").is_none());
    }

    #[test]
    fn tool_input_to_value_team_delete_is_empty_object_normal() {
        let v = ToolInput::TeamDelete.to_value();
        assert!(v.is_object());
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn tool_input_to_value_generic_parses_when_valid_json_robust() {
        let i = ToolInput::Generic {
            summary: r#"{"hello":"world"}"#.into(),
        };
        let v = i.to_value();
        assert_eq!(v["hello"], "world");
    }

    #[test]
    fn tool_input_to_value_generic_falls_back_to_input_field_robust() {
        // Non-JSON strings get wrapped in `{"input": "..."}` so the wire
        // always sees an object, never a bare scalar.
        let i = ToolInput::Generic {
            summary: "not even close to json".into(),
        };
        let v = i.to_value();
        assert_eq!(v["input"], "not even close to json");
    }

    // ─── MessagePart helpers ──────────────────────────────────────────────

    #[test]
    fn message_part_text_only_for_compact_boundary_includes_token_count_normal() {
        let p = MessagePart::CompactBoundary { pre_tokens: 12_500 };
        let s = p.text_only();
        assert!(s.contains("12500"), "{s}");
    }

    #[test]
    fn message_part_approx_text_len_text_normal() {
        let p = MessagePart::Text("hello world".into());
        assert_eq!(p.approx_text_len(), 11);
    }

    #[test]
    fn message_part_approx_text_len_compact_boundary_zero_robust() {
        let p = MessagePart::CompactBoundary { pre_tokens: 999 };
        assert_eq!(p.approx_text_len(), 0);
    }

    #[test]
    fn message_part_approx_text_len_task_status_includes_summary_normal() {
        let p = MessagePart::TaskStatus(TaskStatusPart {
            task_id: "t1".into(),
            description: "do it".into(),
            status: TaskLifecycle::Running,
            summary: Some("almost done".into()),
            error: None,
            elapsed_ms: None,
        });
        assert_eq!(p.approx_text_len(), "do it".len() + "almost done".len());
    }

    #[test]
    fn message_part_to_display_string_reasoning_wraps_with_marker_normal() {
        let p = MessagePart::Reasoning("internal monologue".into());
        let s = p.to_display_string();
        assert!(s.starts_with("[Reasoning"), "{s}");
        assert!(s.contains("internal monologue"), "{s}");
    }

    // ─── ChatMessage helpers ──────────────────────────────────────────────

    #[test]
    fn chat_message_user_constructs_text_part_normal() {
        let m = ChatMessage::user("hi".into());
        assert!(m.role_is_user());
        assert!(matches!(&m.parts[0], MessagePart::Text(s) if s == "hi"));
        assert!(m.agent_name.is_none(), "user msgs have no agent name");
    }

    #[test]
    fn chat_message_assistant_constructs_text_part_normal() {
        let m = ChatMessage::assistant("hello".into());
        assert!(!m.role_is_user());
        assert!(matches!(&m.parts[0], MessagePart::Text(s) if s == "hello"));
    }

    #[test]
    fn chat_message_assistant_parts_preserves_input_normal() {
        let parts = vec![
            MessagePart::Reasoning("think".into()),
            MessagePart::Text("speak".into()),
        ];
        let m = ChatMessage::assistant_parts(parts);
        assert_eq!(m.parts.len(), 2);
    }

    #[test]
    fn chat_message_compact_boundary_marks_role_user_with_system_agent_robust() {
        let m = ChatMessage::compact_boundary("summary text", 12_345);
        assert!(m.role_is_user(), "compact boundary uses user role for replay");
        assert!(m.is_compact_boundary());
        assert_eq!(m.agent_name.as_deref(), Some("system"));
    }

    #[test]
    fn chat_message_is_compact_boundary_only_when_part_present_normal() {
        let regular = ChatMessage::user("hi".into());
        assert!(!regular.is_compact_boundary());
    }

    // ─── ModelUsage::cache_hit_pct ────────────────────────────────────────

    #[test]
    fn model_usage_cache_hit_pct_zero_input_safe_normal() {
        let u = ModelUsage::default();
        assert_eq!(u.cache_hit_pct(), 0.0);
    }

    #[test]
    fn model_usage_cache_hit_pct_capped_at_100_robust() {
        // If a buggy provider reports cache_read > input we still cap at 100%.
        let u = ModelUsage {
            input_tokens: 10,
            cache_read_tokens: 50,
            ..Default::default()
        };
        assert_eq!(u.cache_hit_pct(), 100.0);
    }

    #[test]
    fn model_usage_cache_hit_pct_normal_value_normal() {
        let u = ModelUsage {
            input_tokens: 100,
            cache_read_tokens: 25,
            ..Default::default()
        };
        assert_eq!(u.cache_hit_pct(), 25.0);
    }

    #[test]
    fn model_usage_total_context_tokens_sums_all_normal() {
        let u = ModelUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 10,
            cache_write_tokens: 20,
            cost_usd: None,
        };
        assert_eq!(u.total_context_tokens(), 330);
    }

    #[test]
    fn model_usage_add_delta_accumulates_normal() {
        let mut u = ModelUsage::default();
        u.add_delta(10, 20, 5, 3);
        u.add_delta(1, 2, 0, 0);
        assert_eq!(u.input_tokens, 11);
        assert_eq!(u.output_tokens, 22);
        assert_eq!(u.cache_read_tokens, 5);
        assert_eq!(u.cache_write_tokens, 3);
    }

    // ─── parse_unified_diff / parse_hunk_header / parse_hunk_start ─────────

    #[test]
    fn parse_hunk_start_strips_sign_and_count_normal() {
        assert_eq!(parse_hunk_start("-12,5"), 12);
        assert_eq!(parse_hunk_start("+200,1"), 200);
        assert_eq!(parse_hunk_start("17"), 17);
    }

    #[test]
    fn parse_hunk_start_returns_one_for_unparseable_robust() {
        assert_eq!(parse_hunk_start("notanumber"), 1);
        assert_eq!(parse_hunk_start(""), 1);
    }

    #[test]
    fn parse_hunk_header_extracts_old_new_starts_normal() {
        let (old, new, _) = parse_hunk_header("@@ -1,5 +10,7 @@ fn foo");
        assert_eq!(old, 1);
        assert_eq!(new, 10);
    }

    #[test]
    fn parse_unified_diff_counts_additions_deletions_normal() {
        let view = parse_unified_diff(
            "x.rs",
            "@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n",
        );
        assert_eq!(view.additions, 1);
        assert_eq!(view.deletions, 1);
        assert_eq!(view.file_path, "x.rs");
        assert_eq!(view.hunks.len(), 1);
    }

    #[test]
    fn parse_unified_diff_handles_multiple_hunks_normal() {
        let view = parse_unified_diff(
            "x.rs",
            "@@ -1,1 +1,1 @@\n-a\n+b\n@@ -10,1 +10,1 @@\n-c\n+d\n",
        );
        assert_eq!(view.hunks.len(), 2);
        assert_eq!(view.additions, 2);
        assert_eq!(view.deletions, 2);
    }

    #[test]
    fn parse_unified_diff_lines_before_hunk_skipped_robust() {
        // Lines before the first @@ have no hunk to attach to — they're
        // dropped silently. A real "missing header" produces an empty
        // hunk list, not a panic.
        let view = parse_unified_diff("x.rs", "stray text\n");
        assert!(view.hunks.is_empty());
        assert_eq!(view.additions, 0);
    }

    // ─── truncate_lines ──────────────────────────────────────────────────

    #[test]
    fn truncate_lines_below_max_returns_unchanged_normal() {
        let s = "a\nb\nc\n";
        // Note: the implementation's `lines.iter().take(max).join("\n")`
        // strips trailing newline since `lines()` doesn't include it.
        let out = truncate_lines(s, 10);
        assert_eq!(out, "a\nb\nc");
    }

    #[test]
    fn truncate_lines_above_max_appends_more_marker_robust() {
        let s = "a\nb\nc\nd\ne\n";
        let out = truncate_lines(s, 2);
        assert!(out.contains("a"));
        assert!(out.contains("b"));
        assert!(!out.contains("c"));
        assert!(out.contains("3 more"), "{out}");
    }

    #[test]
    fn truncate_lines_empty_input_robust() {
        assert_eq!(truncate_lines("", 5), "");
    }
}
