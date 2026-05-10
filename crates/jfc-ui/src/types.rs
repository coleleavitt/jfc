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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => f.write_str("user"),
            Self::Assistant => f.write_str("assistant"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum MessagePart {
    Text(String),
    Reasoning(String),
    Tool(ToolCall),
    TaskStatus(TaskStatusPart),
    CompactBoundary {
        pre_tokens: usize,
    },
    /// A parallel-advisor reply (see `crate::advisor`). Rendered with a
    /// distinct visual style (italic + secondary text color + "ADVISOR:"
    /// prefix) so the user can tell at a glance that this came from the
    /// out-of-band advisor and not the main agent. Doesn't participate in
    /// the model's normal turn accounting — it's a UI-only side effect of
    /// `/advisor <query>`.
    Advisor(String),
}

impl MessagePart {
    pub fn approx_text_len(&self) -> usize {
        match self {
            Self::Text(s) | Self::Reasoning(s) | Self::Advisor(s) => s.len(),
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
            Self::Advisor(s) => format!("[Advisor: {s}]"),
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
            Self::Advisor(s) => format!("[Advisor: {s}]"),
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

/// One step in the per-session undo stack. Captured by the tool
/// dispatcher *before* an Edit / Write / MultiEdit / ApplyPatch fires
/// so `/undo` can restore the pre-mutation state. `previous_content =
/// None` means the file didn't exist before — undo deletes it.
#[derive(Debug, Clone)]
pub struct ToolUndoEntry {
    pub file_path: String,
    pub previous_content: Option<String>,
    pub op_label: String,
}

/// Tri-state display mode for a tool block. Replaces three independent
/// bools (`is_collapsed`, `expanded`, `pinned`) so mutually-exclusive
/// states like "collapsed teaser" + "expanded with raised cap" are
/// unrepresentable-by-construction instead of relying on unchecked
/// invariants every renderer + toggle had to obey by hand. `pinned`
/// is associated only with the variants where it makes sense
/// (Default, Expanded) — the Collapsed teaser is never pinned because
/// pinning would make it expand on the next render anyway, so a
/// `Collapsed { pinned: true }` would be incoherent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolDisplayState {
    /// Default rendering: full content, capped at 80 lines (the
    /// preview cap). The user has not asked for either a one-line
    /// teaser or a raised cap. `pinned=true` resists auto-collapse
    /// (e.g. on huge LargeText results) and surfaces the 📌 glyph.
    Default { pinned: bool },
    /// One-line teaser only ("▶ N reads · click to expand").
    /// Set on huge outputs (LargeText that exceed COLLAPSE_LINES /
    /// COLLAPSE_BYTES) that would otherwise dominate the chat, and
    /// on grouped tool runs the user has not opted into.
    Collapsed,
    /// Full content with the cap raised from 80 to 500. Entered via
    /// `Ctrl+O` / `o` / click on the title. `pinned=true` means the
    /// user double-clicked to lock it expanded — only another
    /// double-click can flip it off, so the long Read they wanted to
    /// keep visible while scrolling doesn't silently re-collapse.
    Expanded { pinned: bool },
}

impl ToolDisplayState {
    /// Default rendering, no pin. The construction default for new
    /// tool calls.
    pub const DEFAULT: Self = Self::Default { pinned: false };

    pub fn is_collapsed(&self) -> bool {
        matches!(self, Self::Collapsed)
    }

    pub fn is_expanded(&self) -> bool {
        matches!(self, Self::Expanded { .. })
    }

    pub fn is_pinned(&self) -> bool {
        matches!(
            self,
            Self::Default { pinned: true } | Self::Expanded { pinned: true }
        )
    }

    /// Single source of truth for the renderer's per-row line cap.
    /// Expanded variants raise the cap to 500; everything else uses
    /// the 80-line preview cap. Note: per-output-kind caps in
    /// message_view (e.g. grep at 200/1000) still scale around
    /// `is_expanded()` — the leaf producers keep their own kind-
    /// specific multipliers — but for the generic text/file paths
    /// this is the canonical decision.
    pub fn cap_lines(&self) -> usize {
        if self.is_expanded() { 500 } else { 80 }
    }

    /// Toggle expanded ↔ default behind `o` / `Ctrl+O` /
    /// click-on-title. A pinned-expanded tool collapses back to a
    /// pinned-default; a pinned-default expands to pinned-expanded.
    /// Collapsed (huge LargeText teaser) is left alone — the caller
    /// uses `toggle_collapsed` for that arm so the two-level expand
    /// (teaser ⇄ body, body ⇄ raised-cap) stays distinct.
    pub fn toggle_expanded(&mut self) {
        *self = match *self {
            Self::Default { pinned } => Self::Expanded { pinned },
            Self::Expanded { pinned } => Self::Default { pinned },
            Self::Collapsed => Self::Default { pinned: false },
        };
    }

    /// Toggle the pin glyph on Default + Expanded. Pinning forces
    /// the Expanded state (the renderer needs a body to put the pin
    /// next to); unpinning leaves the cap state alone. Collapsed
    /// can't be pinned by construction, so a pin on a Collapsed
    /// teaser promotes it to a pinned-Expanded body.
    pub fn toggle_pinned(&mut self) {
        *self = match *self {
            Self::Default { pinned } => {
                if pinned {
                    Self::Default { pinned: false }
                } else {
                    Self::Expanded { pinned: true }
                }
            }
            Self::Expanded { pinned } => Self::Expanded { pinned: !pinned },
            Self::Collapsed => Self::Expanded { pinned: true },
        };
    }

    /// Force the teaser state (used when a huge LargeText result
    /// arrives — the dispatcher collapses by default so the chat
    /// isn't drowned).
    pub fn collapse(&mut self) {
        *self = Self::Collapsed;
    }

    /// Toggle between teaser (Collapsed) and body
    /// (Default { pinned: false }). Used by `o` on huge LargeText
    /// outputs where the two-level expand model pivots around
    /// teaser ⇄ body rather than body ⇄ raised-cap.
    pub fn toggle_collapsed(&mut self) {
        *self = match *self {
            Self::Collapsed => Self::Default { pinned: false },
            // From a body state, the user wanted to fold it back to
            // a teaser. Pin status is dropped intentionally — a
            // teaser is never pinned (see enum doc comment).
            Self::Default { .. } | Self::Expanded { .. } => Self::Collapsed,
        };
    }
}

impl Default for ToolDisplayState {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Debug)]
pub struct ToolCall {
    pub id: crate::ids::ToolId,
    pub kind: ToolKind,
    /// Lifecycle status for this tool. Direct assignment is still
    /// permitted because in-flight migration of the codebase requires
    /// it, but new code SHOULD use the [`Self::mark_running`] /
    /// [`Self::mark_completed`] / [`Self::mark_failed`] /
    /// [`Self::mark_cancelled`] transition methods, which validate the
    /// before-state and refuse invalid jumps (e.g. Failed → Running).
    /// The methods centralize the "what state did we come from?"
    /// invariant so no future caller can silently resurrect a
    /// terminal tool.
    pub status: ExecutionStatus,
    pub input: ToolInput,
    pub output: ToolOutput,
    /// Tri-state display mode (collapsed teaser / default body /
    /// expanded body), with an orthogonal pin flag baked into the
    /// states where it's meaningful. Replaces three separate bools
    /// (`is_collapsed`, `expanded`, `pinned`) so the renderer can't
    /// be handed a contradictory pair like "collapsed AND expanded".
    /// See [`ToolDisplayState`] for the variants and their helpers.
    pub display: ToolDisplayState,
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
}

impl ToolCall {
    /// Construct a fresh ToolCall in the `Pending` state. Use this
    /// from the stream layer where a tool is just leaving the model
    /// and hasn't been dispatched yet — guarantees the start state is
    /// always a sane `Pending`, never accidentally `Running` or
    /// `Completed`.
    pub fn new_pending(id: crate::ids::ToolId, kind: ToolKind, input: ToolInput) -> Self {
        Self {
            id,
            kind,
            status: ExecutionStatus::Pending,
            input,
            output: ToolOutput::Empty,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: Some(std::time::Instant::now()),
        }
    }

    /// Construct a ToolCall that's already in the `Failed` terminal
    /// state — used by the stream layer when malformed provider input
    /// (bad JSON, schema mismatch) means we never even get to dispatch
    /// the tool. The output carries the diagnostic that will be
    /// shipped back to the model as the tool_result.
    pub fn new_failed(
        id: crate::ids::ToolId,
        kind: ToolKind,
        input: ToolInput,
        output: ToolOutput,
    ) -> Self {
        Self {
            id,
            kind,
            status: ExecutionStatus::Failed,
            input,
            output,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    /// Pending → Running. Returns Err if the tool is already in a
    /// terminal state (Completed/Failed/Cancelled). Idempotent on
    /// Running.
    pub fn mark_running(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Running)
    }

    /// {Pending|Running} → Completed. Returns Err on terminal state.
    /// Idempotent on Completed.
    pub fn mark_completed(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Completed)
    }

    /// {Pending|Running} → Failed. Returns Err if the tool is already
    /// in a different terminal state (Completed/Cancelled).
    pub fn mark_failed(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Failed)
    }

    /// {Pending|Running} → Cancelled. Returns Err on a different
    /// terminal state. Used when the user denies a tool or moves on
    /// before it dispatches.
    pub fn mark_cancelled(&mut self) -> Result<(), InvalidToolTransition> {
        self.try_transition_to(ExecutionStatus::Cancelled)
    }

    fn try_transition_to(&mut self, target: ExecutionStatus) -> Result<(), InvalidToolTransition> {
        if !self.status.allows_transition_to(target) {
            return Err(InvalidToolTransition {
                from: self.status,
                to: target,
            });
        }
        self.status = target;
        Ok(())
    }
}

/// Canonical lifecycle for both Tool and Task execution. The two used
/// to be separate (`ToolStatus` had four variants — Pending/Running/
/// Complete/Failed; `TaskLifecycle` had six — Pending/Running/Idle/
/// Completed/Failed/Cancelled) and required hand-coded mapping in
/// both directions. They encoded the same concept with subtly
/// different variants, so we unify on this single enum and keep the
/// old type names as aliases for documentation purposes (see
/// [`ToolStatus`] and [`TaskLifecycle`] below).
///
/// Variant choices:
/// - `Idle` was Task-only (a teammate finished its turn but is still
///   waiting for new input). Tools never enter `Idle`; helpers like
///   [`Self::is_alive`] still treat it correctly there.
/// - `Completed` is canonical (more standard English than `Complete`,
///   which was the Tool-only legacy name). Wire-format readers in
///   `session.rs` accept both spellings for backward compat — see the
///   test `execution_status_serde_back_compat_normal`.
/// - `Cancelled` was Task-only; surfacing it on tools too lets the
///   classifier/permission layer mark a denied tool as Cancelled
///   instead of Failed where appropriate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExecutionStatus {
    Pending,
    Running,
    /// Started but quiescent — formerly Task-only. Distinct from
    /// `Running` so the task panel can stop its "Receiving output…"
    /// spinner without having to mark the task terminal (the agent
    /// could resume on the next SendMessage). Tools never enter this
    /// state in practice; if one does, treat it as a programmer error
    /// and log via tracing rather than panic.
    Idle,
    Completed,
    Failed,
    Cancelled,
}

