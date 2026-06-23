use std::path::PathBuf;

use tokio::sync::mpsc;

use super::ExecutionResult;
use crate::types::{ChatMessage, ToolCall};
use jfc_provider::{
    FallbackReason, ModelInfo, ProviderId, ResolvedModel, ServerToolResultKind, StopReason,
};

/// Bounded channel capacity for the main runtime event loop. Fine-grained
/// provider streaming and concurrent tool result floods can briefly exceed a
/// thousand events; keeping more headroom prevents the SSE reader from
/// backpressuring the provider while still bounding memory.
pub const APP_EVENT_BUFFER: usize = 4096;

pub type EventSender = mpsc::Sender<EngineEvent>;
pub type EventReceiver = mpsc::Receiver<EngineEvent>;

/// Send an event that must not be dropped — terminal/continuation signals
/// such as [`ToolEvent::AllComplete`] whose loss permanently wedges the
/// agentic loop (the next turn never fires). Tries the non-blocking path
/// first; if the bounded channel is momentarily full, hands the event to a
/// task that awaits capacity instead of discarding it. Only a *closed*
/// channel (receiver gone — app shutting down) is a no-op. Must be called
/// from within a Tokio runtime.
pub fn send_critical(tx: &mpsc::Sender<EngineEvent>, ev: EngineEvent) {
    match tx.try_send(ev) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(ev)) => {
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = tx.send(ev).await;
            });
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            tracing::debug!(
                target: "jfc::runtime",
                "send_critical: event channel closed; dropping (app shutting down)"
            );
        }
    }
}

/// Every event the engine produces or consumes. This is the frontend-neutral
/// event bus: nothing in here may reference ratatui/crossterm types. The TUI
/// wraps it in [`AppEvent`] alongside its own terminal events; headless and
/// remote frontends consume it directly.
pub enum EngineEvent {
    /// Stream event emitted by a specific model-stream task.
    ///
    /// Model streams can be superseded by retry, watchdog, interrupt-on-submit,
    /// or a fresh user turn while the old task is still unwinding. The stream id
    /// lets the dispatcher reject stale terminal/provider events before they
    /// mutate the current transcript.
    ScopedStream {
        stream_id: u64,
        event: StreamEvent,
    },
    Stream(StreamEvent),
    Tool(ToolEvent),
    Compaction(CompactionEvent),
    Provider(ProviderEvent),
    Task(TaskEvent),
    Team(TeamEvent),
    Goal(GoalEvent),
    Voice(VoiceEvent),
    /// Live progress update from a running workflow background task.
    WorkflowProgress(WorkflowProgressEvent),
    /// Inbound command for the engine — from detached producers (remote
    /// control, schedulers, background tasks) or frontend code paths that
    /// only hold an event sender.
    Control(ControlEvent),
    /// Outbound request/notification from the engine that a frontend must
    /// surface to the user (plan review, plan-mode transitions).
    Frontend(FrontendEvent),
}

/// Inbound engine commands that previously rode on `UiEvent` or were faked
/// as synthetic terminal keystrokes by the remote-control host.
pub enum ControlEvent {
    /// Submit a user prompt as if the user typed it and pressed Enter. Used
    /// by the pre-submit compaction gate (re-fires the original prompt once
    /// compaction shrank the context), the task factory, and remote clients.
    SubmitPrompt(String),
    /// Interrupt the current turn: cancel streams, abort in-flight tools,
    /// deny pending approvals. Replaces the remote host's synthetic Esc.
    Interrupt,
    /// Resolve a specific pending permission request. Carries the tool id so
    /// late/orphaned responses can be matched to unresolved transcript
    /// tool_use blocks instead of blindly answering whichever modal is
    /// currently focused.
    ResolveApproval { tool_use_id: String, approved: bool },
    /// Resolve the pending plan-approval (ExitPlanMode) request. Replaces
    /// the remote host's synthetic 'y'/'n' keystrokes.
    ResolvePlan { approved: bool },
    /// Load a session by id — async load via the same helper the sidebar's
    /// Enter handler uses. Lives on the event bus because picker handlers
    /// are sync; routing through here keeps the disk I/O on the event-loop
    /// task.
    LoadSession(crate::ids::SessionId),
    /// Surface a non-blocking notice. The TUI renders it as a toast on the
    /// auto-expiring strip; headless frontends map it to stderr/log lines.
    Notice {
        kind: crate::toast::ToastKind,
        text: String,
    },
    /// Async result from the periodic `git worktree list` refresh, spawned
    /// off-loop so a slow or locked git repo cannot stall the frontend.
    WorktreeCountLoaded(usize),
    /// Run a slash command. Queued-prompt draining inside the engine cannot
    /// call the frontend's command dispatch directly; it routes the command
    /// back over the bus and the frontend executes it. Stage 8 of the
    /// extraction moves engine-pure command semantics into the engine and
    /// shrinks this to view commands only.
    RunCommand(String),
    /// User responded to an MCP elicitation. The `id` must match a pending
    /// `FrontendEvent::ElicitationRequest`. The engine routes this to
    /// `mcp_elicitation::resolve(id, response)` which unblocks the waiting
    /// `JfcClientHandler::create_elicitation` future.
    ResolveElicitation {
        id: String,
        response: crate::mcp_elicitation::ElicitationResponse,
    },
}

