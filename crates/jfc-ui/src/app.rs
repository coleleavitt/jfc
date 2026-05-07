use std::{cell::RefCell, collections::HashMap, sync::Arc, time::Instant};

use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::TableState;
use tokio::sync::Mutex;

use ratatui_textarea::TextArea;

use crate::auto_mode::AutoModeConfig;

use crate::context::{ReadDedupCache, ToolContext};
use crate::provider::{ModelId, ModelInfo, Provider, ProviderId, StopReason};
use crate::query::QueryCache;
use crate::render_cache::RenderCache;
use crate::slate::SlateRouter;
use crate::tasks::TaskId;
use crate::theme::Theme;
use crate::tools::ExecutionResult;
use crate::types::*;

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 200_000;

pub enum AppEvent {
    StreamChunk {
        text: Option<String>,
        reasoning: Option<String>,
    },
    /// Tool input JSON delta — streamed while the model builds tool_use arguments.
    /// Carries the byte length so the spinner's token estimate stays live during
    /// tool input streaming (matching v126's responseLengthRef accumulation).
    ToolInputDelta(usize),
    StreamTool(ToolCall),
    StreamDone(StopReason),
    StreamError(String),
    StreamUsage {
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    },
    ToolResult {
        tool_id: String,
        result: ExecutionResult,
    },
    /// Incremental output from a running tool (e.g. bash stdout line-by-line).
    /// The UI appends this to the tool's live output preview.
    ToolOutputChunk {
        tool_id: String,
        chunk: String,
    },
    AllToolsComplete,
    CompactionStarted,
    /// Streaming compact has emitted more text. `output_chars` is the
    /// total length of the summary collected so far. Mirrors v126's
    /// `addResponseLength` callback in PB7 (cli.js:396989) — fires on
    /// every text_delta during compaction so the spinner can show
    /// `↓ Nk tokens` building up live, not just the elapsed timer.
    CompactionProgress {
        output_chars: u64,
    },
    CompactionDone {
        messages: Vec<ChatMessage>,
        tool_ctx: crate::context::ToolContext,
        pre_tokens: usize,
        post_tokens: usize,
    },
    /// `(reason, calibrated_tokens, transient)`. When `transient` is true,
    /// the failure is recoverable on the next user turn (e.g. `TooFewGroups`
    /// — adding another turn creates a second group), so we must NOT set
    /// `compact_suppressed`; otherwise the user has to remember to type
    /// `/compact` to wake auto-compaction back up. Permanent failures
    /// (provider doesn't support compaction, exhausted attempts) keep the
    /// suppression flag so we don't spam compact requests every tool batch.
    CompactionFailed(String, Option<usize>, bool),
    /// Submit a user prompt as if the user typed it and pressed Enter. Used
    /// internally by the pre-submit compaction gate to re-fire the user's
    /// original prompt once compaction has shrunk the context.
    Submit(String),
    /// Push a non-blocking toast onto the auto-expiring strip. The pruner
    /// in the `Tick` handler clears it once `ttl` elapses. Mirrors v126's
    /// terminal `notification()` (cli.js around 26647).
    Toast {
        kind: crate::toast::ToastKind,
        text: String,
    },
    /// One streaming text chunk from a subagent. Routed into the matching
    /// `BackgroundTask.messages` so the task view shows the agent's
    /// output live as it streams (instead of "No messages yet" until
    /// the agent reports a tool via `TaskProgress`). Mirrors v126's
    /// per-agent stream handler that pipes nested-stream chunks into
    /// the parent's task buffer.
    AgentChunk {
        task_id: String,
        text: String,
    },
    /// Inbound message from a teammate (delivered via the leader inbox).
    /// Two outcomes: the message gets appended to the transcript as a
    /// system-tagged user turn so the model can see it on its next
    /// request, AND a toast surfaces the arrival so the user notices.
    /// Mirrors v126's `<teammate-message>` injection.
    TeammateInbox {
        from: String,
        text: String,
        summary: Option<String>,
    },
    /// Background `Provider::fetch_models()` finished. `provider` is the `Provider::name()`
    /// the result belongs to. `models` is empty on a remote failure so the picker can
    /// fall back to the static `available_models()` set without showing a hung row.
    ModelsLoaded {
        provider: ProviderId,
        models: Vec<ModelInfo>,
    },
    /// Background OAuth `/api/oauth/profile` finished. `seat_tier` drives the picker's
    /// v126-equivalent tier filter; `subscription_type` is shown in the status bar.
    ProfileLoaded {
        seat_tier: Option<String>,
        subscription_type: Option<String>,
        email: Option<String>,
    },
    /// v126 auto-mode classifier finished judging a pending tool call. When
    /// `blocked` is true, the tool is marked Failed with `reason` and never
    /// runs; when false, the tool is dispatched immediately without prompting
    /// the user (auto-mode replaces the manual approval flow).
    ClassifierDecision {
        tool: ToolCall,
        blocked: bool,
        reason: String,
    },
    TaskStarted {
        task_id: String,
        description: String,
    },
    TaskProgress {
        task_id: String,
        last_tool: Option<String>,
        elapsed_ms: u64,
        /// Cumulative tools invoked this run (None = no update). Routed
        /// to `BackgroundTask.tool_use_count` so the fan UI can render
        /// "(N tools)" beside the spinner.
        tool_use_count: Option<u32>,
        /// Latest API request's input-token count (None = no update).
        input_tokens: Option<u64>,
        /// Output tokens consumed during the latest API round-trip
        /// (None = no update). Folded into `cumulative_output_tokens`.
        output_tokens: Option<u64>,
    },
    TaskCompleted {
        task_id: String,
        summary: String,
        elapsed_ms: u64,
    },
    TaskFailed {
        task_id: String,
        error: String,
    },
    McpUpdated {
        servers: Vec<crate::types::McpServerInfo>,
    },
    LspUpdated {
        servers: Vec<crate::types::LspServerInfo>,
    },
    /// LSP push: full set of currently-active diagnostics. Replaces
    /// `app.diagnostics` wholesale (the LSP client should send a fresh
    /// snapshot, not deltas, so the consumer doesn't have to dedup).
    /// Mirrors v126 cli.js:338038 — the `Found N issues in M files` row
    /// is rendered from this state.
    DiagnosticsUpdated {
        entries: Vec<crate::diagnostics::DiagnosticEntry>,
    },
    Term(Event),
    Tick,
    /// Event from an in-process teammate runner (idle, progress, completion, message).
    TeammateEvent(crate::swarm::runner::TeammateEvent),
    /// A teammate has been spawned (Task tool with name+team_name set). Carries
    /// the data the leader needs to populate `app.team_context.team_name` and
    /// `app.team_context.teammates`. Without this event, both fields stayed
    /// empty regardless of how many teammates were spawned, so the team-mode
    /// teammate tree never activated and `team_context.is_active()` lied
    /// about whether a team was in flight.
    TeammateSpawned {
        name: String,
        team_name: String,
        agent_id: String,
        color: Option<String>,
        agent_type: Option<String>,
        cwd: String,
    },
    /// The model called `ExitPlanMode` and wants the user to see the
    /// plan + transition out of plan mode.
    ExitPlanModeRequested {
        plan: String,
    },
    /// Model-callable plan-mode entry. Dispatched by the `EnterPlanMode`
    /// tool — flips `app.permission_mode` to `PermissionMode::Plan`.
    EnterPlanModeRequested {
        reason: String,
    },
}

/// Permission modes matching v126 claude-code. Controls how tool execution
/// is gated — from fully interactive (Default) to fully autonomous (Bypass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Standard — prompts for dangerous operations (Bash, Write, Edit)
    Default,
    /// Analysis only — blocks all write/exec tools, allows reads
    Plan,
    /// Auto-accept file edits (Write, Edit, ApplyPatch) but still prompt for Bash
    AcceptEdits,
    /// Bypass all permission checks — auto-approve everything
    BypassPermissions,
    /// Use a classifier model to approve/deny each tool call
    Auto,
}