/// Documentation alias — used to be a separate enum. Same wire format
/// as before for the on-disk session journal (see `session.rs`).
pub type TaskLifecycle = ExecutionStatus;
/// Documentation alias — used to be a separate enum. The legacy
/// `ToolStatus::Completed` variant is now [`ExecutionStatus::Completed`].
pub type ToolStatus = ExecutionStatus;

impl ExecutionStatus {
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

    /// Counts as "alive" for fan-out / agent-count purposes. Running
    /// and Idle teammates both still belong on the agent fan even
    /// though only Running ones are actively producing output.
    pub fn is_alive(self) -> bool {
        matches!(self, Self::Pending | Self::Running | Self::Idle)
    }

    /// Returns true if a transition from `self` to `target` is
    /// well-formed. Used by [`ToolCall`]'s `mark_*` helpers to refuse
    /// invalid jumps like Failed→Running. Idempotent same-state
    /// transitions (e.g. Running→Running) are allowed because the
    /// stream layer occasionally re-asserts state on retry.
    pub fn allows_transition_to(self, target: Self) -> bool {
        if self == target {
            return true;
        }
        // Terminal states never transition out.
        if self.is_terminal() {
            return false;
        }
        // From any non-terminal state, any other state is reachable —
        // the strict ordering (Pending → Running → terminal) is too
        // restrictive for real provider streams, which sometimes
        // collapse Pending and skip directly to Completed when a tool
        // was approved + executed faster than the UI can poll.
        true
    }
}

#[derive(Clone, Debug)]
pub struct TaskStatusPart {
    pub task_id: crate::ids::TaskId,
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
    TaskGet,
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
    /// Query the code graph using pipe-based DSL.
    PostBounty,
    RunBounty,
    MarketStatus,
    GraphQuery,
    /// Run coverage collection and annotate the graph.
    RunCoverage,
    /// Edit code by symbol handle (semantic editing).
    SymbolEdit,
    /// v132 parity: invoked by the model to surface a finalized plan.
    ExitPlanMode,
    MultiEdit,
    AskUserQuestion,
    WebFetch,
    WebSearch,
    /// MCP-advertised tool, full `mcp__server__tool` name.
    Mcp(String),
    CronCreate,
    CronList,
    CronDelete,
    ScheduleWakeup,
    Monitor,
    /// Query LSP for hover/definition/references.
    Lsp,
    /// Send a desktop notification.
    PushNotification,
    /// Hit a webhook URL pre-registered in triggers.toml.
    RemoteTrigger,
    /// Model-callable: enter plan mode.
    EnterPlanMode,
    EnterWorktree,
    ExitWorktree,
    NotebookRead,
    NotebookEdit,
    /// Deliberately-named generic tool wrapping a string label —
    /// used by sample harnesses and code that constructs a ToolKind
    /// for a tool whose semantics we know but don't represent as a
    /// first-class variant. Kept distinct from `UnknownTool` so the
    /// "we got a name we don't recognize" path is grep-able and
    /// can deny-by-default in permission checks.
    Generic(String),
    /// A model-advertised tool name that did not match any known
    /// variant in [`ToolKind::from_name`]. Distinct from `Generic`
    /// (which is for deliberately-named tools we just don't represent
    /// as first-class variants) so that adding a new `ToolKind::Foo`
    /// variant is a compile error at every match site instead of a
    /// silent dispatch to `Generic("Foo")` until someone notices.
    /// Always denied by permission checks — we won't dispatch a tool
    /// we don't understand.
    UnknownTool {
        advertised_name: String,
    },
}

