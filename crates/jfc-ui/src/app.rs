use std::{cell::RefCell, collections::HashMap, sync::Arc, time::Instant};

use crossterm::event::Event;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::TableState;
use tokio::sync::Mutex;

use tui_textarea::TextArea;

use crate::auto_mode::AutoModeConfig;

use crate::context::{ReadDedupCache, ToolContext};
use crate::provider::{ModelId, ModelInfo, Provider, ProviderId, StopReason};
use crate::query::QueryCache;
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
    AllToolsComplete,
    CompactionStarted,
    CompactionDone {
        messages: Vec<ChatMessage>,
        tool_ctx: crate::context::ToolContext,
        pre_tokens: usize,
        post_tokens: usize,
    },
    CompactionFailed(String),
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
            Self::Plan => {
                match tool.kind {
                    ToolKind::Read | ToolKind::Glob | ToolKind::Grep
                    | ToolKind::TaskCreate | ToolKind::TaskUpdate
                    | ToolKind::TaskList | ToolKind::TaskDone => {
                        PermissionDecision::Approved
                    }
                    ToolKind::Bash => {
                        let cmd = tool.input.summary().to_lowercase();
                        if is_readonly_bash(&cmd) {
                            PermissionDecision::Approved
                        } else {
                            PermissionDecision::Denied("Plan mode: write operations blocked")
                        }
                    }
                    _ => PermissionDecision::Denied("Plan mode: write operations blocked"),
                }
            }
            Self::AcceptEdits => {
                match tool.kind {
                    ToolKind::Write | ToolKind::Edit | ToolKind::ApplyPatch
                    | ToolKind::Read | ToolKind::Glob | ToolKind::Grep
                    | ToolKind::TaskCreate | ToolKind::TaskUpdate
                    | ToolKind::TaskList | ToolKind::TaskDone => {
                        PermissionDecision::Approved
                    }
                    _ => PermissionDecision::NeedsPrompt,
                }
            }
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
        "ls" | "cat" | "head" | "tail" | "find" | "grep" | "rg" | "fd"
            | "wc" | "file" | "stat" | "which" | "whoami" | "pwd" | "echo"
            | "date" | "env" | "printenv" | "uname" | "hostname" | "id"
            | "tree" | "du" | "df" | "free" | "ps"
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
#[derive(Debug, Clone)]
pub struct QueuedPrompt {
    pub text: String,
    pub is_meta: bool,
}

pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const TICK_MS: u64 = 80;

pub struct BackgroundTask {
    pub task_id: String,
    pub description: String,
    pub status: crate::types::TaskLifecycle,
    pub started_at: std::time::Instant,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub last_tool: Option<String>,
    pub messages: Vec<String>,
}

pub struct App {
    pub theme: Theme,
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
    /// Wall-clock instant the current compaction started. `Some` while a
    /// compact request is in flight (set on `CompactionStarted`, cleared on
    /// `CompactionDone`/`CompactionFailed`). The renderer shows a
    /// `Compacting…` spinner whenever this is `Some`, so a long pre-submit
    /// compaction doesn't look like a frozen UI.
    pub compacting_started_at: Option<Instant>,
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
    pub viewing_task_expanded: std::collections::HashSet<usize>,
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
}

impl App {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        let providers = vec![Arc::clone(&provider)];
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        textarea.set_placeholder_text("Type a message… (Enter to send, Shift+Enter for newline)");

        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
            .unwrap_or_default();

        let mut app = Self {
            theme: Theme::dark(),
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
            cwd,
            reasoning_expanded: HashMap::new(),
            pending_approval: None,
            approval_queue: std::collections::VecDeque::new(),
            queued_prompts: std::collections::VecDeque::new(),
            always_approved: Vec::new(),
            session_approved: Vec::new(),
            follow_bottom: true,
            tool_ctx: ToolContext::new(),
            dedup_cache: Arc::new(Mutex::new(ReadDedupCache::new())),
            pending_tool_calls: Vec::new(),
            force_compact_pending: false,
            compacting_started_at: None,
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
            delivered_diagnostics: std::collections::HashSet::new(),
            usage_apply_baseline: (0, 0, 0, 0),
            background_tasks: HashMap::new(),
            show_info_sidebar: true,
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
            usage_by_model: HashMap::new(),
            leader_key_active: false,
            leader_key_timeout: None,
            viewing_task_id: None,
            viewing_task_expanded: std::collections::HashSet::new(),
            pending_attachments: Vec::new(),
            tool_hit_regions: RefCell::new(Vec::new()),
        };
        app.sync_selected_context_window();
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
        let new_id = id.unwrap_or_else(crate::session::generate_session_id);
        self.current_session_id = Some(new_id);
        self.task_store = crate::tasks::TaskStore::in_memory();
        self.task_completion_times.clear();
        self.task_activities.clear();
        self.task_panel_selected = 0;
        self.task_panel_state = ratatui::widgets::TableState::default().with_selected(Some(0));
        self.viewing_task_id = None;
        self.viewing_task_expanded.clear();
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
        let last_usage = self
            .messages
            .iter()
            .rev()
            .find_map(|m| m.usage.as_ref());
        if let Some(u) = last_usage {
            self.last_usage_input = u.input_tokens as u32;
            self.last_usage_output = u.output_tokens as u32;
            // Anthropic counts input + cache_creation + cache_read +
            // output as the "context tokens used" — same as v126's W_$
            // (cli.js:197281). The Context gauge reads
            // `tool_ctx.approx_tokens` so writing the totalled value
            // there gives the user-visible bar the right fill.
            self.tool_ctx.approx_tokens = u.total_context_tokens() as usize;
        } else {
            self.last_usage_input = 0;
            self.last_usage_output = 0;
            self.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&self.messages);
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.max_scroll();
        self.follow_bottom = true;
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.follow_bottom = false;
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.follow_bottom = false;
    }

    pub fn scroll_down(&mut self, lines: usize) {
        let max = self.max_scroll();
        self.scroll_offset = (self.scroll_offset + lines).min(max);
        if self.scroll_offset >= max {
            self.follow_bottom = true;
        }
    }

    pub fn scroll_page_up(&mut self) {
        let half = self.half_page();
        self.scroll_up(half);
    }

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
        self.selected_model_info()
            .and_then(|model| model.context_window_tokens)
            .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS)
    }

    pub fn sync_selected_context_window(&mut self) {
        self.max_context_tokens = self.selected_context_window_tokens();
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
            return false;
        }
        if self.session_approved.iter().any(|n| n == name) {
            return false;
        }
        matches!(
            tool.kind,
            ToolKind::Bash | ToolKind::Write | ToolKind::Edit | ToolKind::ApplyPatch
        )
    }

    /// Check if a tool should be auto-denied by the current permission mode.
    pub fn tool_denied_by_mode(&self, tool: &ToolCall) -> Option<&'static str> {
        match self.permission_mode.auto_approves(tool) {
            PermissionDecision::Denied(reason) => Some(reason),
            _ => None,
        }
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