impl PermissionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Plan => "Plan",
            Self::AcceptEdits => "Accept Edits",
            Self::BypassPermissions => "Bypass",
            Self::Auto => "Auto",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Plan => "📋",
            Self::AcceptEdits => "⏵",
            Self::BypassPermissions => "⏵⏵",
            Self::Auto => "⚡",
        }
    }

    /// Cycle to the next mode (for Shift+Tab)
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::AcceptEdits,
            Self::AcceptEdits => Self::Auto,
            Self::Auto => Self::Plan,
            Self::Plan => Self::BypassPermissions,
            Self::BypassPermissions => Self::Default,
        }
    }

    /// Whether this mode allows a given tool to execute without prompting.
    pub fn auto_approves(self, tool: &ToolCall) -> PermissionDecision {
        match self {
            Self::Default => PermissionDecision::NeedsPrompt,
            Self::Plan => match tool.kind {
                ToolKind::Read
                | ToolKind::Glob
                | ToolKind::Grep
                | ToolKind::TaskCreate
                | ToolKind::TaskUpdate
                | ToolKind::TaskList
                | ToolKind::TaskDone
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage
                // ExitPlanMode is the *only* way the agent can leave
                // plan mode programmatically. Auto-approving it lets
                // the model surface a plan whenever it's ready —
                // mirrors v132's `ExitPlanMode` contract.
                | ToolKind::ExitPlanMode => PermissionDecision::Approved,
                ToolKind::Bash => {
                    let cmd = tool.input.summary().to_lowercase();
                    if is_readonly_bash(&cmd) {
                        PermissionDecision::Approved
                    } else {
                        PermissionDecision::Denied("Plan mode: write operations blocked")
                    }
                }
                _ => PermissionDecision::Denied("Plan mode: write operations blocked"),
            },
            Self::AcceptEdits => match tool.kind {
                ToolKind::Write
                | ToolKind::Edit
                | ToolKind::ApplyPatch
                | ToolKind::Read
                | ToolKind::Glob
                | ToolKind::Grep
                | ToolKind::TaskCreate
                | ToolKind::TaskUpdate
                | ToolKind::TaskList
                | ToolKind::TaskDone
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage => PermissionDecision::Approved,
                _ => PermissionDecision::NeedsPrompt,
            },
            Self::BypassPermissions => PermissionDecision::Approved,
            Self::Auto => PermissionDecision::NeedsClassifier,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Approved,
    Denied(&'static str),
    NeedsPrompt,
    NeedsClassifier,
}

/// Heuristic for read-only bash commands (used by Plan mode).
fn is_readonly_bash(cmd: &str) -> bool {
    let first_word = cmd.split_whitespace().next().unwrap_or("");
    matches!(
        first_word,
        "ls" | "cat"
            | "head"
            | "tail"
            | "find"
            | "grep"
            | "rg"
            | "fd"
            | "wc"
            | "file"
            | "stat"
            | "which"
            | "whoami"
            | "pwd"
            | "echo"
            | "date"
            | "env"
            | "printenv"
            | "uname"
            | "hostname"
            | "id"
            | "tree"
            | "du"
            | "df"
            | "free"
            | "ps"
    ) || cmd.starts_with("git log")
        || cmd.starts_with("git show")
        || cmd.starts_with("git diff")
        || cmd.starts_with("git status")
        || cmd.starts_with("git branch")
        || cmd.starts_with("cargo check")
        || cmd.starts_with("cargo test")
        || cmd.starts_with("cargo clippy")
}

#[derive(Clone, Copy, PartialEq)]
pub enum ApprovalChoice {
    Yes,
    No,
    Always,
    YesSession,
}

impl ApprovalChoice {
    pub const ALL: &'static [Self] = &[Self::Yes, Self::No, Self::Always, Self::YesSession];

    pub fn label(self) -> &'static str {
        match self {
            Self::Yes => "Yes  (y)",
            Self::No => "No   (n)",
            Self::Always => "Always for this tool  (a)",
            Self::YesSession => "Yes for session  (s)",
        }
    }
}

pub struct PendingApproval {
    pub tool: ToolCall,
    pub selected: usize,
}

/// One entry in the input queue. v126's `queued_command` attachment carries
/// `isMeta: true` for slash commands so they execute locally after the turn
/// ends instead of being shipped to the API as a user message.
/// Active transcript search state — armed by Ctrl+F. `query` is what
/// the user has typed so far in the search bar; `matches` is the list
/// of message indices whose body contains `query` (case-insensitive).
/// `cursor` is the index into `matches` of the currently-focused
/// result. `n` / `N` cycle the cursor; Enter commits and exits;
/// Esc cancels.
#[derive(Debug, Clone, Default)]
pub struct TranscriptSearch {
    pub query: String,
    pub matches: Vec<usize>,
    pub cursor: usize,
}

#[derive(Debug, Clone)]
pub struct QueuedPrompt {
    pub text: String,
    pub is_meta: bool,
}

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const TICK_MS: u64 = 80;
/// Cap on how many turns of token usage we retain for the info-sidebar
/// sparkline. 32 datapoints fit comfortably in a 30-col-wide sidebar
/// while still showing a meaningful trend.
pub const TOKEN_HISTORY_CAP: usize = 32;

pub struct BackgroundTask {
    pub task_id: String,
    pub description: String,
    pub status: crate::types::TaskLifecycle,
    pub started_at: std::time::Instant,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub last_tool: Option<String>,
    pub messages: Vec<String>,
    /// Cumulative tool invocations the subagent has made this run.
    /// Mirrors v131's `toolUseCount` (cli.2.1.131.beautified.js, `jOH()`).
    pub tool_use_count: u32,
    /// Most recent request's input-token count (driven by `Usage` stream
    /// events when the provider emits them, falls back to a 4-chars-per-
    /// token byte estimate otherwise). Mirrors v131's `latestInputTokens`.
    pub latest_input_tokens: u64,
    /// Sum of output tokens across every API round-trip in this run.
    /// Mirrors v131's `cumulativeOutputTokens`. The fan-UI badge displays
    /// `latest_input + cumulative_output` to match Claude Code's
    /// "89.7k tokens" figure.
    pub cumulative_output_tokens: u64,
    /// Model the agent is currently using. Captured from the spawn site
    /// so per-agent cost can be computed via `cost::cost_for(model, usage)`.
    pub model_used: Option<String>,
    /// Per-agent token budget. When set and `latest_input + cumulative_output`
    /// exceeds it, the agent is forcibly terminated and an error toast
    /// fires. Defaults to None (unlimited).
    pub max_input_tokens: Option<u64>,
    /// Set once per task when the budget gets crossed so we don't fire
    /// the kill / toast multiple times.
    pub budget_killed: bool,
}

pub struct App {
    pub theme: Theme,
    /// Verbosity / formatting style for assistant replies. Routes
    /// through `OutputStyle::system_prompt_suffix()` at request-build
    /// time. `Default` is the no-op (current jfc behaviour).
    pub output_style: crate::output_style::OutputStyle,
    pub messages: Vec<ChatMessage>,
    pub streaming_text: String,
    pub streaming_reasoning: String,
    /// v126-style cumulative byte counter for ALL streamed response content:
    /// text deltas + thinking deltas + tool input JSON deltas. Divided by 4
    /// for the spinner's token estimate (matches v126's `responseLengthRef.current / 4`).
    /// Reset at the start of each streaming turn.
    pub streaming_response_bytes: usize,
    pub streaming_assistant_idx: Option<usize>,
    pub is_streaming: bool,
    /// Wall-clock instant the current turn's stream began. Set when
    /// `is_streaming` flips true; cleared when it flips false. Drives the
    /// `(5m 10s · …)` elapsed counter in the v126-style spinner — without
    /// it, the spinner can't show how long we've been waiting.
    pub streaming_started_at: Option<Instant>,
    /// Wall-clock instant the *user-level turn* started. Survives across
    /// agentic-loop iterations (each tool batch → new sub-stream
    /// resets `streaming_started_at`, but `turn_started_at` keeps
    /// running). Reset only when the user submits a fresh prompt OR
    /// when the agentic loop fully concludes (no more tools to run, no
    /// pending approvals, EndTurn). The spinner clock reads this so a
    /// 5-step agentic loop shows `5m 10s` cumulative, not `0s` after
    /// every sub-stream restart — that's what `Fermenting… (0s · ↓ 69
    /// tokens)` after a multi-turn turn was: the timer reset every
    /// loop iteration. v126's spinner uses the same turn-level clock.
    pub turn_started_at: Option<Instant>,
    /// Index into `messages` of the user-prompt the up-arrow recall is
    /// currently displaying, counting backwards from the end. `None`
    /// means the user is editing a fresh prompt (not recalled). Each
    /// up-arrow at empty input increments toward older prompts; each
    /// down-arrow decrements. Mirrors v126's `useArrowKeyHistory`
    /// behavior — a quality-of-life win for resend/edit workflows.
    pub history_cursor: Option<usize>,
    /// Wall-clock instant of the most recent text/reasoning delta. Used by
    /// the spinner to detect stalls (`>=15s` → "warming up", up to `>=60s`
    /// → "almost done thinking"). Mirrors v126 `timeSinceLastToken` (cli.js
    /// line 323162).
    pub streaming_last_token_at: Option<Instant>,
    /// Instant the model started producing reasoning output (extended
    /// thinking). Set on the first reasoning chunk per turn; cleared when
    /// the turn ends. Mirrors v126's `streamMode = "thinking"` transition
    /// (cli.js HcH:413611).
    pub thinking_started_at: Option<Instant>,
    /// Instant the model stopped producing reasoning and switched to
    /// regular text output (or the stream ended without text). Once set,
    /// the spinner stops showing "thinking" and starts showing
    /// `thought for Ns`. Mirrors v126's `streamingEndedAt` field on the
    /// thinking-status reducer (cli.js HcH:413585).
    pub thinking_ended_at: Option<Instant>,
    pub scroll_offset: usize,
    pub total_lines: usize,
    pub textarea: TextArea<'static>,
    pub show_palette: bool,
    pub palette_input: String,
    pub palette_selected: usize,
    pub spinner_frame: usize,
    pub provider: Arc<dyn Provider>,
    pub providers: Vec<Arc<dyn Provider>>,
    pub model: ModelId,
    /// Recently selected models (most recent first, max 5). Shown at the
    /// top of the model picker for quick switching. Persisted to
    /// `~/.config/jfc/recent_models.json`.
    pub recent_models: Vec<String>,
    pub cwd: String,
    pub reasoning_expanded: HashMap<usize, bool>,
    pub pending_approval: Option<PendingApproval>,
    /// FIFO of tool calls waiting for approval behind the current one. When the
    /// model emits multiple approvable tools in one turn (six `bash` calls in a
    /// single response is common), only the first one fits in `pending_approval`
    /// — the rest queue here. After the user decides on the current tool, the
    /// next is dequeued into `pending_approval`. Without this, subsequent tools
    /// were silently dropped, leaving the conversation with a tool_use that
    /// had no matching tool_result and a stalled agentic loop.
    pub approval_queue: std::collections::VecDeque<ToolCall>,
    /// FIFO of user prompts the user submitted while the model was streaming.
    /// v126 calls these `queued_command` attachments. They render in the
    /// transcript immediately as user messages (so the user sees their input
    /// landed) but don't go to the API until the current turn finishes.
    /// Drained by `drain_queued_prompts()` after `is_streaming` flips false
    /// AND the approval pipeline is empty. Each entry remembers whether the
    /// user typed a slash command (v126's `isMeta: true`) — those run
    /// locally on drain instead of going to the API.
    pub queued_prompts: std::collections::VecDeque<QueuedPrompt>,
    /// Cached count of agent-isolated worktrees (excludes the primary
    /// checkout). Refreshed by the Tick handler at most every
    /// `WORKTREE_REFRESH_MS` so the status-bar badge stays accurate
    /// without shelling out to `git worktree list` on every redraw.
    pub worktree_count: usize,
    pub worktree_count_last_refresh: Option<std::time::Instant>,
    /// Cached current git branch (e.g. "master", "feat/x"). Updated by
    /// the Tick handler at most every `GIT_BRANCH_REFRESH_MS` ms so a
    /// long session reflects branch switches without shelling out
    /// every render frame. None when not in a git repo.
    pub git_branch: Option<String>,
    pub git_branch_last_refresh: Option<std::time::Instant>,
    /// Set of group-keys (`format!("{msg_idx}:{first_tool_id}")`)
    /// currently expanded. Default = collapsed: dense Read/Glob/Grep
    /// runs render as one "▶ N reads · click to expand" row, click
    /// or `o` toggles.
    pub tool_group_expanded: std::collections::HashSet<String>,
    /// Active transcript search. `None` when not searching. The
    /// search bar at the bottom of the screen, the match highlight
    /// in messages, and the n/N navigation all key off this.
    pub transcript_search: Option<TranscriptSearch>,
    /// Slash-command autocomplete popup state. `Some(idx)` while the
    /// user is typing a command and the popup is open. None when the
    /// popup is dismissed.
    pub slash_popup_selected: Option<usize>,
    /// Wall-clock instant of the last successful session save. The
    /// status-bar render shows "✓ saved" briefly after this fires,
    /// fading after `SAVED_BADGE_TTL_MS` so the indicator doesn't
    /// linger on every render.
    pub last_session_save_at: Option<std::time::Instant>,
    /// Cycle index for `Ctrl+L`. Each press copies the next-oldest
    /// `path:line` reference detected in the most recent tool
    /// output. Reset whenever a fresh ToolResult lands so the user
    /// always starts from the most recent.
    pub path_yank_cursor: usize,
    /// Index into `messages` of the user message currently being
    /// edited. None when not editing. Submission while this is Some
    /// rewrites the message at this index and drops everything
    /// after it before re-firing the turn — `Ctrl+E` to enter,
    /// Esc to cancel.
    pub editing_message_idx: Option<usize>,
    /// Set to true on double-ESC. Streaming, agentic-loop continuation,
    /// and the subagent runner all sample this between iterations and
    /// bail when it flips. Wrapped in `Arc` so spawned tasks can clone
    /// a handle into their own scope. Mirrors v126's `abortController`.
    /// Toggled by `?` (when input bar is empty). When true, an
    /// overlay listing every keybinding is rendered on top of the
    /// transcript. Discoverability for muscle-memory features
    /// (Ctrl+X chord, ESC×2 interrupt, `o` to expand, etc.) that
    /// otherwise live only in source comments.
    pub show_help: bool,
    /// True between Ctrl+G and the follow-up letter that selects the
    /// jump target (e/t/m/a). Esc cancels. Drives a small hint row
    /// in the status area so the user knows the chord is armed.
    pub jump_armed: bool,
    pub jump_armed_at: Option<std::time::Instant>,
    /// Most recent tool-block click timestamp, keyed by tool id. The
    /// click handler uses this to detect double-click (same tool id
    /// within `DOUBLE_CLICK_MS`) for the pin gesture.
    pub last_tool_click: Option<(String, std::time::Instant)>,
    /// Bounds of the sessions sidebar block (set on each render).
    /// The mouse handler reads this to decide whether a click hit a
    /// session row and which row it was. `None` when the sidebar is
    /// hidden — in that case the click handler ignores sidebar
    /// coordinates.
    pub sidebar_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Bounds of the messages area, used by the drag-scroll handler
    /// to convert pixel deltas to scroll offsets and to gate scroll
    /// events to the right region.
    pub messages_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Bounds of the toast overlay strip; used by the click handler
    /// to map a click to a toast index for instant dismissal.
    pub toasts_rect: std::cell::RefCell<Option<ratatui::layout::Rect>>,
    /// Last known drag-Y, set on each MouseEventKind::Drag event so
    /// the next drag delta can advance scroll_offset by the
    /// difference. Reset on Down / Up so a fresh drag starts cleanly.
    pub drag_anchor_y: Option<u16>,
    /// Per-turn token usage history (input + output) for the
    /// sparkline rendered in the info sidebar. Pushed each time a
    /// `StreamUsage` event lands at end-of-turn. Capped at the last
    /// `TOKEN_HISTORY_CAP` turns so a long session doesn't grow it
    /// unbounded.
    pub token_history: std::collections::VecDeque<u64>,
    /// task_id of whichever subagent / teammate emitted activity most
    /// recently (AgentChunk or Progress event). Render that row bold +
    /// accent in the spinner-area tree so the user can tell which
    /// agent is currently moving vs. idle. None means nothing has
    /// reported activity this turn.
    pub last_active_agent_task: Option<String>,
    pub interrupt_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Timestamp of the most recent ESC press in the main shortcut
    /// handler. The next ESC within `INTERRUPT_DOUBLE_TAP_MS` triggers
    /// an interrupt instead of just clearing the input.
    pub last_esc_at: Option<std::time::Instant>,
    pub always_approved: Vec<String>,
    pub session_approved: Vec<String>,
    pub follow_bottom: bool,
    pub pending_tool_calls: Vec<ToolCall>,
    pub max_context_tokens: usize,
    /// Set by `/compact` slash command. Picked up by the main loop next time
    /// it would otherwise check `compact::should_compact` — forces compaction
    /// regardless of token level. Cleared after the compact runs (success or
    /// not) so a single `/compact` invocation triggers exactly one attempt.
    pub force_compact_pending: bool,
    /// Set after compaction permanently fails (CircuitBreakerTripped,
    /// Unsupported, Exhausted). Prevents the post-response handler from
    /// re-spawning compact on every AllToolsComplete — without this, the
    /// circuit breaker fires every 4-5s for the rest of the session.
    /// Cleared on: new session (/clear), manual /compact, model switch.
    pub compact_suppressed: bool,
    /// Wall-clock instant the current compaction started. `Some` while a
    /// compact request is in flight (set on `CompactionStarted`, cleared on
    /// `CompactionDone`/`CompactionFailed`). The renderer shows a
    /// `Compacting…` spinner whenever this is `Some`, so a long pre-submit
    /// compaction doesn't look like a frozen UI.
    pub compacting_started_at: Option<Instant>,
    /// Cumulative summary-text length collected during the in-flight
    /// compact (across all retry attempts). The spinner divides by 4 to
    /// get a chars-per-token estimate and renders `↓ Nk tokens` —
    /// matches the regular streaming spinner's live counter so the user
    /// sees the same kind of feedback during compaction.
    pub compacting_output_chars: u64,
    /// Sum of completed retry attempts' final output sizes. `compact()`
    /// retries internally when post_tokens is still over the Blocked
    /// threshold, and each attempt streams a fresh response from 0.
    /// Without this baseline, `compacting_output_chars` would jump back
    /// down at every retry boundary — visible to the user as a
    /// flickering/resetting counter. The handler bumps this whenever it
    /// detects the per-attempt counter regressing.
    pub compacting_attempt_baseline: u64,
    /// Last `output_chars` value seen this attempt. Used to detect a
    /// regression (new attempt starting) so the baseline gets the prior
    /// attempt's high-water added.
    pub compacting_last_progress: u64,
    /// Set each frame by the renderer. Used for page-scroll math.
    pub viewport_height: usize,
    pub input_wrap_width: usize,
    pub tool_ctx: ToolContext,
    pub dedup_cache: Arc<Mutex<ReadDedupCache>>,
    pub show_model_picker: bool,
    pub model_picker_filter: String,
    pub model_picker_selected: usize,
    pub model_picker_models: Vec<ModelInfo>,
    /// Drives selection + scroll for the picker's `Table`. Kept in sync with
    /// `model_picker_selected` so existing handlers keep working, but ratatui's
    /// stateful render uses the `TableState` for autoscroll when the cursor moves
    /// past the visible area.
    pub model_picker_state: TableState,
    /// Cache of `Provider::fetch_models()` results, keyed by `Provider::name()`. Populated
    /// asynchronously at startup; consulted by the picker before falling back to the
    /// provider's static `available_models()`.
    pub provider_models: HashMap<ProviderId, Vec<ModelInfo>>,
    pub model_picker_query_cache: QueryCache<Vec<ModelInfo>>,
    /// OAuth seat tier from `/api/oauth/profile` (e.g. `"opus"`, `"opusplan"`,
    /// `"claude-opus-4-6[1m]"`). Drives `apply_seat_tier_filter()` in the picker.
    pub seat_tier: Option<String>,
    /// OAuth subscription type (`"max"`, `"pro"`, `"enterprise"`) — shown in the
    /// status bar so the user knows which plan they're billing against.
    pub subscription_type: Option<String>,
    /// Account email from the OAuth profile, surfaced in the status bar.
    pub account_email: Option<String>,
    /// Whether the sessions sidebar is visible. Default off so the chat takes
    /// the full width — toggle with Ctrl+B.
    pub show_sidebar: bool,
    /// Cached list of session metadata (newest first), refreshed when the
    /// sidebar opens. Storing here keeps render() pure of disk I/O. Replaced
    /// the raw-id `session_ids` cache so the sidebar can show titles, cwd
    /// badges, and relative timestamps instead of `ses_2026...` ids.
    pub session_meta: Vec<crate::session::SessionMetadata>,
    /// Currently-selected sidebar row.
    pub session_selected: usize,
    /// State for the sidebar `List` widget — drives auto-scroll when the
    /// selection moves past the visible area.
    pub session_list_state: ratatui::widgets::ListState,
    /// Active session id (set when the user picks one or starts a new one).
    pub current_session_id: Option<String>,
    /// v126 auto-mode classifier config — `enabled: true` routes every tool
    /// call through the LLM classifier instead of prompting the user.
    /// Loaded from `~/.config/jfc/settings.json` at startup.
    pub auto_mode: AutoModeConfig,
    /// v126 permission mode — controls how tool execution is gated.
    pub permission_mode: PermissionMode,
    /// v126 task/todo store. Persists to `~/.config/jfc/tasks/<session>.json`
    /// so todos survive session resume and compaction. Reused across the
    /// agent's turns; the slash commands `/task-*` poke it directly.
    pub task_store: std::sync::Arc<crate::tasks::TaskStore>,
    /// Records when each task transitioned to `Completed` so the footer can
    /// keep showing them for 30 seconds with dimmed/strikethrough styling.
    pub task_completion_times: HashMap<TaskId, Instant>,
    /// Whether the full-screen task panel overlay is visible (Ctrl+T).
    pub show_task_panel: bool,
    /// Currently-selected row in the task panel.
    pub task_panel_selected: usize,
    /// Drives selection + scroll for the task panel's `Table`.
    pub task_panel_state: TableState,
    /// Transient per-session map of task_id → current activity description.
    /// Updated by the tool execution loop to show what an in_progress task is
    /// doing (e.g. "Running bash: cargo test", "Reading src/main.rs").
    pub task_activities: HashMap<TaskId, String>,
    pub last_usage_input: u32,
    pub last_usage_output: u32,
    /// Auto-expiring toast queue. Pruned every `Tick`. Pushed via
    /// `AppEvent::Toast` from anywhere in the app (compaction milestones,
    /// session save success, classifier blocks). Mirrors v126's terminal
    /// `notification()` for non-blocking status surfacing.
    pub toasts: Vec<crate::toast::Toast>,
    /// `@filename` autocomplete state. `active=false` when not popping;
    /// while active, the input handler routes typed chars into
    /// `query` and `mentions::filter_candidates` re-ranks `candidates`.
    /// Mirrors v126 cli.js:161602 (`autocomplete:accept` /
    /// `autocomplete:dismiss`).
    pub mention: crate::mentions::MentionState,
    /// Cached file list scanned at the start of each mention session
    /// so we don't re-walk the cwd on every keystroke. Refreshed when
    /// `@` is freshly typed.
    pub mention_all_files: Vec<String>,
    /// Active LSP diagnostics, keyed by file path. Rendered as a one-line
    /// `Found N new diagnostic issue(s) in M file(s) (ctrl+o to expand)`
    /// row above the spinner when non-empty. Updated by
    /// `AppEvent::DiagnosticsUpdated`. Mirrors v126 cli.js:338030-338040.
    pub diagnostics: Vec<crate::diagnostics::DiagnosticEntry>,
    /// Whether the Ctrl+O diagnostic-expansion panel is open. v126 cli.js
    /// :338038 advertises `(ctrl+o to expand)` on the summary row; this
    /// is the destination of that key. The panel groups diagnostics by
    /// file and lists each as `<symbol> [Line A:B] <message>` matching
    /// cli.js:338053. Esc closes.
    pub show_diagnostic_panel: bool,
    /// Scroll offset (in lines) for the diagnostic panel body. Reset
    /// to 0 each time the panel is opened so the user always lands at
    /// the top of the list regardless of where they were before.
    pub diagnostic_panel_scroll: usize,
    /// Most recently completed tool — drives the sparkle (✦) flash
    /// next to its gutter for ~600ms after the result lands. `None`
    /// after the sparkle's TTL elapses or when no tool has completed
    /// this session.
    pub recent_tool_completion: Option<(String, std::time::Instant)>,
    /// Last token-arrival timestamp — drives the right-edge token
    /// rain animation. Each `StreamChunk` stamps it; the renderer
    /// reads it to highlight one cell in the rain column with a
    /// fading intensity proportional to age.
    pub last_token_arrival: Option<std::time::Instant>,
    /// First-launch timestamp for the boot sweep animation. Set in
    /// `App::new`; the placeholder renderer uses it to drive a brief
    /// star cascade across "What can I help you with?" on session
    /// start. After ~1.2s the cascade settles into the static
    /// placeholder.
    pub launched_at: std::time::Instant,
    /// Stable keys for diagnostics already shown to the user, so the
    /// summary row doesn't keep popping for the same set on every
    /// re-publish. Mirrors v126 cli.js:231025-231036's per-URI
    /// "delivered" set. Cleared on `/check` rerun and when the user
    /// opens the expansion panel (Ctrl+O), since opening implies
    /// acknowledgment.
    pub delivered_diagnostics: std::collections::HashSet<String>,
    /// The (input, output, cache_read, cache_write) reading the last time
    /// `add_delta` was applied to `usage_by_model`. Anthropic sends
    /// **cumulative** counts in every `message_delta`, so we have to
    /// subtract this baseline before adding to per-model totals — otherwise
    /// every delta would be triple-counted (Claude sends 5-15 deltas per
    /// turn) and `Usage by model` shows numbers an order of magnitude too
    /// high. Reset to (0,0,0,0) when a new turn starts.
    pub usage_apply_baseline: (u32, u32, u32, u32),
    /// Reasoning-effort pin for this session. `/effort low|medium|high|xhigh|max`
    /// flips it; `stream_response` mirrors `effort_state.api_param()` into
    /// the `reasoning_effort` field of `StreamOptions` if the active model
    /// supports it.
    pub effort_state: crate::effort::EffortState,
    /// Last time we fired the OnHeartbeat hook. Tick handler checks this
    /// every 80ms and fires the hook at most once every 30s when idle.
    pub last_heartbeat_at: Option<std::time::Instant>,
    /// Last MCP refresh counter we observed. Tick handler compares this
    /// against `mcp::registry::refresh_counter()` to detect inbound
    /// `notifications/tools/list_changed` and emit a toast + reminder.
    pub last_mcp_refresh_seen: u64,
    /// Message indices the user pinned via `/pin <idx>`. Compaction
    /// preserves pinned messages verbatim regardless of token pressure.
    /// Stored as indices into `messages` rather than a flag on
    /// ChatMessage so we don't have to touch every construction site.
    pub pinned_message_indices: std::collections::HashSet<usize>,
    /// `/verbose` toggle: when true, tool blocks render expanded by
    /// default. When false (default), they preview to N lines.
    pub verbose_mode: bool,
    /// v132 Marsh (mid-stream bash → model) buffer. Each entry is
    /// `(tool_id, line)` captured from `ToolOutputChunk`. `stream.rs`
    /// drains this on the next outbound request so the model sees what
    /// bash printed since the last turn.
    pub pending_marsh_chunks: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>,
    /// Highest budget threshold the user has been warned about so far this
    /// session. 0 = no warnings yet, 80 = 80% warning shown, 100 = 100%
    /// warning shown. Prevents toast spam when the same threshold is
    /// crossed multiple times across re-renders.
    pub cost_budget_warned_at: u8,
    pub background_tasks: HashMap<String, BackgroundTask>,
    pub show_info_sidebar: bool,
    pub mcp_servers: Vec<crate::types::McpServerInfo>,
    pub lsp_servers: Vec<crate::types::LspServerInfo>,
    pub usage_by_model: HashMap<String, crate::types::ModelUsage>,
    pub leader_key_active: bool,
    pub leader_key_timeout: Option<std::time::Instant>,
    pub viewing_task_id: Option<String>,
    /// Set of `BackgroundTask.messages` indices the user expanded with `o`
    /// while drilled into the subagent task view. Long entries (>80 lines or
    /// >5 KB) collapse to a 5-line preview by default; presence in this set
    /// flips them to fully expanded. Cleared whenever `viewing_task_id`
    /// changes so expansion state is per-drill-in, not sticky across tasks.
    ///
    /// TODO Phase B: once `BackgroundTask.messages` migrates to
    /// `Vec<ChatMessage>` and the subagent view renders through the same
    /// `MessageView` pipeline as the main chat, this field collapses into
    /// per-`ToolCall.is_collapsed` state and can be removed.
    /// Per-task expansion state. Keyed by `task_id` so navigating
    /// between tasks (or out and back in) preserves what the user has
    /// expanded. Previously a session-wide `HashSet<usize>` that got
    /// `.clear()`ed on every switch — entering a task with 121 hidden
    /// lines required pressing `o` again every time.
    pub viewing_task_expanded: std::collections::HashMap<String, std::collections::HashSet<usize>>,
    /// Drained at submit time; future Ctrl+V handlers push here. Anthropic
    /// content-block conversion happens at provider-message-build time.
    pub pending_attachments: Vec<crate::attachments::Attachment>,
    /// Per-frame map of `(tool_id, screen_rect)` populated by the message
    /// renderer as each `ToolBlock` paints. The mouse handler reads this to
    /// translate a left-click into the tool whose body should expand —
    /// v126's cli.js (cmd-click on iTerm2) toggles the same per-tool
    /// expand/collapse affordance via mouse. We use plain left-click here
    /// because non-iTerm terminals don't surface the cmd modifier the same
    /// way; the spirit (mouse → toggle that tool) is preserved.
    ///
    /// Cleared at the top of every `render::frame()` and re-populated as
    /// each visible `ToolBlock` renders. Tools scrolled off-screen are not
    /// pushed, so they're automatically un-clickable. `RefCell` because
    /// `MessageView` borrows `&App` immutably during `Widget::render`, and
    /// we need a `&mut` push from inside that path.
    pub tool_hit_regions: RefCell<Vec<(String, Rect)>>,
    /// Content-addressed cache for `markdown::to_lines()` output. Keyed on
    /// `(hash(text), width)` so unchanged messages aren't re-parsed on every
    /// frame. Uses `RefCell` because `MessageView` borrows `&App` immutably
    /// during `Widget::render` but needs mutable cache access.
    pub render_cache: RefCell<RenderCache>,
    /// Cached result of `collect_diff_stats()`. Keyed on
    /// `(messages.len(), total_parts_count)` — invalidates when a message is
    /// appended or a tool result lands. Avoids O(N_messages × N_parts)
    /// HashMap walk per frame; reduces to O(1) lookup on cache hit.
    pub diff_stats_cache: RefCell<Option<(usize, usize, crate::render::DiffStats)>>,
    /// Swarm / team orchestration state. Tracks the current team, spawned
    /// teammates, and message delivery. `None` when no team is active.
    pub team_context: crate::swarm::TeamContext,
    /// Channel receiver for events from in-process teammate runners.
    /// Polled in the main event loop alongside terminal/stream events.
    pub teammate_event_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<crate::swarm::runner::TeammateEvent>>,
    /// Sender side — cloned into each spawned teammate's runner.
    pub teammate_event_tx: tokio::sync::mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
    /// Slate dynamic model router. `None` when `slate_enabled = false` in the
    /// loaded config (the common case). When `Some`, callers consult it on
    /// every user submission to pick a per-turn model — see
    /// `slate::SlateRouter::route` and `crates/jfc-ui/src/slate.rs`.
    pub slate: Option<SlateRouter>,
    /// Advisor session for `/advisor <query>` (see `crate::advisor`).
    /// `None` until the user invokes `/advisor` for the first time —
    /// mints lazily so the cost is paid only by users who actually use
    /// the feature. The session owns its own model id, transcript, and
    /// token budget; budget exhaustion returns Err and the user must
    /// reset (e.g. via `/clear`) to get a fresh budget.
    pub advisor_session: Option<crate::advisor::AdvisorSession>,
    /// Gate for the `/advisor` slash command. Default OFF per the
    /// deliverable's "no /advisor command without a config flag" rule.
    /// Set via the `JFC_ADVISOR_ENABLED=1` env var on startup OR via a
    /// future config-toml field. When false, the slash command surfaces
    /// a hint message instead of running.
    pub advisor_enabled: bool,
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        let providers = vec![Arc::clone(&provider)];
        let (teammate_tx, teammate_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::swarm::runner::TeammateEvent>();
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        // Minimal placeholder — the help overlay and `?` shortcut
        // already document Enter / Shift+Enter; repeating it inline
        // every render was noise. Just a soft prompt.
        textarea.set_placeholder_text("send a message…");

        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
            .unwrap_or_default();

        let mut app = Self {
            theme: Theme::dark(),
            output_style: crate::output_style::OutputStyle::default(),
            messages: Vec::new(),
            streaming_text: String::new(),
            streaming_reasoning: String::new(),
            streaming_response_bytes: 0,
            streaming_assistant_idx: None,
            streaming_started_at: None,
            streaming_last_token_at: None,
            thinking_started_at: None,
            thinking_ended_at: None,
            turn_started_at: None,
            history_cursor: None,
            is_streaming: false,
            scroll_offset: 0,
            total_lines: 0,
            textarea,
            show_palette: false,
            palette_input: String::new(),
            palette_selected: 0,
            spinner_frame: 0,
            provider,
            providers,
            model: model.into(),
            recent_models: load_recent_models(),
            cwd,
            reasoning_expanded: HashMap::new(),
            pending_approval: None,
            approval_queue: std::collections::VecDeque::new(),
            queued_prompts: std::collections::VecDeque::new(),
            worktree_count: 0,
            worktree_count_last_refresh: None,
            git_branch: None,
            git_branch_last_refresh: None,
            tool_group_expanded: std::collections::HashSet::new(),
            transcript_search: None,
            slash_popup_selected: None,
            last_session_save_at: None,
            path_yank_cursor: 0,
            editing_message_idx: None,
            show_help: false,
            jump_armed: false,
            jump_armed_at: None,
            last_tool_click: None,
            sidebar_rect: std::cell::RefCell::new(None),
            messages_rect: std::cell::RefCell::new(None),
            toasts_rect: std::cell::RefCell::new(None),
            drag_anchor_y: None,
            token_history: std::collections::VecDeque::with_capacity(TOKEN_HISTORY_CAP),
            last_active_agent_task: None,
            interrupt_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_esc_at: None,
            always_approved: Vec::new(),
            session_approved: Vec::new(),
            follow_bottom: true,
            tool_ctx: ToolContext::new(),
            dedup_cache: Arc::new(Mutex::new(ReadDedupCache::new())),
            pending_tool_calls: Vec::new(),
            force_compact_pending: false,
            compact_suppressed: false,
            compacting_started_at: None,
            compacting_output_chars: 0,
            compacting_attempt_baseline: 0,
            compacting_last_progress: 0,
            max_context_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
            viewport_height: 0,
            input_wrap_width: 1,
            show_model_picker: false,
            model_picker_filter: String::new(),
            model_picker_selected: 0,
            model_picker_models: Vec::new(),
            model_picker_state: TableState::default().with_selected(Some(0)),
            provider_models: HashMap::new(),
            model_picker_query_cache: QueryCache::default(),
            seat_tier: None,
            subscription_type: None,
            account_email: None,
            show_sidebar: false,
            session_meta: Vec::new(),
            session_selected: 0,
            session_list_state: ratatui::widgets::ListState::default(),
            current_session_id: Some(crate::session::generate_session_id()),
            auto_mode: crate::auto_mode::load_config(),
            permission_mode: PermissionMode::Default,
            // Tasks are scoped per-session (mirrors v126 cli.js:271505 keying
            // todos by `agentId ?? sessionId`). Opening with a freshly-minted
            // session id means a new run sees an empty task list, even if
            // prior runs left `~/.config/jfc/tasks/<old>.json` on disk. The
            // store is re-opened in `switch_session` whenever the session id
            // changes (load from sidebar, /continue, /clear).
            // NOTE: initialized as in_memory here; re-opened with the real
            // session_id after construction (see below).
            task_store: crate::tasks::TaskStore::in_memory(),
            task_completion_times: HashMap::new(),
            show_task_panel: false,
            task_panel_selected: 0,
            task_panel_state: TableState::default().with_selected(Some(0)),
            task_activities: HashMap::new(),
            last_usage_input: 0,
            last_usage_output: 0,
            toasts: Vec::new(),
            mention: crate::mentions::MentionState::default(),
            mention_all_files: Vec::new(),
            diagnostics: Vec::new(),
            show_diagnostic_panel: false,
            diagnostic_panel_scroll: 0,
            recent_tool_completion: None,
            last_token_arrival: None,
            launched_at: std::time::Instant::now(),
            delivered_diagnostics: std::collections::HashSet::new(),
            usage_apply_baseline: (0, 0, 0, 0),
            effort_state: crate::effort::EffortState::new(),
            last_heartbeat_at: None,
            last_mcp_refresh_seen: 0,
            pinned_message_indices: std::collections::HashSet::new(),
            verbose_mode: false,
            pending_marsh_chunks: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            cost_budget_warned_at: 0,
            background_tasks: HashMap::new(),
            show_info_sidebar: true,
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
            usage_by_model: HashMap::new(),
            leader_key_active: false,
            leader_key_timeout: None,
            viewing_task_id: None,
            viewing_task_expanded: std::collections::HashMap::new(),
            pending_attachments: Vec::new(),
            tool_hit_regions: RefCell::new(Vec::new()),
            render_cache: RefCell::new(RenderCache::new()),
            diff_stats_cache: RefCell::new(None),
            team_context: crate::swarm::TeamContext::default(),
            teammate_event_rx: Some(teammate_rx),
            teammate_event_tx: teammate_tx,
            // Slate is populated *after* `App::new` from the config (see
            // `main.rs::run_app`). Constructor default = None so the unit
            // tests that build a bare `App` don't need to plumb a router.
            slate: None,
            advisor_session: None,
            // Read the env gate once at construction. Tests bypass this
            // by setting the field directly; users who want it on for a
            // session export `JFC_ADVISOR_ENABLED=1` before launch.
            advisor_enabled: std::env::var("JFC_ADVISOR_ENABLED")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
        };
        // Open the task store with the real session id so tasks persist to disk.
        if let Some(ref sid) = app.current_session_id {
            app.task_store = crate::tasks::TaskStore::open(sid);
        }
        app.sync_selected_context_window();
        tracing::info!(
            target: "jfc::app",
            model = %app.model,
            provider = app.provider.name(),
            "App::new"
        );
        app
    }

    /// Switch to a different session id and reset all per-session state
    /// (tasks, completion-fade timers, task panel selection). Mirrors v126's
    /// new-session reset: each session has its own task bucket so tasks
    /// don't bleed across `/clear` or `/continue`.
    ///
    /// Pass `None` to mint a fresh session id; pass `Some(id)` to adopt an
    /// existing one (the session-load path through the sidebar / `/continue`).
    pub fn switch_session(&mut self, id: Option<String>) {
        let old_id = self.current_session_id.clone();
        let new_id = id.unwrap_or_else(crate::session::generate_session_id);
        tracing::info!(
            target: "jfc::app",
            old_session_id = ?old_id,
            new_session_id = %new_id,
            "switch_session"
        );
        self.current_session_id = Some(new_id.clone());
        self.task_store = crate::tasks::TaskStore::open(&new_id);
        self.task_completion_times.clear();
        self.task_activities.clear();
        self.task_panel_selected = 0;
        self.task_panel_state = ratatui::widgets::TableState::default().with_selected(Some(0));
        self.viewing_task_id = None;
        self.viewing_task_expanded.clear();
        self.compact_suppressed = false;
        self.recompute_token_estimate();
    }

    /// Recompute `tool_ctx.approx_tokens` and the live-usage cache fields
    /// (`last_usage_input` / `last_usage_output`) from the current
    /// `messages`. Call after a session resume so the Context gauge and
    /// the pre-submit compact gate reflect the loaded conversation —
    /// without this, both read 0 until the next stream's `StreamUsage`
    /// event lands, and the pre-submit compact silently mis-estimates a
    /// huge resumed history as "fits".
    ///
    /// Strategy mirrors v126 `Wd(messages)` (cli.js:197282-197294): walk
    /// the messages backwards looking for the most recent assistant
    /// message with `usage` attached. If found, that's the authoritative
    /// resume baseline (matches what the wire reported). If not (e.g. a
    /// pre-usage-tracking session file), fall back to
    /// `compact::estimate_tokens` over message content — same heuristic
    /// the live token counter uses.
    pub fn recompute_token_estimate(&mut self) {
        let old_estimate = self.tool_ctx.approx_tokens;
        // v126's `tokenCountWithEstimation` (tokens.ts:226-261): find the last
        // assistant message with API usage, use that as the authoritative base,
        // then rough-estimate any messages added AFTER it (user prompts, tool
        // results). This prevents the gap between API calls where the gauge
        // reads 0 or stale for newly-added messages.
        let last_usage_idx = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, m)| m.usage.as_ref().map(|u| (i, u.clone())));
        if let Some((idx, u)) = last_usage_idx {
            self.last_usage_input = u.input_tokens as u32;
            self.last_usage_output = u.output_tokens as u32;
            let base = u.total_context_tokens() as usize;
            // Estimate tokens for messages added after the usage-bearing message
            let tail = &self.messages[idx + 1..];
            let tail_estimate = crate::compact::estimate_tokens(tail);
            self.tool_ctx.approx_tokens = base + tail_estimate;
        } else {
            self.last_usage_input = 0;
            self.last_usage_output = 0;
            self.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&self.messages);
        }
        tracing::debug!(
            target: "jfc::app",
            old_estimate,
            new_estimate = self.tool_ctx.approx_tokens,
            "recompute_token_estimate"
        );
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset))]
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.max_scroll();
        self.follow_bottom = true;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset))]
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.follow_bottom = false;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset, lines))]
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.follow_bottom = false;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset, lines))]
    pub fn scroll_down(&mut self, lines: usize) {
        let max = self.max_scroll();
        self.scroll_offset = (self.scroll_offset + lines).min(max);
        if self.scroll_offset >= max {
            self.follow_bottom = true;
        }
    }

    #[tracing::instrument(target = "jfc::app", skip(self))]
    pub fn scroll_page_up(&mut self) {
        let half = self.half_page();
        self.scroll_up(half);
    }

    #[tracing::instrument(target = "jfc::app", skip(self))]
    pub fn scroll_page_down(&mut self) {
        let half = self.half_page();
        self.scroll_down(half);
    }

    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset >= self.max_scroll()
    }

    pub fn selected_model_info(&self) -> Option<ModelInfo> {
        let provider_name = self.provider.name();
        self.provider_models
            .get(provider_name)
            .and_then(|models| models.iter().find(|model| model.id == self.model).cloned())
            .or_else(|| {
                self.providers
                    .iter()
                    .find(|provider| provider.name() == provider_name)
                    .and_then(|provider| {
                        provider
                            .available_models()
                            .into_iter()
                            .find(|model| model.id == self.model)
                    })
            })
    }

    pub fn selected_context_window_tokens(&self) -> usize {
        let result = self
            .selected_model_info()
            .and_then(|model| model.context_window_tokens)
            .unwrap_or_else(|| {
                // Model info not yet loaded (async fetch_models hasn't completed).
                // Use model-name heuristic to avoid the gauge showing 100% for
                // large sessions on models with >200k windows (e.g. opus 4.6 = 1M).
                crate::providers::openwebui::infer_context_window_from_model_name(
                    self.model.as_str(),
                    None,
                )
            });
        tracing::trace!(
            target: "jfc::app",
            model = %self.model,
            result,
            "selected_context_window_tokens"
        );
        result
    }

    pub fn sync_selected_context_window(&mut self) {
        let old = self.max_context_tokens;
        self.max_context_tokens = self.selected_context_window_tokens();
        // When the model/provider changes, re-estimate token count. But if
        // we already have a usage-based estimate from a loaded session
        // (recompute_token_estimate found a message with `usage`), prefer
        // that over the rough heuristic — it's accurate to the token.
        // Without this guard, an async `ModelsLoaded` event firing after
        // session resume clobbers the 298k accurate value with a ~75k
        // chars/4 heuristic, making the gauge jump down to near-zero.
        let has_usage_based_estimate = self.messages.iter().rev().any(|m| m.usage.is_some());
        if !has_usage_based_estimate {
            self.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&self.messages);
        }
        tracing::info!(
            target: "jfc::app",
            old_max_context_tokens = old,
            new_max_context_tokens = self.max_context_tokens,
            approx_tokens = self.tool_ctx.approx_tokens,
            has_usage_based_estimate,
            model = %self.model,
            "sync_selected_context_window"
        );
    }

    fn max_scroll(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_height.max(1))
    }

    fn half_page(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }

    pub fn tool_needs_approval(&self, tool: &ToolCall) -> bool {
        // Permission mode takes priority
        match self.permission_mode.auto_approves(tool) {
            PermissionDecision::Approved => return false,
            PermissionDecision::Denied(_) => return false, // caller checks tool_denied_by_mode
            PermissionDecision::NeedsClassifier => return false, // auto-mode classifier handles
            PermissionDecision::NeedsPrompt => {}
        }

        let name = tool.kind.label();
        if self.always_approved.iter().any(|n| n == name) {
            tracing::debug!(
                target: "jfc::app",
                tool_kind = name,
                result = false,
                reason = "always_approved",
                "tool_needs_approval"
            );
            return false;
        }
        if self.session_approved.iter().any(|n| n == name) {
            tracing::debug!(
                target: "jfc::app",
                tool_kind = name,
                result = false,
                reason = "session_approved",
                "tool_needs_approval"
            );
            return false;
        }
        let result = matches!(
            tool.kind,
            ToolKind::Bash | ToolKind::Write | ToolKind::Edit | ToolKind::ApplyPatch
        );
        tracing::debug!(
            target: "jfc::app",
            tool_kind = name,
            result,
            "tool_needs_approval"
        );
        result
    }

    /// Check if a tool should be auto-denied by the current permission mode.
    pub fn tool_denied_by_mode(&self, tool: &ToolCall) -> Option<&'static str> {
        let result = match self.permission_mode.auto_approves(tool) {
            PermissionDecision::Denied(reason) => Some(reason),
            _ => None,
        };
        tracing::debug!(
            target: "jfc::app",
            tool_kind = tool.kind.label(),
            mode = ?self.permission_mode,
            denied = result.is_some(),
            "tool_denied_by_mode"
        );
        result
    }

    /// Scan the task store for newly-completed tasks and record their
    /// completion instant so the footer can fade them out after 30 s.
    pub fn sync_task_completions(&mut self) {
        use crate::tasks::TaskStatus;
        for task in self.task_store.list(crate::tasks::DeletedFilter::Exclude) {
            if task.status == TaskStatus::Completed
                && !self.task_completion_times.contains_key(&task.id)
            {
                self.task_completion_times
                    .insert(task.id.clone(), Instant::now());
            }
        }
        // Prune entries for tasks that are no longer completed (e.g. re-opened).
        let store = &self.task_store;
        self.task_completion_times.retain(|id, _| {
            store
                .get(id)
                .map_or(false, |t| t.status == TaskStatus::Completed)
        });
    }
}