/// Returned by [`ToolCall::mark_running`] and friends when the caller
/// asked for a state transition that the lifecycle enum forbids
/// (e.g. Failed → Running, or any movement out of a terminal state).
/// The Display impl produces a one-line message suitable for logging.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid ToolCall status transition: {from:?} → {to:?}")]
pub struct InvalidToolTransition {
    pub from: ExecutionStatus,
    pub to: ExecutionStatus,
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
    TaskGet {
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
    GraphQuery {
        query: String,
        max_tokens: Option<usize>,
        /// Whether to append a `--- handles ---` footer of structured
        /// `kind:qualified_name` handles for chaining. `None` defaults to
        /// `true` at dispatch time. `#[serde(default)]` so older
        /// serialized sessions (which never set this field) still round-trip.
        #[serde(default)]
        include_handles: Option<bool>,
    },
    PostBounty {
        description: String,
        budget: u64,
        acceptance_criteria: String,
        #[serde(default)]
        max_solvers: Option<u8>,
        /// When true, `execute_tool` runs the full Post→Bid→Solve→
        /// Validate→Settle cycle synchronously, spawning real solver
        /// + validator subagents that hit the LLM. Default false:
        /// the bounty is queued as Open and the user / model drives
        /// progress manually via `market_status`. Set true only
        /// when you want a complete competitive resolve in one
        /// tool call (expensive).
        #[serde(default)]
        auto_dispatch: bool,
    },
    MarketStatus {
        #[serde(default)]
        bounty_id: Option<String>,
    },
    /// Drive an already-posted Open bounty through the full
    /// Solve→Validate→Settle cycle. The split from PostBounty
    /// exists so the model can reason about market state in
    /// stages (post first, decide whether to dispatch later)
    /// without paying the dispatch cost on every post call.
    RunBounty {
        bounty_id: String,
        #[serde(default)]
        max_solvers: Option<u8>,
    },
    RunCoverage {
        /// Optional path to an existing lcov.info file. If omitted, the tool
        /// runs `cargo llvm-cov --lcov` to generate one.
        #[serde(default)]
        lcov_path: Option<String>,
        /// Whether to include a list of untested functions in the output.
        #[serde(default = "default_true")]
        include_untested_list: bool,
    },
    SymbolEdit {
        handle: String,
        new_content: String,
        #[serde(default)]
        validate: bool,
        /// When true, the cascade plan produced by `validate=true` is
        /// also auto-queued into the project's TaskStore — one entry
        /// per file, tagged with metadata.kind="cascade". The model
        /// can then drive the actual call-site updates by spawning
        /// Task tool sub-agents against the queued items, and the
        /// user sees them in the standard task panel + `/cascade`
        /// slash command. Requires `validate=true` to have any
        /// effect — without validation we don't compute the cascade.
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
        multi_select: bool,
    },
    WebFetch {
        url: String,
        prompt: Option<String>,
    },
    WebSearch {
        query: String,
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
        edit_mode: Option<String>,
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
    /// Matches Claude Code v2.1.131's per-tool default cap (`yIK = 5e4` in
    /// the deob bundle). Was 30KB; 50KB lets a Read on a typical source
    /// file land entirely in the head slice without triggering the
    /// truncation marker, while still keeping the per-result wire size
    /// bounded so a single tool call can't blow a 1M-token request.
    pub const APPROX_LEN_CAP: usize = 50_000;

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

/// Variants of the "messages alternate user/assistant" invariant. Each
/// carries enough context (an index, a role, optionally a `ToolId`) for
/// a tracing log to point at the offending entry.
///
/// Surfacing the variant is the entire point — the violation is the
/// signal. Auto-fixing in flight masks other bugs, so the validator
/// never mutates the slice. See `validate_turn_invariants`.
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum TurnInvariantError {
    /// Two adjacent `Role::User` messages. Triggered by a queue drain
    /// that pushed a new user prompt without first consuming the prior
    /// assistant turn (or by a corrupt session.json).
    #[error("two consecutive user messages at index {at_index}")]
    ConsecutiveUser { at_index: usize },
    /// Two adjacent `Role::Assistant` messages — the structural shape
    /// of the plan-continuation phantom-assistant bug.
    #[error("two consecutive assistant messages at index {at_index}")]
    ConsecutiveAssistant { at_index: usize },
    /// A non-streaming message has zero textual content and no tool
    /// activity. Empty assistant placeholders mid-stream are allowed
    /// by skipping the LAST assistant slot when validating (see
    /// `validate_turn_invariants_inner`'s `allow_streaming_tail`
    /// flag).
    #[error("empty {role} message at index {at_index}")]
    EmptyMessage { at_index: usize, role: Role },
    /// A `MessagePart::Tool` that's still `Pending`/`Running` after a
    /// later turn was added — the tool_use is structurally orphaned
    /// because the model has moved on without a result.
    #[error(
        "orphan tool_use {tool_id} at index {at_index} (no matching tool_result before next turn)"
    )]
    OrphanToolUse {
        tool_id: crate::ids::ToolId,
        at_index: usize,
    },
    /// A `MessagePart::Tool` carrying a resolved (`Complete`/`Failed`)
    /// status on a `Role::User` message. Tool calls live on assistant
    /// messages; finding one user-side means deserialization or the
    /// stream pipeline routed something to the wrong message.
    #[error("orphan tool_result {tool_id} at index {at_index} (tool part on a user message)")]
    OrphanToolResult {
        tool_id: crate::ids::ToolId,
        at_index: usize,
    },
    /// The first message in the slice has `Role::Assistant`. Outside
    /// of system-injected boundary markers (`compact_boundary`, which
    /// uses `Role::User`), every legitimate session opens with a user
    /// prompt.
    #[error("leading assistant message at index 0 (role={role})")]
    LeadingAssistant { role: Role },
}

/// Walk a message slice and report the first `TurnInvariantError` that
/// breaks the user/assistant alternation invariant.
///
/// ## What's checked
/// - First message must be `Role::User` (system-injected boundaries
///   always materialize as user-role; see `ChatMessage::compact_boundary`).
/// - No two adjacent messages share a role.
/// - No message is fully empty (no text, no tool, no boundary, no
///   advisor side-channel) — except an *assistant streaming
///   placeholder* (the last message in the slice when streaming,
///   carrying just an empty `MessagePart::Text("")`).
/// - Every `MessagePart::Tool` whose status is `Pending`/`Running`
///   must sit on the most-recent assistant message, otherwise it's an
///   orphaned tool_use the model has already moved past.
/// - No `MessagePart::Tool` may sit on a `Role::User` message — tool
///   calls always belong to assistant turns.
///
/// ## What's NOT checked (intentional)
/// - Compact-boundary messages: `compact_boundary` produces a User
///   message that may be followed by another User reply describing
///   the resumed task. The single permitted exception is "first
///   message after a CompactBoundary may be User even if the prior
///   was also User."
/// - Tool ID uniqueness: provider IDs occasionally collide on retry;
///   that's a separate pathology owned by `tools::dedupe`.
fn default_true() -> bool {
    true
}

pub fn validate_turn_invariants(messages: &[ChatMessage]) -> Result<(), TurnInvariantError> {
    validate_turn_invariants_inner(messages, /* allow_streaming_tail = */ false)
}

/// Inner form that allows the trailing message to be a (possibly empty)
/// assistant streaming placeholder. Used when we're about to push a
/// fresh assistant slot in `continue_agentic_loop`/`drain_queued_prompts`
/// and want to confirm the *prior* state was sound — the placeholder
/// itself is a known-empty stub the next stream tick will fill.
pub(crate) fn validate_turn_invariants_inner(
    messages: &[ChatMessage],
    allow_streaming_tail: bool,
) -> Result<(), TurnInvariantError> {
    if messages.is_empty() {
        return Ok(());
    }

    // 1) Leading-assistant check. The plan-continuation bug's UI
    // symptom was a session that opened "blank assistant → real user"
    // because of a phantom slot — catch that shape immediately.
    let first = &messages[0];
    if first.role == Role::Assistant && !first.is_compact_boundary() {
        return Err(TurnInvariantError::LeadingAssistant { role: first.role });
    }

    // 2) Pairwise alternation + emptiness checks.
    let last_idx = messages.len() - 1;
    for (i, m) in messages.iter().enumerate() {
        // Alternation against the previous message.
        if i > 0 {
            let prev = &messages[i - 1];
            if prev.role == m.role {
                // CompactBoundary is a system-injected user-role message
                // that may legitimately be followed by another user
                // message describing the resumed task. Skip the
                // alternation check across that exact seam.
                let either_is_boundary = prev.is_compact_boundary() || m.is_compact_boundary();
                if !either_is_boundary {
                    return Err(match m.role {
                        Role::User => TurnInvariantError::ConsecutiveUser { at_index: i },
                        Role::Assistant => TurnInvariantError::ConsecutiveAssistant { at_index: i },
                    });
                }
            }
        }

        // Emptiness check. A message is "empty" if it carries no
        // text/reasoning, no tool, no advisor reply, and no compact
        // boundary marker. The streaming-placeholder exception lets
        // us stage an empty assistant slot just before stream_response
        // starts pumping tokens into it.
        let has_content = m.parts.iter().any(|p| match p {
            MessagePart::Text(s) | MessagePart::Reasoning(s) | MessagePart::Advisor(s) => {
                !s.is_empty()
            }
            MessagePart::Tool(_)
            | MessagePart::TaskStatus(_)
            | MessagePart::CompactBoundary { .. } => true,
        });
        let is_streaming_tail = allow_streaming_tail && i == last_idx && m.role == Role::Assistant;
        if !has_content && !is_streaming_tail {
            return Err(TurnInvariantError::EmptyMessage {
                at_index: i,
                role: m.role,
            });
        }

        // Tool-routing check: tool parts only belong on assistant
        // messages. A `MessagePart::Tool` on a User-role message is
        // structurally a misrouted tool_result.
        if m.role == Role::User {
            for part in &m.parts {
                if let MessagePart::Tool(tc) = part {
                    return Err(TurnInvariantError::OrphanToolResult {
                        tool_id: tc.id.clone(),
                        at_index: i,
                    });
                }
            }
        }
    }

    // 3) Orphan tool_use detection. A Pending/Running tool on any
    // assistant message that's NOT the last one is orphaned — the
    // turn rolled forward without a tool_result. The tail assistant
    // is allowed to have in-flight tools (it's still streaming /
    // awaiting approval).
    for (i, m) in messages.iter().enumerate() {
        if m.role != Role::Assistant {
            continue;
        }
        let is_tail = i == last_idx;
        if is_tail {
            continue;
        }
        for part in &m.parts {
            if let MessagePart::Tool(tc) = part {
                if matches!(tc.status, ToolStatus::Pending | ToolStatus::Running) {
                    return Err(TurnInvariantError::OrphanToolUse {
                        tool_id: tc.id.clone(),
                        at_index: i,
                    });
                }
            }
        }
    }

    Ok(())
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
            "taskget" => Self::TaskGet,
            "task" => Self::Task,
            "skill" => Self::Skill,
            "memorycreate" => Self::MemoryCreate,
            "memorydelete" => Self::MemoryDelete,
            "teamcreate" => Self::TeamCreate,
            "teamdelete" => Self::TeamDelete,
            "sendmessage" => Self::SendMessage,
            "teammembermode" => Self::TeamMemberMode,
            "graphquery" | "graph_query" => Self::GraphQuery,
            "runcoverage" | "run_coverage" => Self::RunCoverage,
            "symboledit" | "symbol_edit" => Self::SymbolEdit,
            "exitplanmode" => Self::ExitPlanMode,
            "multiedit" => Self::MultiEdit,
            "askuserquestion" => Self::AskUserQuestion,
            "webfetch" | "web_fetch" => Self::WebFetch,
            "websearch" | "web_search" => Self::WebSearch,
            "postbounty" | "post_bounty" => Self::PostBounty,
            "marketstatus" | "market_status" => Self::MarketStatus,
            "runbounty" | "run_bounty" => Self::RunBounty,
            "croncreate" | "cron_create" => Self::CronCreate,
            "cronlist" | "cron_list" => Self::CronList,
            "crondelete" | "cron_delete" => Self::CronDelete,
            "schedulewakeup" | "schedule_wakeup" => Self::ScheduleWakeup,
            "monitor" => Self::Monitor,
            "lsp" => Self::Lsp,
            "pushnotification" | "push_notification" => Self::PushNotification,
            "remotetrigger" | "remote_trigger" => Self::RemoteTrigger,
            "enterplanmode" | "enter_plan_mode" => Self::EnterPlanMode,
            "enterworktree" | "enter_worktree" => Self::EnterWorktree,
            "exitworktree" | "exit_worktree" => Self::ExitWorktree,
            "notebookread" | "notebook_read" => Self::NotebookRead,
            "notebookedit" | "notebook_edit" => Self::NotebookEdit,
            // MCP-namespaced tools route to the Mcp variant. Goes last
            // so it doesn't shadow specific matches.
            _ if name.starts_with("mcp__") => Self::Mcp(name.to_owned()),
            // Anything else is a model-advertised tool name we don't
            // recognize. Route to `UnknownTool` rather than the old
            // silent `Generic` fallback — the goal is for adding a new
            // `ToolKind::Foo` variant to be a compile error at every
            // match site (so dispatch wires up correctly) instead of a
            // silent dispatch to `Generic("Foo")` until someone notices.
            // Permission checks (see `auto_approves`) deny UnknownTool
            // in every mode, so a typo or hallucinated tool name fails
            // loudly instead of getting routed to "not yet implemented".
            _ => Self::UnknownTool {
                advertised_name: name.to_owned(),
            },
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
            Self::TaskGet => "TaskGet",
            Self::Task => "Task",
            Self::Skill => "Skill",
            Self::MemoryCreate => "MemoryCreate",
            Self::MemoryDelete => "MemoryDelete",
            Self::TeamCreate => "TeamCreate",
            Self::TeamDelete => "TeamDelete",
            Self::SendMessage => "SendMessage",
            Self::TeamMemberMode => "TeamMemberMode",
            Self::GraphQuery => "GraphQuery",
            Self::RunCoverage => "RunCoverage",
            Self::SymbolEdit => "SymbolEdit",
            Self::ExitPlanMode => "ExitPlanMode",
            Self::MultiEdit => "MultiEdit",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::WebFetch => "WebFetch",
            Self::WebSearch => "WebSearch",
            Self::PostBounty => "PostBounty",
            Self::RunBounty => "RunBounty",
            Self::MarketStatus => "MarketStatus",
            Self::Mcp(name) => name.as_str(),
            Self::CronCreate => "CronCreate",
            Self::CronList => "CronList",
            Self::CronDelete => "CronDelete",
            Self::ScheduleWakeup => "ScheduleWakeup",
            Self::Monitor => "Monitor",
            Self::Lsp => "LSP",
            Self::PushNotification => "PushNotification",
            Self::RemoteTrigger => "RemoteTrigger",
            Self::EnterPlanMode => "EnterPlanMode",
            Self::EnterWorktree => "EnterWorktree",
            Self::ExitWorktree => "ExitWorktree",
            Self::NotebookRead => "NotebookRead",
            Self::NotebookEdit => "NotebookEdit",
            Self::Generic(name) => name.as_str(),
            // The advertised name is what the model sent us — surface it
            // verbatim so logs and the transcript identify which name we
            // refused to dispatch.
            Self::UnknownTool { advertised_name } => advertised_name.as_str(),
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
            Self::TaskGet => "TaskGet",
            Self::Task => "Task",
            Self::Skill => "Skill",
            Self::MemoryCreate => "MemoryCreate",
            Self::MemoryDelete => "MemoryDelete",
            Self::TeamCreate => "TeamCreate",
            Self::TeamDelete => "TeamDelete",
            Self::SendMessage => "SendMessage",
            Self::TeamMemberMode => "TeamMemberMode",
            Self::GraphQuery => "graph_query",
            Self::RunCoverage => "run_coverage",
            Self::SymbolEdit => "symbol_edit",
            Self::ExitPlanMode => "ExitPlanMode",
            Self::MultiEdit => "MultiEdit",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::WebFetch => "WebFetch",
            Self::WebSearch => "WebSearch",
            Self::PostBounty => "post_bounty",
            Self::RunBounty => "run_bounty",
            Self::MarketStatus => "market_status",
            Self::Mcp(name) => name.as_str(),
            Self::CronCreate => "CronCreate",
            Self::CronList => "CronList",
            Self::CronDelete => "CronDelete",
            Self::ScheduleWakeup => "ScheduleWakeup",
            Self::Monitor => "Monitor",
            Self::Lsp => "LSP",
            Self::PushNotification => "PushNotification",
            Self::RemoteTrigger => "RemoteTrigger",
            Self::EnterPlanMode => "EnterPlanMode",
            Self::EnterWorktree => "EnterWorktree",
            Self::ExitWorktree => "ExitWorktree",
            Self::NotebookRead => "NotebookRead",
            Self::NotebookEdit => "NotebookEdit",
            Self::Generic(name) => name.as_str(),
            // Round-trip the advertised name on the wire so a session
            // resumed from disk re-parses to the same UnknownTool kind.
            Self::UnknownTool { advertised_name } => advertised_name.as_str(),
        }
    }
}