/// Outbound engine→frontend requests that previously rode on `UiEvent`.
pub enum FrontendEvent {
    /// The model called `ExitPlanMode` and wants the user to review the
    /// plan + transition out of plan mode.
    PlanReview { plan: String },
    /// A structured review run completed and was persisted under `.jfc/reviews`.
    ReviewCompleted {
        review: crate::review::ReviewOutputEvent,
    },
    /// The model recorded one actionable review comment.
    ReviewCommentAdded {
        comment: crate::review::ReviewComment,
    },
    /// The model submitted a plan artifact through `SubmitPlan`.
    ImplementationPlanSubmitted { plan: crate::review::SubmittedPlan },
    /// The model proposed a commit message for the current diff.
    CommitMessageSuggested {
        suggestion: crate::review::CommitMessageSuggestion,
    },
    /// Model-callable plan-mode entry. Dispatched by the `EnterPlanMode`
    /// tool — flips the permission mode to `PermissionMode::Plan`.
    PlanModeEntered { reason: String },
    /// Model-callable session goal. Dispatched by the `SetGoal` tool — the agent
    /// distilled a stop-condition for the current task and wants the goal loop
    /// to drive it to completion. An empty/clear `condition` clears the goal.
    GoalSet { condition: String },
    /// An MCP server is requesting interactive user input (elicitation/create).
    /// The frontend must present the form or URL to the user, collect their
    /// response, and dispatch `ControlEvent::ResolveElicitation` with the
    /// matching `id`.
    ElicitationRequest {
        /// Unique ID — pass back in `ControlEvent::ResolveElicitation`.
        id: String,
        /// Which MCP server sent this.
        server_name: String,
        /// What's being requested (form fields or URL).
        kind: crate::mcp_elicitation::ElicitationKind,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StreamToolChoice {
    #[default]
    Auto,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamRequestOverrides {
    pub tool_choice: StreamToolChoice,
    /// System reminders queued by background events (file watcher,
    /// MCP refresh, …) and drained into `prepare_stream_request` so
    /// they land in the next outbound request's system prompt exactly
    /// once, without mutating `app.engine.messages` per FS event.
    pub background_reminders: Vec<String>,
    /// Combined disallowed tools from CLI `--disallowed-tools` and
    /// CLAUDE.md frontmatter. These tools are removed from the
    /// advertised tool catalog before sending to the model.
    pub disallowed_tools: Vec<String>,
    /// Additional roots whose CLAUDE.md layers should be loaded into context.
    pub extra_dirs: Vec<PathBuf>,
    /// Optional allowlist from CLI/managed settings. When non-empty, only
    /// matching tool names are advertised to the model.
    pub allowed_tools: Vec<String>,
    /// Additional Anthropic beta tokens supplied by `--betas`.
    pub custom_betas: Vec<String>,
    /// Request eager local tool input streaming on Anthropic native routes.
    pub fine_grained_tool_streaming: bool,
    /// Request strict local tool schema validation on Anthropic native routes.
    pub strict_tool_schemas: bool,
    /// Per-request task budget token hint from `--task-budget`.
    pub task_budget: Option<u64>,
    /// Optional cap for legacy extended-thinking models.
    pub max_thinking_tokens: Option<u32>,
    /// Thinking display mode from `--thinking-display`.
    pub thinking_display: Option<String>,
    /// Interactive brief mode: keep `SendUserMessage` advertised and suppress
    /// routine assistant prose from the visible transcript.
    pub brief_mode: bool,
    /// Tokens saved by the most recent compaction, forwarded once on the next
    /// outbound request so the `context-hint-2026-04-09` beta path fires
    /// (`context_hint.target_tokens_saved`). Drained after a single use —
    /// the hint is only meaningful on the turn immediately following a
    /// compaction. `None` when no compaction happened since the last send.
    pub context_hint_tokens_saved: Option<u64>,
    /// Last API-reported input-token count for this conversation, when known.
    /// Used by the optional `<total_tokens>... tokens left</total_tokens>`
    /// prompt attachment in countdown mode.
    pub last_usage_input_tokens: Option<u64>,
    /// Context window for the active model, when known.
    pub context_window_tokens: Option<u64>,
    /// Test/explicit override for the optional total-token reminder mode.
    /// Production callers leave this as `None`, so env/config controls apply.
    pub total_tokens_reminder_mode: Option<crate::total_tokens_reminder::TotalTokensReminderMode>,
    /// Behavioral interaction mode for this turn (Code/Fast/Chat/Brainstorm).
    /// Resolved once per user turn from the sticky `/mode` toggle + optional
    /// inference, then copied here. `Code` (the default) appends nothing, so the
    /// default request is byte-identical to pre-feature behavior.
    pub interaction_mode: crate::interaction_mode::InteractionMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamRequestMetadata {
    pub advertised_tool_count: usize,
    pub action_expected: bool,
    pub tool_choice: StreamToolChoice,
    pub resolved_model: Option<ResolvedModel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamLifecyclePhase {
    PreparingContext,
    WaitingForFirstByte,
    StreamOpened,
    RetryingWithoutThinking,
    NonStreamingFallback,
}

impl StreamLifecyclePhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::PreparingContext => "preparing context",
            Self::WaitingForFirstByte => "waiting first byte",
            Self::StreamOpened => "stream opened",
            Self::RetryingWithoutThinking => "retrying without thinking",
            Self::NonStreamingFallback => "non-stream fallback",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamLifecycleStatus {
    pub phase: StreamLifecyclePhase,
    pub detail: Option<String>,
    pub updated_at: std::time::Instant,
}

impl StreamLifecycleStatus {
    pub fn new(phase: StreamLifecyclePhase, detail: impl Into<Option<String>>) -> Self {
        Self {
            phase,
            detail: detail.into(),
            updated_at: std::time::Instant::now(),
        }
    }
}

#[derive(Clone)]
pub enum StreamEvent {
    Chunk {
        text: Option<String>,
        reasoning: Option<String>,
    },
    /// Tool input JSON delta — streamed while the model builds tool_use
    /// arguments. Carries the provider block index and the delta text so
    /// frontends can both keep token estimates live (TUI spinner) and emit
    /// faithful wire events (headless stream-json).
    ToolInputDelta {
        index: usize,
        delta: String,
    },
    /// Server-authoritative thinking token estimate delta. Emitted on each
    /// `thinking_delta` event with `estimated_tokens` set. Accumulates across
    /// the thinking block for display (matching cli.js's thinking_tokens system
    /// events which surface token/sec stats during redacted-thinking phase).
    ThinkingTokens(u32),
    Tool(Box<ToolCall>),
    /// Opaque redacted thinking blob — store on message parts for round-tripping.
    RedactedThinking(String),
    /// API response metadata — the message ID (stored for
    /// `diagnostics.previous_message_id`) plus the provider's early
    /// input-token count when available (headless re-emits it on the wire).
    ResponseId {
        id: String,
        input_tokens: Option<u64>,
    },
    /// Anthropic-side `server_tool_result` block (e.g.
    /// `web_search_tool_result`) paired with a previously-dispatched
    /// `server_tool_use`. The event_loop handler finds the matching
    /// ToolCall on the streaming assistant message and replaces its
    /// output with a `ToolOutput::ServerToolResult` so the result
    /// round-trips byte-faithfully on the next resend. See
    /// `live_events.rs` for the SSE-to-runtime translation and
    /// `tool_wire::server_tool_result_content` for the resend path.
    ServerToolResult {
        tool_use_id: crate::ids::ToolId,
        tool_kind: ServerToolResultKind,
        content: serde_json::Value,
    },
    Done(StopReason),
    Error(String),
    /// The provider switched from the requested model to a fallback
    /// (e.g. 529 overload caused an Opus→Sonnet swap). The UI shows a
    /// toast and optionally updates `app.engine.model`.
    FallbackTriggered {
        original_model: String,
        fallback_model: String,
        reason: FallbackReason,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    },
    /// System prompt token estimate from the most recent stream request.
    /// Used by the CompactionDone handler to add overhead to the post-
    /// compact approx_tokens gauge.
    SystemPromptLen(usize),
    /// Byte length of the memory-recall block injected into this turn's system
    /// prompt (only sent when > 0). Surfaced to the user as a brief "recalled
    /// memory" toast so they can see context was pulled in.
    MemoryRecalled(usize),
    /// Per-request control metadata captured after tools and permission
    /// filtering are known. The event loop uses this to tell a valid prose
    /// answer from a narration-only failure on agentic prompts.
    RequestMetadata(StreamRequestMetadata),
    /// User-visible stream lifecycle phase before normal content arrives.
    /// Keeps long context assembly, first-byte waits, and fallback retries from
    /// looking like a frozen UI.
    Lifecycle(StreamLifecycleStatus),
    /// Content-free wire-liveness tick — forwarded from a provider `Keepalive`
    /// (an SSE `ping`/comment frame). The dispatcher routes it straight to
    /// `record_stream_activity()` so the stream idle watchdog resets even
    /// during long no-delta phases (extended thinking, big tool-input streams).
    /// It mutates nothing else: no text, tokens, or message parts.
    Keepalive,
}

pub fn stream_event(ev: &EngineEvent) -> Option<&StreamEvent> {
    match ev {
        EngineEvent::Stream(event) | EngineEvent::ScopedStream { event, .. } => Some(event),
        _ => None,
    }
}

pub fn scoped_stream_sender(tx: EventSender, stream_id: u64) -> EventSender {
    let (scoped_tx, mut scoped_rx) = mpsc::channel(APP_EVENT_BUFFER);
    tokio::spawn(async move {
        while let Some(ev) = scoped_rx.recv().await {
            let ev = match ev {
                EngineEvent::Stream(event) => EngineEvent::ScopedStream { stream_id, event },
                other => other,
            };
            if tx.send(ev).await.is_err() {
                break;
            }
        }
    });
    scoped_tx
}
pub enum ToolEvent {
    Result {
        tool_id: crate::ids::ToolId,
        result: ExecutionResult,
    },
    /// Incremental output from a running tool (e.g. bash stdout line-by-line).
    /// The UI appends this to the tool's live output preview.
    OutputChunk {
        tool_id: crate::ids::ToolId,
        chunk: String,
    },
    AllComplete,
    /// v126 auto-mode classifier finished judging a pending tool call. When
    /// `blocked` is true, the tool is marked Failed with `reason` and never
    /// runs; when false, the tool is dispatched immediately without prompting
    /// the user (auto-mode replaces the manual approval flow).
    ClassifierDecision {
        tool: Box<ToolCall>,
        blocked: bool,
        reason: String,
    },
    /// SDK/remote bridge state: update the set of tool_use ids currently
    /// executing. `action` is "add", "remove", or "set" to match upstream's
    /// `set_in_progress_tool_use_ids` shape.
    SetInProgressToolUseIds {
        action: String,
        ids: Vec<String>,
    },
    /// Tool use has been yielded but not executed yet. This covers approval
    /// waits, classifier waits, and stream_done batch queues.
    DeferredToolUse {
        id: String,
        name: String,
        input_preview: String,
        reason: String,
    },
    /// Single-line label for the just-completed tool batch.
    UseSummary {
        summary: String,
        preceding_tool_use_ids: Vec<String>,
    },
}

pub enum CompactionEvent {
    Started,
    /// Streaming compact has emitted more text. `output_chars` is the
    /// total length of the summary collected so far. Mirrors v126's
    /// `addResponseLength` callback in PB7 (cli.js:396989) — fires on
    /// every text_delta during compaction so the spinner can show
    /// `↓ Nk tokens` building up live, not just the elapsed timer.
    Progress {
        output_chars: u64,
    },
    Done {
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
    Failed {
        reason: String,
        calibrated_tokens: Option<usize>,
        transient: bool,
    },
}

pub enum TaskEvent {
    /// One streaming text chunk from a subagent. Routed into the matching
    /// `BackgroundTask.messages` so the task view shows the agent's
    /// output live as it streams (instead of "No messages yet" until
    /// the agent reports a tool via `TaskProgress`). Mirrors v126's
    /// per-agent stream handler that pipes nested-stream chunks into
    /// the parent's task buffer.
    AgentChunk {
        task_id: crate::ids::TaskId,
        text: String,
    },
    Started {
        task_id: crate::ids::TaskId,
        description: String,
        model_used: Option<String>,
        max_input_tokens: Option<u64>,
        /// True iff this task is a detached background worker (run via
        /// `spawn_background_agent_worker`). Detached workers register
        /// themselves into the daemon roster from their own process with
        /// the correct PID and launch_path — the UI must NOT overwrite
        /// that record on TaskStarted. Foreground (in-process) teammates
        /// and subagents have `is_detached = false`; for those the daemon
        /// roster is only used as a passive log target, and the
        /// reconciler later marks them stale when the UI exits.
        ///
        /// Default to `false` so legacy/test sites that omit the field
        /// keep their previous behavior (foreground registration).
        is_detached: bool,
        /// Queued task id (`t<N>`) this delegation fulfils, if the model
        /// linked the Task call to a todo via `parent_task_id`. The
        /// `TaskStarted` handler flips that task to `in_progress`; the
        /// matching `TaskCompleted`/`TaskFailed` handler flips it to
        /// `completed`/`failed`. `None` for un-linked ad-hoc delegations.
        parent_task_id: Option<String>,
    },
    Progress {
        task_id: crate::ids::TaskId,
        last_tool: Option<String>,
        elapsed_ms: u64,
        /// Cumulative tools invoked this run (None = no update). Routed
        /// to `BackgroundTask.tool_use_count` so the fan UI can render
        /// "(N tools)" beside the spinner.
        tool_use_count: Option<u32>,
        /// Latest API request's input-token count (None = no update).
        input_tokens: Option<u64>,
        /// Latest API request's cache-read token count (None = no update).
        cache_read_tokens: Option<u64>,
        /// Latest API request's cache-write token count (None = no update).
        cache_write_tokens: Option<u64>,
        /// Output tokens consumed during the latest API round-trip
        /// (None = no update). Folded into `cumulative_output_tokens`.
        output_tokens: Option<u64>,
    },
    Completed {
        task_id: crate::ids::TaskId,
        summary: String,
        elapsed_ms: u64,
    },
    Failed {
        task_id: crate::ids::TaskId,
        error: String,
    },
}

pub enum ProviderEvent {
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
    /// Background snapshot of the active Anthropic OAuth account. Cached on
    /// `App.anthropic_account_snapshot` and consumed by the ribbon to show
    /// utilization (5h / 7d) plus the active claim type.
    AnthropicSnapshotUpdated {
        snapshot: Option<crate::providers::anthropic_accounts::AccountSnapshot>,
    },
    /// Best-effort heartbeat from status.claude.com. Kept separate from
    /// provider stream errors so the UI can show both the immediate HTTP
    /// retry state and the broader Anthropic service state.
    ClaudeStatusUpdated(crate::claude_status::ClaudeStatusUpdate),
    McpUpdated {
        servers: Vec<crate::types::McpServerInfo>,
    },
    LspUpdated {
        servers: Vec<crate::types::LspServerInfo>,
    },
    /// LSP push: full set of currently-active diagnostics. Replaces
    /// `app.engine.diagnostics` wholesale (the LSP client should send a fresh
    /// snapshot, not deltas, so the consumer doesn't have to dedup).
    /// Mirrors v126 cli.js:338038 — the `Found N issues in M files` row
    /// is rendered from this state.
    DiagnosticsUpdated {
        entries: Vec<crate::diagnostics::DiagnosticEntry>,
    },
}

pub enum TeamEvent {
    /// Event from an in-process teammate runner (idle, progress, completion, message).
    Runner(crate::swarm::runner::TeammateEvent),
    /// Inbound message from a teammate (delivered via the leader inbox).
    /// Two outcomes: the message gets appended to the transcript as a
    /// system-tagged user turn so the model can see it on its next
    /// request, AND a toast surfaces the arrival so the user notices.
    /// Mirrors v126's `<teammate-message>` injection.
    Inbox {
        from: String,
        text: String,
        summary: Option<String>,
    },
    /// A teammate has been spawned (Task tool with name+team_name set). Carries
    /// the data the leader needs to populate `app.engine.team_context.team_name` and
    /// `app.engine.team_context.teammates`. Without this event, both fields stayed
    /// empty regardless of how many teammates were spawned, so the team-mode
    /// teammate tree never activated and `team_context.is_active()` lied
    /// about whether a team was in flight.
    Spawned {
        name: String,
        team_name: String,
        agent_id: String,
        color: Option<String>,
        agent_type: Option<String>,
        cwd: String,
        /// Abort handle returned by `swarm::runner::start_teammate`. The
        /// event handler must move this into
        /// `app.engine.team_context.teammates[agent_id].abort_tx` so the channel
        /// stays open for the teammate's lifetime. Dropping it closes the
        /// channel and the runner's abort_rx.changed() resolves Err on the
        /// next poll — which the runner treats as Cancelled, lighting up
        /// every teammate as "Done" before doing any work.
        abort_tx: Option<tokio::sync::watch::Sender<bool>>,
    },
}

pub enum GoalEvent {
    /// Verdict from the `/goal` stop-condition evaluator. Emitted by a
    /// background task spawned at EndTurn when `app.engine.goal.is_some()`.
    /// The event_loop handler decides whether to inject a continuation
    /// reminder (`ok=false`) or stamp a success banner (`ok=true`).
    Verdict { ok: bool, reason: String },
}

/// Voice mode events from the jfc-voice STT pipeline.
pub enum VoiceEvent {
    /// Interim partial transcript — show in the status bar / input box preview.
    Interim(String),
    /// Final transcript — inject into the textarea and optionally auto-submit.
    Final(String),
    /// Voice state changed (idle / recording / processing).
    StateChanged(u8), // 0=idle, 1=recording, 2=processing
    /// A normalized [0,1] RMS audio level sample, emitted per captured chunk
    /// while recording. Drives the live recording-cursor animation.
    Level(f32),
    /// Error from the voice pipeline.
    Error(String),
}

/// A progress update from a running workflow, routed to the matching
/// `BackgroundTask::workflow_progress` entry. Emitted by the runner's
/// orchestrator loop so the UI can show live phase/agent/log state without
/// waiting for the workflow to complete.
pub enum WorkflowProgressEvent {
    /// The script called `phase(title)` — advance `current_phase`.
    Phase {
        task_id: crate::ids::TaskId,
        title: String,
    },
    /// A new `agent()` call was dispatched (not a cache hit).
    AgentStarted {
        task_id: crate::ids::TaskId,
        index: u32,
        label: String,
        phase: Option<String>,
    },
    /// An `agent()` call was satisfied from the resume cache (no dispatch).
    AgentCacheHit {
        task_id: crate::ids::TaskId,
        index: u32,
        label: String,
        phase: Option<String>,
    },
    /// An `agent()` dispatch completed successfully.
    AgentDone {
        task_id: crate::ids::TaskId,
        index: u32,
    },
    /// An `agent()` dispatch failed.
    AgentFailed {
        task_id: crate::ids::TaskId,
        index: u32,
        error: String,
    },
    /// The script emitted a `log(message)` call.
    Log {
        task_id: crate::ids::TaskId,
        message: String,
    },
}

impl WorkflowProgressEvent {
    /// The background-task id this progress event belongs to.
    pub fn task_id(&self) -> &crate::ids::TaskId {
        match self {
            Self::Phase { task_id, .. }
            | Self::AgentStarted { task_id, .. }
            | Self::AgentCacheHit { task_id, .. }
            | Self::AgentDone { task_id, .. }
            | Self::AgentFailed { task_id, .. }
            | Self::Log { task_id, .. } => task_id,
        }
    }
}