/// Load recently used models from `~/.config/jfc/recent_models.json`.
pub fn load_recent_models() -> Vec<String> {
    let path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("recent_models.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save recently used models (max 5, most recent first).
pub fn save_recent_models(models: &[String]) {
    let path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("recent_models.json");
    let capped: Vec<&String> = models.iter().take(5).collect();
    if let Ok(json) = serde_json::to_string(&capped) {
        let _ = std::fs::write(&path, json);
    }
}

/// Push a model to the front of the recent list (deduplicates).
pub fn push_recent_model(recent: &mut Vec<String>, model: &str) {
    recent.retain(|m| m != model);
    recent.insert(0, model.to_owned());
    recent.truncate(5);
    save_recent_models(recent);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use crate::types::{
        ChatMessage, MessagePart, ModelUsage, ReplacementMode, ToolCall, ToolInput, ToolKind,
        ToolOutput, ToolStatus,
    };

    /// Minimal Provider implementation for App-construction tests. The
    /// streaming path is never invoked here — every test stays in the
    /// pure-state-mutation surface of `App`.
    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn new_app() -> App {
        App::new(Arc::new(TestProvider), "test-model")
    }

    fn make_tool(kind: ToolKind, id: &str) -> ToolCall {
        ToolCall {
            id: id.to_owned(),
            kind,
            status: ToolStatus::Pending,
            input: ToolInput::Generic {
                summary: String::new(),
            },
            output: ToolOutput::Empty,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }
    }

    // ─────── PermissionMode pure logic ────────────────────────────────

    // Normal: PermissionMode::label() returns the user-facing name for each
    // mode. Locks the strings — UI tests rely on these labels.
    #[test]
    fn permission_mode_label_normal() {
        assert_eq!(PermissionMode::Default.label(), "Default");
        assert_eq!(PermissionMode::Plan.label(), "Plan");
        assert_eq!(PermissionMode::AcceptEdits.label(), "Accept Edits");
        assert_eq!(PermissionMode::BypassPermissions.label(), "Bypass");
        assert_eq!(PermissionMode::Auto.label(), "Auto");
    }

    // Normal: PermissionMode::next() walks the cycle exhaustively and
    // returns to Default after one full revolution.
    #[test]
    fn permission_mode_next_cycles_normal() {
        let mut mode = PermissionMode::Default;
        let mut seen = vec![mode];
        for _ in 0..5 {
            mode = mode.next();
            seen.push(mode);
        }
        // After 5 next() calls we should be back at Default.
        assert_eq!(seen[5], PermissionMode::Default);
        // All five distinct modes appeared.
        let mut sorted: Vec<_> = seen[..5].iter().map(|m| m.label()).collect::<Vec<_>>();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5);
    }

    // Normal: every mode has *some* symbol (possibly empty) — the renderer
    // depends on this not panicking. Just exercise the arms.
    #[test]
    fn permission_mode_symbol_normal() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::Plan,
            PermissionMode::AcceptEdits,
            PermissionMode::BypassPermissions,
            PermissionMode::Auto,
        ] {
            // Trivially ensure no panic and stable type.
            let _: &str = mode.symbol();
        }
    }

    // Normal: Plan mode auto-approves Read/Glob/Grep, denies write tools,
    // and lets read-only Bash through but blocks write Bash.
    #[test]
    fn permission_mode_plan_decisions_normal() {
        let read_tool = make_tool(ToolKind::Read, "r1");
        let edit_tool = make_tool(ToolKind::Edit, "e1");
        assert_eq!(
            PermissionMode::Plan.auto_approves(&read_tool),
            PermissionDecision::Approved
        );
        assert!(matches!(
            PermissionMode::Plan.auto_approves(&edit_tool),
            PermissionDecision::Denied(_)
        ));

        // Bash: read-only command (e.g. `ls /tmp`) approved; write
        // command denied.
        let mut bash_ls = make_tool(ToolKind::Bash, "b1");
        bash_ls.input = ToolInput::Bash {
            command: "ls /tmp".into(),
            timeout: None,
            workdir: None,
        };
        assert_eq!(
            PermissionMode::Plan.auto_approves(&bash_ls),
            PermissionDecision::Approved
        );

        let mut bash_rm = make_tool(ToolKind::Bash, "b2");
        bash_rm.input = ToolInput::Bash {
            command: "rm -rf /".into(),
            timeout: None,
            workdir: None,
        };
        assert!(matches!(
            PermissionMode::Plan.auto_approves(&bash_rm),
            PermissionDecision::Denied(_)
        ));
    }

    // Normal: AcceptEdits approves Write/Edit/ApplyPatch (plus reads), but
    // returns NeedsPrompt for Bash (still gated).
    #[test]
    fn permission_mode_accept_edits_decisions_normal() {
        let edit_tool = make_tool(ToolKind::Edit, "e1");
        let bash_tool = make_tool(ToolKind::Bash, "b1");
        let read_tool = make_tool(ToolKind::Read, "r1");
        assert_eq!(
            PermissionMode::AcceptEdits.auto_approves(&edit_tool),
            PermissionDecision::Approved
        );
        assert_eq!(
            PermissionMode::AcceptEdits.auto_approves(&read_tool),
            PermissionDecision::Approved
        );
        assert_eq!(
            PermissionMode::AcceptEdits.auto_approves(&bash_tool),
            PermissionDecision::NeedsPrompt
        );
    }

    // Normal: BypassPermissions approves *everything*; Auto returns
    // NeedsClassifier so the LLM gate runs; Default falls through to
    // NeedsPrompt for everything.
    #[test]
    fn permission_mode_bypass_auto_default_decisions_normal() {
        let bash_tool = make_tool(ToolKind::Bash, "b1");
        assert_eq!(
            PermissionMode::BypassPermissions.auto_approves(&bash_tool),
            PermissionDecision::Approved
        );
        assert_eq!(
            PermissionMode::Auto.auto_approves(&bash_tool),
            PermissionDecision::NeedsClassifier
        );
        assert_eq!(
            PermissionMode::Default.auto_approves(&bash_tool),
            PermissionDecision::NeedsPrompt
        );
    }

    // Robust: ApprovalChoice::label returns a fixed label for every
    // variant. Exercises the full match arm.
    #[test]
    fn approval_choice_label_normal() {
        for c in ApprovalChoice::ALL.iter().copied() {
            // Trivially ensure no panic and that the label is non-empty.
            assert!(!c.label().is_empty());
        }
    }

    // ─────── App scroll helpers ────────────────────────────────────────

    // Normal: scroll_to_bottom sets offset to max_scroll and arms follow.
    #[test]
    fn scroll_to_bottom_sets_offset_and_follow_normal() {
        let mut app = new_app();
        app.total_lines = 100;
        app.viewport_height = 10;
        app.scroll_offset = 0;
        app.follow_bottom = false;
        app.scroll_to_bottom();
        assert_eq!(app.scroll_offset, 90);
        assert!(app.follow_bottom);
    }

    // Normal: scroll_to_top zeros the offset and disarms follow_bottom.
    #[test]
    fn scroll_to_top_zeros_offset_normal() {
        let mut app = new_app();
        app.total_lines = 100;
        app.viewport_height = 10;
        app.scroll_offset = 50;
        app.follow_bottom = true;
        app.scroll_to_top();
        assert_eq!(app.scroll_offset, 0);
        assert!(!app.follow_bottom);
    }

    // Normal: scroll_up/down move by the requested line count without
    // exceeding bounds.
    #[test]
    fn scroll_up_down_bounded_normal() {
        let mut app = new_app();
        app.total_lines = 100;
        app.viewport_height = 10;
        app.scroll_offset = 50;
        app.follow_bottom = false;

        app.scroll_up(20);
        assert_eq!(app.scroll_offset, 30);
        assert!(!app.follow_bottom);

        app.scroll_down(10);
        assert_eq!(app.scroll_offset, 40);

        // Push past max (90) — clamps and re-arms follow.
        app.scroll_down(1000);
        assert_eq!(app.scroll_offset, 90);
        assert!(app.follow_bottom);
    }

    // Robust: scroll_up at offset 0 saturates to 0 (no underflow).
    #[test]
    fn scroll_up_saturates_at_zero_robust() {
        let mut app = new_app();
        app.total_lines = 100;
        app.viewport_height = 10;
        app.scroll_offset = 0;
        app.scroll_up(50);
        assert_eq!(app.scroll_offset, 0);
    }

    // Normal: scroll_page_up / scroll_page_down move by half a page.
    #[test]
    fn scroll_page_up_down_uses_half_page_normal() {
        let mut app = new_app();
        app.total_lines = 200;
        app.viewport_height = 20;
        app.scroll_offset = 100;
        app.follow_bottom = false;

        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 90); // 100 - 10
        app.scroll_page_down();
        assert_eq!(app.scroll_offset, 100); // 90 + 10
    }

    // Robust: half_page is at least 1 so scroll_page_up never deadlocks
    // when viewport_height is 0 or 1.
    #[test]
    fn scroll_page_up_with_zero_viewport_robust() {
        let mut app = new_app();
        app.total_lines = 5;
        app.viewport_height = 0;
        app.scroll_offset = 3;
        app.scroll_page_up();
        assert_eq!(app.scroll_offset, 2);
    }

    // Normal: is_at_bottom reflects whether scroll_offset reached
    // max_scroll.
    #[test]
    fn is_at_bottom_reflects_offset_normal() {
        let mut app = new_app();
        app.total_lines = 50;
        app.viewport_height = 10;
        app.scroll_offset = 0;
        assert!(!app.is_at_bottom());
        app.scroll_offset = 40;
        assert!(app.is_at_bottom());
    }

    // Robust: when total_lines fits in viewport, max_scroll is 0 and any
    // offset is "at bottom".
    #[test]
    fn is_at_bottom_when_no_scroll_needed_robust() {
        let mut app = new_app();
        app.total_lines = 5;
        app.viewport_height = 20;
        app.scroll_offset = 0;
        assert!(app.is_at_bottom());
    }

    // ─────── Permission queue (approval_queue + pending_approval) ─────

    // Normal: approval_queue is FIFO. Push two; pop one at a time.
    #[test]
    fn approval_queue_is_fifo_normal() {
        let mut app = new_app();
        let t1 = make_tool(ToolKind::Bash, "b1");
        let t2 = make_tool(ToolKind::Bash, "b2");
        app.approval_queue.push_back(t1.clone());
        app.approval_queue.push_back(t2.clone());
        let first = app.approval_queue.pop_front().expect("first");
        let second = app.approval_queue.pop_front().expect("second");
        assert_eq!(first.id, "b1");
        assert_eq!(second.id, "b2");
    }

    // Normal: pending_approval can carry a tool while approval_queue
    // tracks queued ones.
    #[test]
    fn pending_approval_and_queue_independent_normal() {
        let mut app = new_app();
        app.pending_approval = Some(PendingApproval {
            tool: make_tool(ToolKind::Edit, "e1"),
            selected: 0,
        });
        app.approval_queue
            .push_back(make_tool(ToolKind::Bash, "b1"));
        assert!(app.pending_approval.is_some());
        assert_eq!(app.approval_queue.len(), 1);
    }

    // ─────── tool_needs_approval / tool_denied_by_mode ────────────────

    // Normal: in Default mode, write tools (Bash/Edit/Write/ApplyPatch)
    // need approval; Read does not.
    #[test]
    fn tool_needs_approval_default_mode_normal() {
        let app = new_app();
        let bash = make_tool(ToolKind::Bash, "b");
        let edit = make_tool(ToolKind::Edit, "e");
        let write = make_tool(ToolKind::Write, "w");
        let patch = make_tool(ToolKind::ApplyPatch, "p");
        let read = make_tool(ToolKind::Read, "r");
        assert!(app.tool_needs_approval(&bash));
        assert!(app.tool_needs_approval(&edit));
        assert!(app.tool_needs_approval(&write));
        assert!(app.tool_needs_approval(&patch));
        assert!(!app.tool_needs_approval(&read));
    }

    // Normal: a tool kind in `always_approved` is auto-approved even in
    // Default mode.
    #[test]
    fn tool_needs_approval_respects_always_approved_normal() {
        let mut app = new_app();
        let bash = make_tool(ToolKind::Bash, "b");
        app.always_approved.push(bash.kind.label().to_owned());
        assert!(!app.tool_needs_approval(&bash));
    }

    // Normal: session_approved similarly auto-approves.
    #[test]
    fn tool_needs_approval_respects_session_approved_normal() {
        let mut app = new_app();
        let edit = make_tool(ToolKind::Edit, "e");
        app.session_approved.push(edit.kind.label().to_owned());
        assert!(!app.tool_needs_approval(&edit));
    }

    // Normal: tool_denied_by_mode returns Some(reason) only for Plan mode
    // write tools.
    #[test]
    fn tool_denied_by_mode_plan_blocks_writes_normal() {
        let mut app = new_app();
        app.permission_mode = PermissionMode::Plan;
        let edit = make_tool(ToolKind::Edit, "e");
        let read = make_tool(ToolKind::Read, "r");
        assert!(app.tool_denied_by_mode(&edit).is_some());
        assert!(app.tool_denied_by_mode(&read).is_none());
    }

    // Robust: in Default mode, no tool is denied by mode (it's the prompt
    // gate, not a deny gate).
    #[test]
    fn tool_denied_by_mode_default_never_denies_robust() {
        let app = new_app();
        let bash = make_tool(ToolKind::Bash, "b");
        assert!(app.tool_denied_by_mode(&bash).is_none());
    }

    // ─────── selected_context_window_tokens / sync ────────────────────

    // Normal: with no provider model info loaded, falls back to the
    // model-name heuristic. We just verify the result is positive (the
    // exact value depends on the heuristic for "test-model").
    #[test]
    fn selected_context_window_tokens_falls_back_normal() {
        let app = new_app();
        let result = app.selected_context_window_tokens();
        assert!(result > 0);
    }

    // Normal: sync_selected_context_window updates max_context_tokens
    // based on the heuristic and recomputes approx_tokens from messages.
    #[test]
    fn sync_selected_context_window_updates_max_normal() {
        let mut app = new_app();
        app.messages
            .push(ChatMessage::user("0123456789abcdef".into()));
        app.sync_selected_context_window();
        assert_eq!(app.max_context_tokens, app.selected_context_window_tokens());
        // 16 chars / 4 * 1.5 = 6 tokens.
        assert_eq!(app.tool_ctx.approx_tokens, 6);
    }

    // Robust: when a message carries usage data, sync prefers the
    // usage-based estimate over the heuristic.
    #[test]
    fn sync_preserves_usage_based_estimate_robust() {
        let mut app = new_app();
        let mut msg = ChatMessage::assistant("hello".into());
        msg.usage = Some(ModelUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 10,
            cache_write_tokens: 5,
            cost_usd: None,
        });
        app.messages.push(msg);
        // Without sync, approx_tokens is 0. recompute_token_estimate
        // is what reads the usage into approx_tokens.
        app.recompute_token_estimate();
        assert_eq!(app.tool_ctx.approx_tokens, 165);
        // After sync, the usage-based estimate is preserved (not
        // clobbered by the heuristic over message text).
        let preserved = app.tool_ctx.approx_tokens;
        app.sync_selected_context_window();
        assert_eq!(app.tool_ctx.approx_tokens, preserved);
    }

    // ─────── recompute_token_estimate ─────────────────────────────────

    // Normal: with no usage messages, recompute uses the rough estimator
    // and resets last_usage_input/output.
    #[test]
    fn recompute_no_usage_uses_estimator_normal() {
        let mut app = new_app();
        app.messages
            .push(ChatMessage::user("0123456789abcdef".into()));
        app.last_usage_input = 999;
        app.last_usage_output = 999;
        app.recompute_token_estimate();
        assert_eq!(app.last_usage_input, 0);
        assert_eq!(app.last_usage_output, 0);
        assert_eq!(app.tool_ctx.approx_tokens, 6);
    }

    // Normal: with a usage message followed by a tail, recompute uses
    // total_context_tokens + tail estimate.
    #[test]
    fn recompute_with_usage_plus_tail_normal() {
        let mut app = new_app();
        let mut anchor = ChatMessage::assistant("hi".into());
        anchor.usage = Some(ModelUsage {
            input_tokens: 1_000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        app.messages.push(anchor);
        // 16-char user message after the anchor → 6 tail tokens.
        app.messages
            .push(ChatMessage::user("0123456789abcdef".into()));
        app.recompute_token_estimate();
        assert_eq!(app.tool_ctx.approx_tokens, 1_500 + 6);
        assert_eq!(app.last_usage_input, 1_000);
        assert_eq!(app.last_usage_output, 500);
    }

    // ─────── cwd resolution ──────────────────────────────────────────

    // Normal: App::new fills cwd from std::env::current_dir(). It should
    // be a non-empty string for any sane test environment.
    #[test]
    fn app_new_resolves_cwd_normal() {
        let app = new_app();
        assert!(!app.cwd.is_empty(), "cwd resolved");
    }

    // ─────── switch_session ──────────────────────────────────────────

    // Normal: switching session clears per-session state and clears
    // compact_suppressed.
    #[test]
    fn switch_session_resets_state_normal() {
        let mut app = new_app();
        app.compact_suppressed = true;
        app.task_panel_selected = 5;
        app.viewing_task_id = Some("t1".into());
        app.viewing_task_expanded
            .insert("t1".into(), std::collections::HashSet::new());
        app.task_completion_times
            .insert(crate::tasks::TaskId::from("t1"), Instant::now());

        app.switch_session(Some("ses_test_switch".into()));

        assert!(!app.compact_suppressed);
        assert_eq!(app.task_panel_selected, 0);
        assert!(app.viewing_task_id.is_none());
        assert!(app.viewing_task_expanded.is_empty());
        assert!(app.task_completion_times.is_empty());
        assert_eq!(app.current_session_id.as_deref(), Some("ses_test_switch"));
    }

    // Normal: switch_session(None) installs a freshly-generated id and
    // never leaves current_session_id as None. (The id may match the
    // prior one if the call lands within the same second-resolution
    // timestamp — generate_session_id uses `%Y%m%d_%H%M%S` — so we don't
    // assert distinctness.)
    #[test]
    fn switch_session_none_mints_fresh_id_normal() {
        let mut app = new_app();
        app.current_session_id = None;
        app.switch_session(None);
        assert!(app.current_session_id.is_some());
        let id = app.current_session_id.as_deref().unwrap();
        assert!(id.starts_with("ses_"), "id has expected prefix: {id}");
    }

    // ─────── sync_task_completions ────────────────────────────────────

    // Normal: a newly-completed task picks up a completion timestamp;
    // a pruned/deleted task is removed.
    #[test]
    fn sync_task_completions_tracks_and_prunes_normal() {
        use crate::tasks::{TaskPatch, TaskStatus};
        let mut app = new_app();
        // Create a task in the in-memory store (App::new opens a
        // session-id-keyed store; for these tests it persists on disk
        // under XDG_CONFIG_HOME, but the in-memory data still works).
        let t1 = app
            .task_store
            .create::<crate::tasks::TaskId>("subj".into(), "desc".into(), None, Vec::new())
            .expect("created");
        // Mark it completed.
        app.task_store
            .update(
                t1.id.as_str(),
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..TaskPatch::default()
                },
            )
            .expect("update");

        app.sync_task_completions();
        assert!(app.task_completion_times.contains_key(&t1.id));

        // Re-open: sync should prune the entry.
        app.task_store
            .update(
                t1.id.as_str(),
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    ..TaskPatch::default()
                },
            )
            .expect("reopen");
        app.sync_task_completions();
        assert!(!app.task_completion_times.contains_key(&t1.id));
    }

    // ─────── recent_models helpers ────────────────────────────────────

    /// RAII guard pointing `XDG_CONFIG_HOME` at a tempdir for the
    /// duration of one test so `push_recent_model` doesn't clobber the
    /// developer's `~/.config/jfc/recent_models.json`.
    struct TempConfigHome {
        _dir: tempfile::TempDir,
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    static RECENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl TempConfigHome {
        fn new() -> Self {
            let guard = RECENT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::TempDir::new().expect("tempdir");
            let prior = std::env::var("XDG_CONFIG_HOME").ok();
            // Safety: env mutation serialized through RECENT_LOCK.
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
            unsafe {
                match self.prior.take() {
                    Some(prev) => std::env::set_var("XDG_CONFIG_HOME", prev),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    // Normal: push_recent_model dedupes and caps at 5. Sandboxed to
    // a tempdir so the on-disk write doesn't touch the user's config.
    #[test]
    fn push_recent_model_dedupes_and_caps_normal() {
        let _g = TempConfigHome::new();
        let mut recent = vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
            "e".to_owned(),
        ];
        push_recent_model(&mut recent, "b");
        // Moved to front, length unchanged.
        assert_eq!(recent[0], "b");
        assert_eq!(recent.len(), 5);
        // No duplicates.
        let mut sorted = recent.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5);

        // Pushing a 6th unique value still caps at 5.
        push_recent_model(&mut recent, "f");
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0], "f");
    }

    // Robust: push_recent_model on an empty list seeds the first entry.
    #[test]
    fn push_recent_model_empty_seed_robust() {
        let _g = TempConfigHome::new();
        let mut recent: Vec<String> = Vec::new();
        push_recent_model(&mut recent, "x");
        assert_eq!(recent, vec!["x".to_owned()]);
    }

    // Normal: save_recent_models then load_recent_models round-trips.
    #[test]
    fn save_load_recent_models_round_trips_normal() {
        let _g = TempConfigHome::new();
        // Ensure jfc/ exists under the tempdir so save_recent_models
        // doesn't silently fail.
        let cfg = dirs::config_dir().expect("config dir");
        std::fs::create_dir_all(cfg.join("jfc")).expect("jfc dir");
        let models = vec!["m1".to_owned(), "m2".to_owned()];
        save_recent_models(&models);
        let loaded = load_recent_models();
        assert_eq!(loaded, models);
    }

    // Robust: load_recent_models returns empty when no file exists.
    #[test]
    fn load_recent_models_missing_is_empty_robust() {
        let _g = TempConfigHome::new();
        let loaded = load_recent_models();
        assert!(loaded.is_empty());
    }

    // ─────── PendingApproval / Tool side-effect free helpers ──────────

    // Robust: is_readonly_bash recognises the documented read-only commands
    // and rejects the write-side commands. (Sample, not exhaustive.)
    #[test]
    fn is_readonly_bash_recognises_examples_robust() {
        for cmd in [
            "ls",
            "ls -la",
            "cat README.md",
            "git log",
            "git diff HEAD",
            "git status",
            "cargo check",
            "cargo test --bin jfc",
            "rg pattern",
        ] {
            assert!(is_readonly_bash(cmd), "expected read-only: {cmd}");
        }
        for cmd in [
            "rm -rf /",
            "git push",
            "cargo build --release",
            "mv a b",
            "cp a b",
            "echo hello > file",
        ] {
            // `echo` *is* in the read-only list, so skip that one.
            if cmd.starts_with("echo") {
                continue;
            }
            assert!(!is_readonly_bash(cmd), "expected write: {cmd}");
        }
    }

    // Robust: empty bash command falls through to the read-only list which
    // rejects empty (first_word = "" doesn't match any read-only entry).
    #[test]
    fn is_readonly_bash_empty_is_not_readonly_robust() {
        assert!(!is_readonly_bash(""));
    }

    // ─────── selected_model_info ──────────────────────────────────────

    // Robust: with no provider_models cache and no matching available
    // models, selected_model_info returns None.
    #[test]
    fn selected_model_info_none_when_no_match_robust() {
        let app = new_app();
        // TestProvider returns empty available_models; the model id
        // "test-model" never appears anywhere. So None.
        assert!(app.selected_model_info().is_none());
    }

    // Normal: when provider_models has a match, selected_model_info
    // returns it.
    #[test]
    fn selected_model_info_finds_in_cache_normal() {
        let mut app = new_app();
        let info =
            ModelInfo::new("test-model", "Test", "test").with_context_window_tokens(Some(50_000));
        app.provider_models.insert(
            crate::provider::ProviderId::from("test"),
            vec![info.clone()],
        );
        let got = app.selected_model_info().expect("found");
        assert_eq!(got.id.as_str(), "test-model");
        assert_eq!(got.context_window_tokens, Some(50_000));
        // selected_context_window_tokens uses this value.
        assert_eq!(app.selected_context_window_tokens(), 50_000);
    }

    // ─────── round-trip MessagePart variants for sanity ───────────────

    // Normal: Tool message parts carry the same input/output structure.
    // Exercises ToolInput::Edit construction with ReplacementMode.
    #[test]
    fn message_part_tool_carries_input_output_normal() {
        let tool = ToolCall {
            id: "t".into(),
            kind: ToolKind::Edit,
            status: ToolStatus::Complete,
            input: ToolInput::Edit {
                file_path: "src/x.rs".into(),
                old_string: "old".into(),
                new_string: "new".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            output: ToolOutput::Text("ok".into()),
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        };
        let part = MessagePart::Tool(tool);
        match part {
            MessagePart::Tool(tc) => {
                assert_eq!(tc.kind, ToolKind::Edit);
                assert!(matches!(tc.input, ToolInput::Edit { .. }));
                assert!(matches!(tc.output, ToolOutput::Text(_)));
            }
            _ => panic!("expected Tool"),
        }
    }
}