/// Errors returned by [`ToolInput::from_value`] when the provider-supplied
/// JSON for a tool call doesn't match the tool's expected shape. The
/// `Display` impl produces a one-line message suitable for shipping back
/// to the model in a `tool_result` block — see `stream.rs`'s ToolDone
/// handler for the wiring.
///
/// This is the "validate at the boundary" half of the parsing strategy:
/// we accept untrusted JSON exactly once, here, and either build a typed
/// `ToolInput` or refuse with a precise reason. Downstream code never
/// has to defensively re-check fields.
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum ToolInputError {
    #[error("tool `{tool}`: missing required field `{field}`")]
    MissingField { tool: String, field: &'static str },
    #[error("tool `{tool}`: field `{field}` has wrong type (expected {expected}, got {got})")]
    WrongType {
        tool: String,
        field: &'static str,
        expected: &'static str,
        got: &'static str,
    },
    #[error("tool `{tool}`: invalid input — {reason}")]
    InvalidShape { tool: String, reason: String },
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
            Self::TaskGet { task_id } => format!("get: {task_id}"),
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
            Self::GraphQuery { query, .. } => query.clone(),
            Self::RunCoverage { lcov_path, .. } => {
                format!("coverage({})", lcov_path.as_deref().unwrap_or("auto"))
            }
            Self::SymbolEdit { handle, .. } => format!("edit: {handle}"),
            Self::PostBounty {
                description,
                budget,
                ..
            } => {
                format!(
                    "bounty ({budget} tok): {}",
                    description.chars().take(60).collect::<String>()
                )
            }
            Self::MarketStatus { bounty_id } => match bounty_id {
                Some(id) => format!("market status: {id}"),
                None => "market status".into(),
            },
            Self::RunBounty { bounty_id, .. } => format!("run bounty: {bounty_id}"),
            Self::ExitPlanMode { plan } => {
                let head: String = plan.lines().next().unwrap_or("").chars().take(60).collect();
                format!("exit plan mode: {head}")
            }
            Self::MultiEdit { file_path, edits } => {
                let n = edits.as_array().map(|a| a.len()).unwrap_or(0);
                format!("{file_path} ({n} edit{})", if n == 1 { "" } else { "s" })
            }
            Self::AskUserQuestion { question, .. } => {
                format!("ask: {}", question.chars().take(60).collect::<String>())
            }
            Self::WebFetch { url, .. } => format!("fetch: {url}"),
            Self::WebSearch { query, .. } => format!("search: {query}"),
            Self::Mcp { name, arguments } => {
                let label = crate::mcp::split_advertised(name)
                    .map(|(server, tool)| format!("{tool}@{server}"))
                    .unwrap_or_else(|| name.clone());
                let preview: String = arguments.to_string().chars().take(60).collect();
                format!("{label}: {preview}")
            }
            Self::CronCreate {
                schedule,
                description,
                ..
            } => format!("cron `{schedule}`: {description}"),
            Self::CronList => "list cron jobs".into(),
            Self::CronDelete { id } => format!("delete cron: {id}"),
            Self::ScheduleWakeup {
                delay_seconds,
                reason,
                ..
            } => format!("wake in {delay_seconds}s: {reason}"),
            Self::Monitor { command, until } => {
                let preview: String = command.chars().take(40).collect();
                format!("monitor `{preview}` until /{until}/")
            }
            Self::Lsp {
                kind, file, line, ..
            } => format!("lsp {kind} {file}:{line}"),
            Self::PushNotification { message, title } => match title {
                Some(t) if !t.is_empty() => format!("{t}: {message}"),
                _ => message.clone(),
            },
            Self::RemoteTrigger { trigger_id, .. } => format!("trigger: {trigger_id}"),
            Self::EnterPlanMode { reason } => {
                let preview: String = reason.chars().take(60).collect();
                format!("enter plan mode: {preview}")
            }
            Self::EnterWorktree { name, branch } => match branch {
                Some(b) => format!("enter worktree {name} ({b})"),
                None => format!("enter worktree {name}"),
            },
            Self::ExitWorktree => "exit worktree".into(),
            Self::NotebookRead { path } => path.clone(),
            Self::NotebookEdit {
                path,
                cell_id,
                edit_mode,
                ..
            } => {
                let mode = edit_mode.as_deref().unwrap_or("replace");
                format!("notebook {mode} {path}#{cell_id}")
            }
            Self::Generic { summary } => summary.clone(),
        }
    }

    /// Boundary-validating constructor: each tool variant explicitly checks
    /// its required fields. Returns `Err` rather than silently substituting
    /// empty strings — the caller (stream.rs) emits a `Failed` tool result
    /// so the model sees the validation error in its `tool_result` block.
    ///
    /// "Validate at construction, trust thereafter." Once a `ToolInput` is
    /// built, downstream code can rely on required string fields actually
    /// being string-typed. `Bash::command` additionally must be non-empty.
    pub fn from_value(tool_name: &str, v: serde_json::Value) -> Result<Self, ToolInputError> {
        let obj = match &v {
            serde_json::Value::Object(m) => Some(m),
            _ => None,
        };
        let json_type_name = |val: &serde_json::Value| -> &'static str {
            match val {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            }
        };
        let tool = || tool_name.to_owned();
        // Required string field: must be present, must be a JSON string,
        // must not be `null`. Empty-string is allowed at this layer (the
        // executor / modal warning handles that case for Write::content,
        // and `Bash::command` is checked separately below).
        let req_str = |key: &'static str| -> Result<String, ToolInputError> {
            let Some(m) = obj else {
                return Err(ToolInputError::InvalidShape {
                    tool: tool(),
                    reason: "tool input was not a JSON object".into(),
                });
            };
            match m.get(key) {
                None | Some(serde_json::Value::Null) => Err(ToolInputError::MissingField {
                    tool: tool(),
                    field: key,
                }),
                Some(serde_json::Value::String(s)) => Ok(s.clone()),
                Some(other) => Err(ToolInputError::WrongType {
                    tool: tool(),
                    field: key,
                    expected: "string",
                    got: json_type_name(other),
                }),
            }
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
        // The Generic + Mcp + UnknownTool arms accept arbitrary shapes,
        // so don't require the input to be a JSON object. Every other
        // arm does.
        let kind = ToolKind::from_name(tool_name);
        let needs_object = !matches!(
            kind,
            ToolKind::Generic(_) | ToolKind::Mcp(_) | ToolKind::UnknownTool { .. }
        );
        if needs_object && obj.is_none() {
            return Err(ToolInputError::InvalidShape {
                tool: tool(),
                reason: format!(
                    "tool input must be a JSON object, got {}",
                    json_type_name(&v)
                ),
            });
        }
        let parsed = match kind {
            ToolKind::Edit => Self::Edit {
                file_path: req_str("file_path")?,
                old_string: req_str("old_string")?,
                new_string: req_str("new_string")?,
                replacement: ReplacementMode::from_replace_all(bool_field("replace_all")),
            },
            ToolKind::Write => Self::Write {
                file_path: req_str("file_path")?,
                content: req_str("content")?,
            },
            ToolKind::Read => Self::Read {
                file_path: req_str("file_path")?,
                offset: opt_u64_field("offset"),
                limit: opt_u64_field("limit"),
            },
            ToolKind::Bash => {
                let command = req_str("command")?;
                if command.is_empty() {
                    return Err(ToolInputError::InvalidShape {
                        tool: tool(),
                        reason: "Bash command must not be empty".into(),
                    });
                }
                Self::Bash {
                    command,
                    timeout: opt_u64_field("timeout"),
                    workdir: opt_str_field("workdir"),
                }
            }
            ToolKind::Glob => Self::Glob {
                pattern: req_str("pattern")?,
                path: opt_str_field("path"),
            },
            ToolKind::Grep => Self::Grep {
                pattern: req_str("pattern")?,
                path: opt_str_field("path"),
                glob: opt_str_field("glob"),
                output_mode: opt_str_field("output_mode"),
            },
            ToolKind::Search => Self::Search {
                query: req_str("query")?,
                path: opt_str_field("path"),
            },
            ToolKind::ApplyPatch => Self::ApplyPatch {
                patch: req_str("patch")?,
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
                    subject: req_str("subject")?,
                    description: req_str("description")?,
                    active_form: opt_str_field("active_form"),
                    blocked_by,
                }
            }
            ToolKind::TaskUpdate => Self::TaskUpdate {
                task_id: req_str("task_id")?,
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
                task_id: req_str("task_id")?,
            },
            ToolKind::TaskGet => Self::TaskGet {
                task_id: req_str("task_id")?,
            },
            ToolKind::Task => Self::Task(TaskInput {
                description: req_str("description")?,
                prompt: req_str("prompt")?,
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
                name: req_str("name")?,
                args: opt_str_field("args"),
            },
            ToolKind::MemoryCreate => Self::MemoryCreate {
                level: req_str("level")?,
                memory_type: req_str("memory_type")?,
                scope: req_str("scope")?,
                body: req_str("body")?,
            },
            ToolKind::MemoryDelete => Self::MemoryDelete {
                path: req_str("path")?,
            },
            ToolKind::TeamCreate => Self::TeamCreate {
                team_name: req_str("team_name")?,
                description: opt_str_field("description"),
            },
            ToolKind::TeamDelete => Self::TeamDelete,
            ToolKind::SendMessage => {
                // SendMessage's `message` accepts a string OR an object —
                // when an object arrives we serialize it to a JSON string
                // for the body. Treat missing/null as a validation error.
                let to = req_str("to")?;
                let message = match obj.and_then(|m| m.get("message")) {
                    None | Some(serde_json::Value::Null) => {
                        return Err(ToolInputError::MissingField {
                            tool: tool(),
                            field: "message",
                        });
                    }
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                };
                Self::SendMessage {
                    to,
                    message,
                    summary: opt_str_field("summary"),
                }
            }
            ToolKind::TeamMemberMode => Self::TeamMemberMode {
                member_name: req_str("member_name")?,
                mode: req_str("mode")?,
            },
            ToolKind::GraphQuery => Self::GraphQuery {
                query: req_str("query")?,
                max_tokens: obj
                    .and_then(|m| m.get("max_tokens"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize),
                include_handles: obj
                    .and_then(|m| m.get("include_handles"))
                    .and_then(|v| v.as_bool()),
            },
            ToolKind::RunCoverage => Self::RunCoverage {
                lcov_path: opt_str_field("lcov_path"),
                include_untested_list: obj
                    .and_then(|m| m.get("include_untested_list"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
            },
            ToolKind::SymbolEdit => Self::SymbolEdit {
                handle: req_str("handle")?,
                new_content: req_str("new_content")?,
                validate: bool_field("validate"),
                dispatch_cascade: bool_field("dispatch_cascade"),
            },
            ToolKind::PostBounty => Self::PostBounty {
                description: req_str("description")?,
                budget: obj
                    .and_then(|m| m.get("budget"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                acceptance_criteria: req_str("acceptance_criteria")?,
                max_solvers: obj
                    .and_then(|m| m.get("max_solvers"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(255) as u8),
                auto_dispatch: bool_field("auto_dispatch"),
            },
            ToolKind::MarketStatus => Self::MarketStatus {
                bounty_id: opt_str_field("bounty_id"),
            },
            ToolKind::RunBounty => Self::RunBounty {
                bounty_id: req_str("bounty_id")?,
                max_solvers: obj
                    .and_then(|m| m.get("max_solvers"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(255) as u8),
            },
            ToolKind::ExitPlanMode => Self::ExitPlanMode {
                plan: req_str("plan")?,
            },
            ToolKind::MultiEdit => Self::MultiEdit {
                file_path: req_str("file_path")?,
                edits: obj
                    .and_then(|m| m.get("edits"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
            },
            ToolKind::AskUserQuestion => Self::AskUserQuestion {
                question: req_str("question")?,
                options: obj
                    .and_then(|m| m.get("options"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Array(vec![])),
                multi_select: bool_field("multi_select"),
            },
            ToolKind::WebFetch => Self::WebFetch {
                url: req_str("url")?,
                prompt: opt_str_field("prompt"),
            },
            ToolKind::WebSearch => Self::WebSearch {
                query: req_str("query")?,
                max_results: obj
                    .and_then(|m| m.get("max_results"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n as u32),
            },
            ToolKind::Mcp(name) => Self::Mcp {
                name,
                arguments: v.clone(),
            },
            ToolKind::CronCreate => Self::CronCreate {
                schedule: req_str("schedule")?,
                command: req_str("command")?,
                description: req_str("description")?,
            },
            ToolKind::CronList => Self::CronList,
            ToolKind::CronDelete => Self::CronDelete { id: req_str("id")? },
            ToolKind::ScheduleWakeup => Self::ScheduleWakeup {
                delay_seconds: obj
                    .and_then(|m| m.get("delay_seconds"))
                    .and_then(|v| v.as_u64())
                    .map(|n| n.min(u32::MAX as u64) as u32)
                    .unwrap_or(0),
                prompt: req_str("prompt")?,
                reason: req_str("reason")?,
            },
            ToolKind::Monitor => Self::Monitor {
                command: req_str("command")?,
                until: req_str("until")?,
            },
            ToolKind::Lsp => Self::Lsp {
                kind: req_str("kind")?,
                file: req_str("file")?,
                line: obj
                    .and_then(|m| m.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                column: obj
                    .and_then(|m| m.get("column"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
            },
            ToolKind::PushNotification => Self::PushNotification {
                message: req_str("message")?,
                title: opt_str_field("title"),
            },
            ToolKind::RemoteTrigger => Self::RemoteTrigger {
                trigger_id: req_str("trigger_id")?,
                payload: obj.and_then(|m| m.get("payload")).cloned(),
            },
            ToolKind::EnterPlanMode => Self::EnterPlanMode {
                reason: req_str("reason")?,
            },
            ToolKind::EnterWorktree => Self::EnterWorktree {
                name: req_str("name")?,
                branch: opt_str_field("branch"),
            },
            ToolKind::ExitWorktree => Self::ExitWorktree,
            ToolKind::NotebookRead => Self::NotebookRead {
                path: req_str("path")?,
            },
            ToolKind::NotebookEdit => Self::NotebookEdit {
                path: req_str("path")?,
                cell_id: req_str("cell_id")?,
                new_source: req_str("new_source")?,
                edit_mode: opt_str_field("edit_mode"),
            },
            ToolKind::Generic(_) => Self::Generic {
                summary: v.to_string(),
            },
            // Unknown tools have no typed schema — preserve the raw JSON
            // in a Generic input so the transcript can render a summary.
            // Permission layer denies dispatch separately, so this never
            // actually executes.
            ToolKind::UnknownTool { .. } => Self::Generic {
                summary: v.to_string(),
            },
        };
        Ok(parsed)
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
            Self::TaskGet { task_id } => json!({ "task_id": task_id }),
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
            Self::GraphQuery {
                query,
                max_tokens,
                include_handles,
            } => {
                let mut v = json!({ "query": query });
                if let Some(mt) = max_tokens {
                    v["max_tokens"] = json!(mt);
                }
                if let Some(ih) = include_handles {
                    v["include_handles"] = json!(ih);
                }
                v
            }
            Self::RunCoverage {
                lcov_path,
                include_untested_list,
            } => {
                let mut v = json!({});
                if let Some(p) = lcov_path {
                    v["lcov_path"] = json!(p);
                }
                if !include_untested_list {
                    v["include_untested_list"] = json!(false);
                }
                v
            }
            Self::SymbolEdit {
                handle,
                new_content,
                validate,
                dispatch_cascade,
            } => {
                let mut v = json!({ "handle": handle, "new_content": new_content });
                if *validate {
                    v["validate"] = json!(true);
                }
                if *dispatch_cascade {
                    v["dispatch_cascade"] = json!(true);
                }
                v
            }
            Self::PostBounty {
                description,
                budget,
                acceptance_criteria,
                max_solvers,
                auto_dispatch,
            } => {
                let mut v = json!({
                    "description": description,
                    "budget": budget,
                    "acceptance_criteria": acceptance_criteria,
                });
                if let Some(n) = max_solvers {
                    v["max_solvers"] = json!(n);
                }
                if *auto_dispatch {
                    v["auto_dispatch"] = json!(true);
                }
                v
            }
            Self::MarketStatus { bounty_id } => {
                let mut v = json!({});
                if let Some(id) = bounty_id {
                    v["bounty_id"] = json!(id);
                }
                v
            }
            Self::RunBounty {
                bounty_id,
                max_solvers,
            } => {
                let mut v = json!({ "bounty_id": bounty_id });
                if let Some(n) = max_solvers {
                    v["max_solvers"] = json!(n);
                }
                v
            }
            Self::ExitPlanMode { plan } => json!({ "plan": plan }),
            Self::MultiEdit { file_path, edits } => json!({
                "file_path": file_path,
                "edits": edits,
            }),
            Self::AskUserQuestion {
                question,
                options,
                multi_select,
            } => json!({
                "question": question,
                "options": options,
                "multi_select": multi_select,
            }),
            Self::WebFetch { url, prompt } => {
                let mut v = json!({ "url": url });
                if let Some(p) = prompt {
                    v["prompt"] = json!(p);
                }
                v
            }
            Self::WebSearch { query, max_results } => {
                let mut v = json!({ "query": query });
                if let Some(n) = max_results {
                    v["max_results"] = json!(n);
                }
                v
            }
            Self::Mcp { arguments, .. } => arguments.clone(),
            Self::CronCreate {
                schedule,
                command,
                description,
            } => json!({
                "schedule": schedule,
                "command": command,
                "description": description,
            }),
            Self::CronList => json!({}),
            Self::CronDelete { id } => json!({ "id": id }),
            Self::ScheduleWakeup {
                delay_seconds,
                prompt,
                reason,
            } => json!({
                "delay_seconds": delay_seconds,
                "prompt": prompt,
                "reason": reason,
            }),
            Self::Monitor { command, until } => json!({
                "command": command,
                "until": until,
            }),
            Self::Lsp {
                kind,
                file,
                line,
                column,
            } => {
                json!({ "kind": kind, "file": file, "line": line, "column": column })
            }
            Self::PushNotification { message, title } => {
                let mut v = json!({ "message": message });
                if let Some(t) = title {
                    v["title"] = json!(t);
                }
                v
            }
            Self::RemoteTrigger {
                trigger_id,
                payload,
            } => {
                let mut v = json!({ "trigger_id": trigger_id });
                if let Some(p) = payload {
                    v["payload"] = p.clone();
                }
                v
            }
            Self::EnterPlanMode { reason } => json!({ "reason": reason }),
            Self::EnterWorktree { name, branch } => {
                let mut v = json!({ "name": name });
                if let Some(b) = branch {
                    v["branch"] = json!(b);
                }
                v
            }
            Self::ExitWorktree => json!({}),
            Self::NotebookRead { path } => json!({ "path": path }),
            Self::NotebookEdit {
                path,
                cell_id,
                new_source,
                edit_mode,
            } => {
                let mut v = json!({
                    "path": path,
                    "cell_id": cell_id,
                    "new_source": new_source,
                });
                if let Some(m) = edit_mode {
                    v["edit_mode"] = json!(m);
                }
                v
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
            status: ToolStatus::Completed,
            input: ToolInput::Edit {
                file_path: "crates/jfc-ui/src/tools.rs".into(),
                old_string: "let timeout = timeout_ms.unwrap_or(120_000);".into(),
                new_string: "let timeout = timeout_ms.unwrap_or(300_000);".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Diff(diff),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }),
        MessagePart::Tool(ToolCall {
            id: "bash-1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
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
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }),
        MessagePart::Tool(ToolCall {
            id: "read-1".into(),
            kind: ToolKind::Read,
            status: ToolStatus::Completed,
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
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
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
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
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
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
        }),
        MessagePart::Tool(ToolCall {
            id: "patch-1".into(),
            kind: ToolKind::ApplyPatch,
            status: ToolStatus::Completed,
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
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
        }),
        MessagePart::Tool(ToolCall {
            id: "generic-1".into(),
            kind: ToolKind::Generic("Delegate".into()),
            status: ToolStatus::Failed,
            input: ToolInput::Generic {
                summary: "OpenClaude remote lookup".into(),
            },
            output: ToolOutput::Empty,
            display: ToolDisplayState::Collapsed,
            elapsed_ms: None,
            started_at: None,
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

    // Truly unknown names route to UnknownTool — distinct from Generic
    // (which is for deliberately-named tools whose semantics we know
    // but don't represent as first-class variants). The variant exists
    // so adding a new ToolKind::Foo is a compile error at every match
    // site instead of a silent dispatch to Generic("Foo").
    #[test]
    fn from_name_unknown_falls_through_to_unknown_tool_robust() {
        match ToolKind::from_name("not_a_real_tool") {
            ToolKind::UnknownTool { advertised_name } => {
                assert_eq!(advertised_name, "not_a_real_tool")
            }
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    // MCP-namespaced names route to the Mcp variant carrying the full
    // advertised name.
    #[test]
    fn from_name_mcp_prefixed_routes_to_mcp_variant_normal() {
        match ToolKind::from_name("mcp__filesystem__read_file") {
            ToolKind::Mcp(s) => assert_eq!(s, "mcp__filesystem__read_file"),
            other => panic!("expected Mcp, got {other:?}"),
        }
    }

    #[test]
    fn from_name_mcp_without_separator_is_unknown_tool_robust() {
        // Without the `mcp__` prefix the name is just an unknown tool,
        // not an MCP-routed call.
        match ToolKind::from_name("mcp_dispatch") {
            ToolKind::UnknownTool { advertised_name } => {
                assert_eq!(advertised_name, "mcp_dispatch")
            }
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    // The 8 v2.1.132 tools must all parse from PascalCase and snake_case.
    #[test]
    fn from_name_resolves_v2_1_132_tools_normal() {
        assert!(matches!(ToolKind::from_name("LSP"), ToolKind::Lsp));
        assert!(matches!(ToolKind::from_name("lsp"), ToolKind::Lsp));
        assert!(matches!(
            ToolKind::from_name("PushNotification"),
            ToolKind::PushNotification
        ));
        assert!(matches!(
            ToolKind::from_name("push_notification"),
            ToolKind::PushNotification
        ));
        assert!(matches!(
            ToolKind::from_name("RemoteTrigger"),
            ToolKind::RemoteTrigger
        ));
        assert!(matches!(
            ToolKind::from_name("remote_trigger"),
            ToolKind::RemoteTrigger
        ));
        assert!(matches!(
            ToolKind::from_name("EnterPlanMode"),
            ToolKind::EnterPlanMode
        ));
        assert!(matches!(
            ToolKind::from_name("EnterWorktree"),
            ToolKind::EnterWorktree
        ));
        assert!(matches!(
            ToolKind::from_name("ExitWorktree"),
            ToolKind::ExitWorktree
        ));
        assert!(matches!(
            ToolKind::from_name("NotebookRead"),
            ToolKind::NotebookRead
        ));
        assert!(matches!(
            ToolKind::from_name("NotebookEdit"),
            ToolKind::NotebookEdit
        ));
    }

    #[test]
    fn label_v2_1_132_tools_normal() {
        assert_eq!(ToolKind::Lsp.label(), "LSP");
        assert_eq!(ToolKind::PushNotification.label(), "PushNotification");
        assert_eq!(ToolKind::RemoteTrigger.label(), "RemoteTrigger");
        assert_eq!(ToolKind::EnterPlanMode.label(), "EnterPlanMode");
        assert_eq!(ToolKind::EnterWorktree.label(), "EnterWorktree");
        assert_eq!(ToolKind::ExitWorktree.label(), "ExitWorktree");
        assert_eq!(ToolKind::NotebookRead.label(), "NotebookRead");
        assert_eq!(ToolKind::NotebookEdit.label(), "NotebookEdit");
    }

    #[test]
    fn api_name_v2_1_132_tools_normal() {
        assert_eq!(ToolKind::Lsp.api_name(), "LSP");
        assert_eq!(ToolKind::PushNotification.api_name(), "PushNotification");
        assert_eq!(ToolKind::RemoteTrigger.api_name(), "RemoteTrigger");
        assert_eq!(ToolKind::EnterPlanMode.api_name(), "EnterPlanMode");
        assert_eq!(ToolKind::EnterWorktree.api_name(), "EnterWorktree");
        assert_eq!(ToolKind::ExitWorktree.api_name(), "ExitWorktree");
        assert_eq!(ToolKind::NotebookRead.api_name(), "NotebookRead");
        assert_eq!(ToolKind::NotebookEdit.api_name(), "NotebookEdit");
    }

    /// The summary string is what shows in the tool row's right column.
    /// Each new tool needs a non-empty, distinguishable summary so the UI
    /// doesn't show identical placeholder strings for multiple calls.
    #[test]
    fn summary_v2_1_132_tools_normal() {
        let lsp = ToolInput::Lsp {
            kind: "hover".into(),
            file: "/tmp/x.rs".into(),
            line: 12,
            column: 4,
        };
        assert!(lsp.summary().contains("hover"), "{}", lsp.summary());
        assert!(lsp.summary().contains("/tmp/x.rs:12"), "{}", lsp.summary());

        let pn = ToolInput::PushNotification {
            message: "hi".into(),
            title: Some("CI".into()),
        };
        assert_eq!(pn.summary(), "CI: hi");

        let rt = ToolInput::RemoteTrigger {
            trigger_id: "deploy".into(),
            payload: None,
        };
        assert_eq!(rt.summary(), "trigger: deploy");

        let pm = ToolInput::EnterPlanMode {
            reason: "double check".into(),
        };
        assert!(pm.summary().contains("double check"), "{}", pm.summary());

        let ew = ToolInput::EnterWorktree {
            name: "feat".into(),
            branch: Some("dev".into()),
        };
        assert!(ew.summary().contains("feat"), "{}", ew.summary());
        assert!(ew.summary().contains("dev"), "{}", ew.summary());

        assert_eq!(ToolInput::ExitWorktree.summary(), "exit worktree");

        let nr = ToolInput::NotebookRead {
            path: "/tmp/n.ipynb".into(),
        };
        assert_eq!(nr.summary(), "/tmp/n.ipynb");

        let ne = ToolInput::NotebookEdit {
            path: "/tmp/n.ipynb".into(),
            cell_id: "c1".into(),
            new_source: "x".into(),
            edit_mode: Some("insert".into()),
        };
        assert!(ne.summary().contains("insert"), "{}", ne.summary());
        assert!(ne.summary().contains("c1"), "{}", ne.summary());
    }

    /// from_value/to_value round-trip for each new tool's parameters.
    #[test]
    fn from_value_to_value_round_trip_v2_1_132_robust() {
        let cases: Vec<(&str, serde_json::Value)> = vec![
            (
                "LSP",
                serde_json::json!({"kind": "definition", "file": "/a/b.rs", "line": 3, "column": 7}),
            ),
            (
                "PushNotification",
                serde_json::json!({"message": "ok", "title": "build"}),
            ),
            (
                "RemoteTrigger",
                serde_json::json!({"trigger_id": "deploy", "payload": {"k": "v"}}),
            ),
            ("EnterPlanMode", serde_json::json!({"reason": "audit"})),
            (
                "EnterWorktree",
                serde_json::json!({"name": "feat", "branch": "main"}),
            ),
            ("ExitWorktree", serde_json::json!({})),
            ("NotebookRead", serde_json::json!({"path": "/tmp/x.ipynb"})),
            (
                "NotebookEdit",
                serde_json::json!({
                    "path": "/tmp/x.ipynb",
                    "cell_id": "c1",
                    "new_source": "y = 2",
                    "edit_mode": "replace",
                }),
            ),
        ];
        for (name, v) in cases {
            let parsed = ToolInput::from_value(name, v.clone())
                .unwrap_or_else(|e| panic!("from_value failed for {name}: {e}"));
            let back = parsed.to_value();
            for (k, vv) in v.as_object().unwrap() {
                assert_eq!(
                    &back[k], vv,
                    "round-trip lost field {k} for {name}: back={back}"
                );
            }
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
    //
    // ToolStatus is now a type alias for ExecutionStatus (the unified
    // lifecycle enum). Labels follow the canonical ExecutionStatus
    // names: Completed → "completed" (not "done", which was the old
    // Tool-only spelling — UI sites that want the friendlier word can
    // map it themselves).

    #[test]
    fn tool_status_labels_normal() {
        assert_eq!(ToolStatus::Pending.label(), "pending");
        assert_eq!(ToolStatus::Running.label(), "running");
        assert_eq!(ToolStatus::Completed.label(), "completed");
        assert_eq!(ToolStatus::Failed.label(), "failed");
    }

    #[test]
    fn tool_status_alias_equals_task_lifecycle_normal() {
        // Both names alias the same underlying ExecutionStatus enum.
        // Exercising equality across the alias names guards against a
        // future "let's split them again" regression.
        let a: ToolStatus = ToolStatus::Completed;
        let b: TaskLifecycle = TaskLifecycle::Completed;
        assert_eq!(a, b);
    }

    // ─── ExecutionStatus transitions ──────────────────────────────────────

    #[test]
    fn execution_status_is_terminal_complete_normal() {
        assert!(ExecutionStatus::Completed.is_terminal());
        assert!(ExecutionStatus::Failed.is_terminal());
        assert!(ExecutionStatus::Cancelled.is_terminal());
        assert!(!ExecutionStatus::Pending.is_terminal());
        assert!(!ExecutionStatus::Running.is_terminal());
        assert!(!ExecutionStatus::Idle.is_terminal());
    }

    #[test]
    fn execution_status_allows_transition_normal() {
        // Forward edges from non-terminal states: any move is OK,
        // including the Idle exit (Tasks legitimately go Idle → Running
        // when a teammate picks up new mail).
        assert!(ExecutionStatus::Pending.allows_transition_to(ExecutionStatus::Running));
        assert!(ExecutionStatus::Running.allows_transition_to(ExecutionStatus::Completed));
        assert!(ExecutionStatus::Idle.allows_transition_to(ExecutionStatus::Running));
        // Terminal lock-in: nothing leaves Failed/Completed/Cancelled.
        assert!(!ExecutionStatus::Failed.allows_transition_to(ExecutionStatus::Running));
        assert!(!ExecutionStatus::Completed.allows_transition_to(ExecutionStatus::Failed));
        assert!(!ExecutionStatus::Cancelled.allows_transition_to(ExecutionStatus::Pending));
        // Idempotent same-state transitions are allowed (the stream
        // layer occasionally re-asserts the same status on retry).
        assert!(ExecutionStatus::Completed.allows_transition_to(ExecutionStatus::Completed));
        assert!(ExecutionStatus::Failed.allows_transition_to(ExecutionStatus::Failed));
    }

    fn fixture_pending_tool() -> ToolCall {
        ToolCall::new_pending(
            crate::ids::ToolId::from("test-tool-1".to_owned()),
            ToolKind::Bash,
            ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
            },
        )
    }

    #[test]
    fn tool_call_pending_to_running_normal() {
        let mut tc = fixture_pending_tool();
        assert_eq!(tc.status, ExecutionStatus::Pending);
        assert!(tc.mark_running().is_ok());
        assert_eq!(tc.status, ExecutionStatus::Running);
    }

    #[test]
    fn tool_call_pending_to_running_to_completed_normal() {
        let mut tc = fixture_pending_tool();
        tc.mark_running().expect("Pending → Running should succeed");
        tc.mark_completed()
            .expect("Running → Completed should succeed");
        assert_eq!(tc.status, ExecutionStatus::Completed);
    }

    #[test]
    fn tool_call_pending_directly_to_completed_normal() {
        // Some provider streams collapse Pending and skip directly to
        // Completed when a tool was approved + executed faster than
        // the UI can poll. The transition rules allow this.
        let mut tc = fixture_pending_tool();
        tc.mark_completed()
            .expect("Pending → Completed should succeed");
        assert_eq!(tc.status, ExecutionStatus::Completed);
    }

    #[test]
    fn tool_call_failed_to_running_returns_err_robust() {
        let mut tc = fixture_pending_tool();
        tc.mark_failed().unwrap();
        let err = tc
            .mark_running()
            .expect_err("Failed → Running must be refused");
        assert_eq!(err.from, ExecutionStatus::Failed);
        assert_eq!(err.to, ExecutionStatus::Running);
        // Status stays at Failed — refused transitions don't mutate.
        assert_eq!(tc.status, ExecutionStatus::Failed);
    }

    #[test]
    fn tool_call_completed_to_failed_returns_err_robust() {
        let mut tc = fixture_pending_tool();
        tc.mark_completed().unwrap();
        let err = tc
            .mark_failed()
            .expect_err("Completed → Failed must be refused");
        assert_eq!(err.from, ExecutionStatus::Completed);
        assert_eq!(err.to, ExecutionStatus::Failed);
        assert_eq!(tc.status, ExecutionStatus::Completed);
    }

    #[test]
    fn tool_call_cancel_from_pending_normal() {
        let mut tc = fixture_pending_tool();
        tc.mark_cancelled()
            .expect("Pending → Cancelled should succeed");
        assert_eq!(tc.status, ExecutionStatus::Cancelled);
        // Now terminal — further transitions refused.
        assert!(tc.mark_completed().is_err());
    }

    #[test]
    fn tool_call_idempotent_same_state_normal() {
        // Re-asserting the same status doesn't error — protects the
        // stream layer from spurious "you already said Running" panics
        // when the provider replays an event mid-stream.
        let mut tc = fixture_pending_tool();
        tc.mark_running().unwrap();
        tc.mark_running().expect("Running → Running is idempotent");
        assert_eq!(tc.status, ExecutionStatus::Running);
    }

    #[test]
    fn tool_call_new_failed_constructor_normal() {
        // new_failed lands directly in the terminal Failed state for
        // the malformed-input path (stream.rs ToolDone handler).
        let tc = ToolCall::new_failed(
            crate::ids::ToolId::from("toolu_x".to_owned()),
            ToolKind::Bash,
            ToolInput::Generic {
                summary: "(empty input for Bash)".into(),
            },
            ToolOutput::Text("bad JSON".into()),
        );
        assert_eq!(tc.status, ExecutionStatus::Failed);
        assert!(matches!(tc.output, ToolOutput::Text(_)));
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
        let view = parse_unified_diff("x.rs", "@@ -1,1 +1,1 @@\n-abc\n+abcd\n");
        let out = ToolOutput::Diff(view);
        // "abc" (3) + "abcd" (4) = 7
        assert_eq!(out.approx_text_len(), 7);
    }

    #[test]
    fn tool_output_text_only_diff_includes_counts_normal() {
        let view = parse_unified_diff("x.rs", "@@ -1,1 +1,1 @@\n-old\n+new\n");
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
        let s = ToolOutput::FileList(vec!["a".into(), "b".into(), "c".into()]).text_only();
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
        let input = ToolInput::from_value("Edit", v).expect("valid Edit input");
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
        let input = ToolInput::from_value("Read", v).expect("valid Read input");
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
        let input = ToolInput::from_value("Task", v).expect("valid Task input");
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
        let input = ToolInput::from_value("TaskCreate", v).expect("valid TaskCreate input");
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
        let input = ToolInput::from_value("SendMessage", v).expect("valid SendMessage input");
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
        let input = ToolInput::from_value("not_a_real_tool", v).expect("Generic accepts any shape");
        match input {
            ToolInput::Generic { summary } => {
                // Generic stores the original JSON as a string.
                assert!(summary.contains("foo"), "{summary}");
                assert!(summary.contains("bar"), "{summary}");
            }
            _ => panic!("expected Generic"),
        }
    }

    /// Inverted from the prior `..._handles_missing_fields_robust` test,
    /// which asserted that missing fields silently defaulted to empty
    /// strings. That behavior shipped a real bug: a malformed Write
    /// tool-use with `{"content": null}` got dispatched as
    /// `Write { file_path: "", content: "" }` and tried to truncate a
    /// real file. The boundary is now strict — missing required fields
    /// produce a typed `ToolInputError::MissingField` so the stream
    /// loop emits a `Failed` tool_result the model can react to.
    #[test]
    fn tool_input_from_value_rejects_missing_fields_robust() {
        let v = serde_json::json!({});
        let err = ToolInput::from_value("Edit", v)
            .expect_err("Edit with empty payload must fail validation");
        match err {
            ToolInputError::MissingField { tool, field } => {
                assert_eq!(tool, "Edit");
                // file_path is the first required field checked.
                assert_eq!(field, "file_path");
            }
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    /// The original symptom: provider sends `{"content": null}` for a
    /// Write tool. Old behavior coerced this into `content: ""` and
    /// happily queued an empty-content overwrite for user approval.
    /// New behavior rejects with `MissingField` (we treat null the same
    /// as absent at the boundary).
    #[test]
    fn tool_input_from_value_rejects_write_with_null_content_robust() {
        let v = serde_json::json!({"file_path": "/etc/passwd", "content": null});
        let err = ToolInput::from_value("Write", v).expect_err("Write with null content must fail");
        assert_eq!(
            err,
            ToolInputError::MissingField {
                tool: "Write".into(),
                field: "content",
            }
        );
    }

    /// Bash::command must be present AND non-empty — an empty bash
    /// command can't do anything useful and frequently signals the
    /// model truncated mid-call.
    #[test]
    fn tool_input_from_value_rejects_bash_with_empty_command_robust() {
        let v = serde_json::json!({"command": ""});
        let err = ToolInput::from_value("Bash", v).expect_err("Bash with empty command must fail");
        match err {
            ToolInputError::InvalidShape { tool, reason } => {
                assert_eq!(tool, "Bash");
                assert!(
                    reason.contains("must not be empty"),
                    "expected non-empty hint, got: {reason}"
                );
            }
            other => panic!("expected InvalidShape, got {other:?}"),
        }
    }

    /// Read::file_path is required — Read with an empty payload should
    /// surface `MissingField{tool: "Read", field: "file_path"}` rather
    /// than silently building `Read { file_path: "" }`.
    #[test]
    fn tool_input_from_value_rejects_read_missing_file_path_robust() {
        let v = serde_json::json!({"offset": 0, "limit": 100});
        let err = ToolInput::from_value("Read", v).expect_err("Read with no file_path must fail");
        assert_eq!(
            err,
            ToolInputError::MissingField {
                tool: "Read".into(),
                field: "file_path",
            }
        );
    }

    /// Wrong-typed required field (a number where a string is expected)
    /// must surface `WrongType` so the diagnostic message tells the
    /// model exactly what shape is expected.
    #[test]
    fn tool_input_from_value_rejects_wrong_typed_field_robust() {
        let v = serde_json::json!({"file_path": 42, "content": "hi"});
        let err = ToolInput::from_value("Write", v).expect_err("file_path must be a string");
        match err {
            ToolInputError::WrongType {
                tool,
                field,
                expected,
                got,
            } => {
                assert_eq!(tool, "Write");
                assert_eq!(field, "file_path");
                assert_eq!(expected, "string");
                assert_eq!(got, "number");
            }
            other => panic!("expected WrongType, got {other:?}"),
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
        assert!(
            m.role_is_user(),
            "compact boundary uses user role for replay"
        );
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
        let view = parse_unified_diff("x.rs", "@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n");
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

    // ─── validate_turn_invariants ─────────────────────────────────────────

    fn pending_tool_call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Pending,
            input: ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Empty,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    fn complete_tool_call(id: &str) -> ToolCall {
        ToolCall {
            status: ToolStatus::Completed,
            output: ToolOutput::Text("ok".into()),
            ..pending_tool_call(id)
        }
    }

    /// Normal: a healthy alternating user/assistant transcript passes
    /// validation cleanly. Empty inputs are also accepted.
    #[test]
    fn validate_turn_invariants_accepts_alternating_transcript_normal() {
        assert!(validate_turn_invariants(&[]).is_ok());
        let msgs = vec![
            ChatMessage::user("hi".into()),
            ChatMessage::assistant("hey".into()),
            ChatMessage::user("more".into()),
            ChatMessage::assistant("ok".into()),
        ];
        validate_turn_invariants(&msgs).expect("alternating transcript is valid");
    }

    /// Robust: two adjacent user messages surface ConsecutiveUser at the
    /// SECOND user's index — that's the position the queue-drain bug
    /// would land at.
    #[test]
    fn validate_turn_invariants_flags_consecutive_user_robust() {
        let msgs = vec![
            ChatMessage::user("first".into()),
            ChatMessage::user("second".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("must flag consecutive user");
        assert_eq!(err, TurnInvariantError::ConsecutiveUser { at_index: 1 });
    }

    /// Robust: this is the structural shape of the plan-continuation
    /// phantom-assistant bug — two assistant messages back-to-back.
    #[test]
    fn validate_turn_invariants_flags_consecutive_assistant_robust() {
        let msgs = vec![
            ChatMessage::user("hi".into()),
            ChatMessage::assistant("a".into()),
            ChatMessage::assistant("b".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("must flag consecutive assistant");
        assert_eq!(
            err,
            TurnInvariantError::ConsecutiveAssistant { at_index: 2 }
        );
    }

    /// Robust: a fully empty user message (no text, no tools, no
    /// boundary) trips EmptyMessage. The streaming-tail exception
    /// only applies to assistants, so a user-empty must always fail.
    #[test]
    fn validate_turn_invariants_flags_empty_user_robust() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            parts: vec![MessagePart::Text(String::new())],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        }];
        let err = validate_turn_invariants(&msgs).expect_err("empty user must fail");
        assert_eq!(
            err,
            TurnInvariantError::EmptyMessage {
                at_index: 0,
                role: Role::User,
            }
        );
    }

    /// Normal: an empty assistant message at the tail of the slice
    /// is allowed when `allow_streaming_tail = true` — that's the
    /// placeholder slot `continue_agentic_loop` stages right before
    /// the stream starts pumping.
    #[test]
    fn validate_turn_invariants_streaming_tail_allowed_normal() {
        let msgs = vec![
            ChatMessage::user("hi".into()),
            ChatMessage::assistant(String::new()),
        ];
        // Strict mode rejects the empty placeholder.
        let err = validate_turn_invariants(&msgs).expect_err("strict mode rejects empty tail");
        assert!(matches!(err, TurnInvariantError::EmptyMessage { .. }));
        // Permissive mode accepts it (the streaming pipeline is about
        // to fill it in).
        validate_turn_invariants_inner(&msgs, /* allow_streaming_tail = */ true)
            .expect("streaming-tail mode accepts empty trailing assistant");
    }

    /// Robust: a Pending tool on a non-tail assistant message means
    /// the model rolled forward without a tool_result — surface as
    /// OrphanToolUse carrying the tool id and index.
    #[test]
    fn validate_turn_invariants_flags_orphan_tool_use_robust() {
        let msgs = vec![
            ChatMessage::user("run it".into()),
            ChatMessage::assistant_parts(vec![MessagePart::Tool(pending_tool_call("tool_42"))]),
            ChatMessage::user("never mind".into()),
            ChatMessage::assistant("ok".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("must flag orphan tool_use");
        match err {
            TurnInvariantError::OrphanToolUse { tool_id, at_index } => {
                assert_eq!(tool_id, crate::ids::ToolId::new("tool_42"));
                assert_eq!(at_index, 1);
            }
            other => panic!("expected OrphanToolUse, got {other:?}"),
        }
    }

    /// Robust: a Tool part on a Role::User message is structurally
    /// misrouted — tool calls always belong to assistant turns.
    #[test]
    fn validate_turn_invariants_flags_tool_on_user_role_robust() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            parts: vec![
                MessagePart::Text("hi".into()),
                MessagePart::Tool(complete_tool_call("tool_99")),
            ],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
        }];
        let err = validate_turn_invariants(&msgs).expect_err("tool part on user role must fail");
        match err {
            TurnInvariantError::OrphanToolResult { tool_id, at_index } => {
                assert_eq!(tool_id, crate::ids::ToolId::new("tool_99"));
                assert_eq!(at_index, 0);
            }
            other => panic!("expected OrphanToolResult, got {other:?}"),
        }
    }

    /// Robust: a transcript that opens with an Assistant message
    /// (without a system-injected boundary) is the visual symptom of
    /// the phantom-leading-slot bug. Surface as LeadingAssistant.
    #[test]
    fn validate_turn_invariants_flags_leading_assistant_robust() {
        let msgs = vec![
            ChatMessage::assistant("oops, I went first".into()),
            ChatMessage::user("hi".into()),
        ];
        let err = validate_turn_invariants(&msgs).expect_err("leading assistant must fail");
        assert_eq!(
            err,
            TurnInvariantError::LeadingAssistant {
                role: Role::Assistant,
            }
        );
    }

    /// Normal: a CompactBoundary is a legitimate Role::User message
    /// that may be followed by another User-role reply describing the
    /// resumed task. The validator must accept that exact seam.
    #[test]
    fn validate_turn_invariants_compact_boundary_seam_allowed_normal() {
        let msgs = vec![
            ChatMessage::user("first round".into()),
            ChatMessage::assistant("ok".into()),
            ChatMessage::compact_boundary("summary text", 12_000),
            ChatMessage::user("continue from here".into()),
            ChatMessage::assistant("resuming".into()),
        ];
        validate_turn_invariants(&msgs)
            .expect("compact boundary may sit between two user messages");
    }
}
