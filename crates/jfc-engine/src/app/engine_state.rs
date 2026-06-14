//! The frontend-neutral engine state: everything the agentic runtime needs
//! to run a turn with no UI attached — conversation, streaming, turn control,
//! approvals, tasks/teams, providers, compaction, and run configuration.
//! Split out of the `App` god object as part of the jfc-engine extraction;
//! this file moves wholesale into the jfc-engine crate in a later stage.
//!
//! Invariant: nothing in here may reference ratatui/crossterm types or any
//! view-only state (scroll, textarea, pickers, render caches).

use indexmap::IndexMap;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Instant,
};

use tokio::sync::Mutex;

use crate::auto_mode::AutoModeConfig;
use crate::context::{ReadDedupCache, ToolContext};
use crate::runtime::{
    DEFERRED_TOOL_USES_CAP, DeferredToolUse, EngineEvent, MessageQueue, StreamLifecycleStatus,
    StreamRequestMetadata, TOOL_USE_SUMMARIES_CAP, ToolUseSummary,
};
use crate::slate::SlateRouter;
use crate::types::*;
use jfc_provider::{ModelId, ModelInfo, Provider, ProviderId};
use jfc_session::TaskId;

use super::{
    BACKGROUND_REMINDERS_CAP, DEFAULT_CONTEXT_WINDOW_TOKENS, STREAM_WATCHDOG_THINKING_TIMEOUT_SECS,
    STREAM_WATCHDOG_TIMEOUT_SECS,
};
use super::{PendingApproval, PermissionDecision, PermissionMode, load_recent_models};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRecoveryProvider {
    Anthropic,
    AnthropicOAuth,
    OpenWebUI,
    /// A transient error recognized from the bare error text (no provider
    /// sentinel) — e.g. a proxy 503 page or a transport-cancellation wrapper
    /// that didn't carry an `auto-retry-*` prefix. Classified uniformly via
    /// `jfc_provider::retry::retryable_stream_error`.
    Provider,
}

impl NetworkRecoveryProvider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::AnthropicOAuth => "anthropic-oauth",
            Self::OpenWebUI => "openwebui",
            Self::Provider => "provider",
        }
    }
}

/// Cap on consecutive auto-retries before a transient stream error is surfaced
/// as a hard error. Without this, a persistent 429/529/overload would restart
/// the stream forever. Each successful chunk/turn resets the counter to 0.
pub const MAX_NETWORK_RECOVERY_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRecoveryReason {
    Overloaded,
    RateLimited,
    ServerError,
    Transient,
}

impl NetworkRecoveryReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overloaded => "overloaded",
            Self::RateLimited => "rate limited",
            Self::ServerError => "server error",
            Self::Transient => "retryable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NetworkRecoveryStatus {
    pub provider: NetworkRecoveryProvider,
    pub reason: NetworkRecoveryReason,
    pub status_code: Option<u16>,
    pub attempts: u32,
    pub updated_at: Instant,
}

pub struct BackgroundTask {
    pub task_id: crate::ids::TaskId,
    pub description: String,
    pub status: crate::types::TaskLifecycle,
    pub started_at: std::time::Instant,
    /// When the task transitioned into a terminal state (Completed /
    /// Failed / Aborted). `None` while the task is still alive. Used by
    /// `render_subagent_tree` to keep the "pinned" hollow-circle row on
    /// screen for `COMPLETED_PIN_WINDOW` *after completion*, regardless
    /// of how long the task ran. Without this a solver that took longer
    /// than 5 minutes would vanish the very instant it finished — the
    /// "disappearing solver" bug.
    pub completed_at: Option<std::time::Instant>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub last_tool: Option<String>,
    /// Raw string log (kept for daemon log compat and the collapse/expand UI).
    pub messages: Vec<String>,
    /// Structured message history mirroring the main chat's Vec<ChatMessage>.
    /// Populated from AgentChunk (assistant text), TaskProgress (tool activity),
    /// and TaskCompleted/TaskFailed events. Used by the MessageView renderer to
    /// give the task view the same visual fidelity as the main conversation.
    pub chat_messages: Vec<crate::types::ChatMessage>,
    /// Cumulative tool invocations the subagent has made this run.
    /// Mirrors v131's `toolUseCount` (cli.2.1.131.beautified.js, `jOH()`).
    pub tool_use_count: u32,
    /// Most recent request's input-token count (driven by `Usage` stream
    /// events when the provider emits them, falls back to a 4-chars-per-
    /// token byte estimate otherwise). Mirrors v131's `latestInputTokens`.
    pub latest_input_tokens: u64,
    /// Most recent request's cache-read token count.
    pub latest_cache_read_tokens: u64,
    /// Most recent request's cache-write token count.
    pub latest_cache_write_tokens: u64,
    /// Sum of output tokens across every API round-trip in this run.
    /// Mirrors v131's `cumulativeOutputTokens`. The fan-UI badge displays
    /// the latest request context plus cumulative output.
    pub cumulative_output_tokens: u64,
    /// Model the agent is currently using. Captured from the spawn site
    /// so per-agent cost can be computed via `cost::cost_for(model, usage)`.
    pub model_used: Option<String>,
    /// Agent's own message transcript — populated by AgentChunk events
    /// from the swarm runner. Used for transcript foregrounding (when
    /// the user presses Enter on an agent in Ctrl+X, we render these
    /// instead of app.messages).
    pub agent_messages: Vec<crate::types::ChatMessage>,
    /// Per-agent token budget. When set and `latest_input + cumulative_output`
    /// exceeds it, the agent is forcibly terminated and an error toast
    /// fires. Defaults to None (unlimited).
    pub max_input_tokens: Option<u64>,
    /// Set once per task when the budget gets crossed so we don't fire
    /// the kill / toast multiple times.
    pub budget_killed: bool,
    /// Queued task id (`t<N>`) this delegated agent fulfils, if linked via
    /// the Task tool's `parent_task_id`. Captured on `TaskStarted` so the
    /// `TaskCompleted`/`TaskFailed` handlers — which only receive a
    /// `task_id` (the agent's run id, not the todo id) — can look up which
    /// `TaskStore` entry to transition. `None` for un-linked delegations.
    pub parent_task_id: Option<String>,
    /// Live workflow progress snapshot. Populated only for background tasks
    /// launched by the Workflow tool (task_id starts with `bgwf_`). Updated
    /// incrementally by `EngineEvent::WorkflowProgress` handlers in the event
    /// loop. `None` for regular subagent/swarm background tasks.
    pub workflow_progress: Option<crate::workflows::WorkflowTaskProgress>,
    /// Wall-clock of the agent's most recent observable activity — a
    /// streamed chunk, a tool call, or a token/usage update. Drives the
    /// fan's `stalled Ns` flag: a *running* agent whose `last_activity_at`
    /// is older than the stall threshold has gone quiet (wedged on a long
    /// tool, rate-limited, or hung) and gets an amber marker so it stands
    /// out from agents that are actually progressing. Set at spawn and
    /// refreshed by the same handlers that bump `last_tool`/token counts.
    pub last_activity_at: std::time::Instant,
}

impl BackgroundTask {
    /// Upper bound on retained per-agent log entries (`messages`) and
    /// structured transcript entries (`chat_messages`). Long agent runs
    /// emit one entry per tool call / stream chunk batch; without a cap
    /// both vecs grow for the agent's whole lifetime and are retained
    /// until the session ends — a steady RSS leak with many or
    /// long-running agents. 500 entries comfortably covers the
    /// collapse/expand UI and task-view rendering, which only ever show
    /// the tail.
    pub const LOG_CAP: usize = 500;

    /// Total tokens attributed to this agent across all four usage buckets
    /// (input + cache-read + cache-write + cumulative output). The roster
    /// surfaces (agents fan, teammates panel, task detail) all show this; it
    /// lived as four hand-rolled `saturating_add` copies before. `saturating`
    /// so a corrupt counter can't overflow-panic the render path.
    pub fn total_tokens(&self) -> u64 {
        self.latest_input_tokens
            .saturating_add(self.latest_cache_read_tokens)
            .saturating_add(self.latest_cache_write_tokens)
            .saturating_add(self.cumulative_output_tokens)
    }

    /// Append to the raw string log, dropping oldest entries over the cap.
    pub fn push_log(&mut self, entry: String) {
        self.messages.push(entry);
        if self.messages.len() > Self::LOG_CAP {
            let excess = self.messages.len() - Self::LOG_CAP;
            self.messages.drain(..excess);
        }
    }

    /// Append to the structured transcript, dropping oldest entries over
    /// the cap.
    pub fn push_chat(&mut self, msg: crate::types::ChatMessage) {
        self.chat_messages.push(msg);
        if self.chat_messages.len() > Self::LOG_CAP {
            let excess = self.chat_messages.len() - Self::LOG_CAP;
            self.chat_messages.drain(..excess);
        }
    }

    /// Append a streamed agent text chunk, coalescing with the previous
    /// entry when both arrived in rapid succession AND the previous entry
    /// doesn't end with a newline — so a single conceptual paragraph
    /// streamed across many deltas renders as one paragraph instead of one
    /// entry per delta. New entries go through the capped push helpers.
    pub fn append_chunk(&mut self, text: String) {
        let coalesce = self
            .messages
            .last()
            .map(|s| !s.ends_with('\n') && !s.starts_with('['))
            .unwrap_or(false);
        if !coalesce {
            self.push_log(text.clone());
            // Start a new assistant message in the structured log.
            self.push_chat(crate::types::ChatMessage::assistant(text));
            return;
        }
        if let Some(last) = self.messages.last_mut() {
            last.push_str(&text);
        }
        // Also coalesce into the structured chat_messages.
        let chat_coalesce = self
            .chat_messages
            .last()
            .map(|m| m.role == crate::types::Role::Assistant)
            .unwrap_or(false);
        if !chat_coalesce {
            self.push_chat(crate::types::ChatMessage::assistant(text));
            return;
        }
        if let Some(msg) = self.chat_messages.last_mut() {
            if let Some(crate::types::MessagePart::Text(t)) = msg.parts.last_mut() {
                t.push_str(&text);
            } else {
                msg.parts.push(crate::types::MessagePart::Text(text));
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundAgentCompletion {
    pub task_id: crate::ids::TaskId,
    pub description: String,
    pub status: crate::types::TaskLifecycle,
    pub body: String,
}

/// A view-facing side effect requested by engine code. Engine handlers must
/// never touch view state (scroll, render caches, textarea) directly — they
/// push effects here and the frontend drains them after each dispatch.
/// Headless frontends ignore most of these. Generalizes the
/// `compact/engine.rs` progress-callback pattern to the whole engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineEffect {
    /// Streamed content was appended to the transcript. Frontends that are
    /// following the bottom should re-pin their viewport.
    TranscriptAppended,
    /// Streaming content was finalized into the transcript — any
    /// streaming-specific render caches are stale and must be invalidated.
    StreamingFinalized,
    /// Pin the viewport to the bottom of the transcript (hard transcript
    /// resets: compaction replace, session load, queued-prompt submit).
    ScrollToBottom,
    /// The provider model catalog / account profile changed — frontends with
    /// an open model picker should rebuild its row set and query cache.
    ModelsRefreshed,
    /// Fresh tool output landed in the transcript — frontends reset
    /// path-yank-style cursors so shortcuts start from the newest refs.
    ToolOutputArrived,
    /// The engine switched sessions (load/clear/continue) — frontends reset
    /// per-session view state (task panel selection, drill-down, token gauge).
    SessionSwitched,
    /// The local prompt-rewrite gate proposed a reworded prompt. Frontends show
    /// original→rewrite + rationale and let the user accept/reject/edit before
    /// re-submitting. Never applied silently.
    PromptRewriteProposed {
        original: String,
        rewrite: String,
        rationale: String,
        /// One-line restatement of the legitimate goal, persisted as a few-shot
        /// exemplar when the user accepts (experience replay).
        original_intent: String,
    },
}

pub struct EngineState {
    /// When this engine instance was constructed — the run clock for
    /// session-duration reporting (/bug, /status).
    pub started_at: Instant,
    /// View-facing side effects queued by engine handlers during event
    /// dispatch; the frontend drains these after every `handle_engine_event`
    /// call (see `apply_engine_effects`).
    pub effects: Vec<EngineEffect>,
    /// Verbosity / formatting style for assistant replies. Routes
    /// through `OutputStyle::system_prompt_suffix()` at request-build
    /// time. `Default` is the no-op (current jfc behaviour).
    pub output_style: crate::output_style::OutputStyle,
    pub messages: Vec<ChatMessage>,
    pub streaming_text: String,
    pub streaming_reasoning: String,
    /// v126 `responseLengthRef`: a single monotonic "response length"
    /// accumulator, displayed as `/4` for the spinner's live token count.
    /// It grows two ways, mirroring cli.js's `i54` reducer:
    ///   * **chars** — every text/reasoning/tool-input delta adds its byte
    ///     length (smooth, per-delta growth);
    ///   * **wire floor** — each `message_delta` usage event floors it up to
    ///     `streaming_response_baseline + output_tokens*4`, the char-equivalent
    ///     of the server's cumulative output count.
    ///
    /// Because chars keep adding *on top of* each wire correction, the
    /// displayed `bytes/4` advances smoothly instead of pinning flat to wire
    /// and jumping ~50 every time a batched usage delta lands. Reset at the
    /// start of each streaming turn; persists across a turn's sub-streams.
    pub streaming_response_bytes: usize,
    /// `responseLengthRef` value captured at the start of the current
    /// sub-stream (cli.js `responseLengthBaseline`). The wire floor is
    /// `baseline + output_tokens*4` because `output_tokens` is the *current
    /// message's* cumulative count, which restarts at 0 each sub-stream — the
    /// baseline carries forward what earlier sub-streams already accumulated.
    /// Captured when a usage event reports fewer output tokens than the last
    /// (a new message began); self-heals to 0 if it ever exceeds the live
    /// accumulator (a missed turn-boundary reset).
    pub streaming_response_baseline: usize,
    /// True output tokens displayed in the status row, straight from the wire
    /// `message_delta` usage events (no chars/4 estimate). Accumulated by real
    /// per-event deltas, so it holds steady between usage events then steps by
    /// the exact increment. Tracks the same lifecycle as
    /// `streaming_response_bytes` (reset together) — just true values instead
    /// of an estimate. Updated in `stream_usage.rs`.
    pub turn_output_tokens: u64,
    /// Loop guard for the refusal-fallback: set once this turn has already
    /// switched to the fallback model after a refusal, so a second refusal
    /// doesn't trigger an endless model-swap loop. Reset at each new user turn.
    pub refusal_fallback_attempted: bool,
    /// Bounded retry counter for content-bearing refusals on the SAME model.
    /// Under transient provider degradation a refusal stop is frequently
    /// spurious and clears on a plain resend, so we retry a couple of times
    /// before surfacing the dead-stop. Reset at each new user turn and when a
    /// turn produces a normal (non-refusal) result.
    pub refusal_resend_count: u32,
    /// Cumulative thinking-token estimate for the turn, summed across all
    /// thinking blocks. Populated during extended-thinking phases (live thinking)
    /// or redacted-thinking blocks. Reset at the start of each streaming turn.
    /// Displayed separately from output tokens.
    pub streaming_thinking_tokens: u64,
    /// Last cumulative `thinking_delta.estimated_tokens` value seen *within the
    /// current thinking block*. The API sends `estimated_tokens` as a running
    /// total per block (e.g. 100, 250, 400), so — exactly like `last_usage_output`
    /// for output tokens — we accumulate the *delta* against this baseline, not
    /// the raw total (which would triple-count: 100+250+400 instead of 400). A
    /// new block restarts the total lower, which we detect and treat as a fresh
    /// block's growth from zero.
    pub last_thinking_estimate: u32,
    /// Tokens freed by the most recent compaction, pending forward to the next
    /// outbound request as `context_hint.target_tokens_saved`
    /// (context-hint-2026-04-09 beta). Set by the CompactionDone handler;
    /// drained into `StreamRequestOverrides` on the next send so the hint
    /// fires exactly once. `None` when no compaction is pending acknowledgement.
    pub pending_context_hint_tokens_saved: Option<u64>,
    /// Visible while a provider is silently retrying a transient network/API
    /// failure. The spinner replaces its normal cycling verb with this code
    /// until the next real stream byte arrives.
    pub network_recovery_status: Option<NetworkRecoveryStatus>,
    pub network_recovery_attempts: u32,
    /// Latest pre-content request lifecycle phase. Unlike
    /// `network_recovery_status`, this also covers normal quiet windows:
    /// context assembly, first-byte wait, stream-open/no-event, and fallback
    /// attempts. Cleared on the first real stream output/tool/done/error.
    pub stream_lifecycle: Option<StreamLifecycleStatus>,
    /// Latest status.claude.com heartbeat. This is intentionally
    /// best-effort UI context, not a dependency for provider requests.
    pub claude_status: Option<crate::claude_status::ClaudeStatusSnapshot>,
    pub claude_status_error: Option<String>,
    pub streaming_assistant_idx: Option<usize>,
    /// Active model-stream id. Stream tasks stamp their events with this id so
    /// late events from superseded tasks can be dropped before they mutate the
    /// current transcript.
    pub active_stream_id: Option<u64>,
    next_stream_id: u64,
    /// Last message ID from the API response, for `diagnostics.previous_message_id`.
    pub last_response_id: Option<String>,
    pub is_streaming: bool,
    /// Updated on every inbound stream event (chunk, tool delta, done, error).
    /// Used by the watchdog to detect stuck `is_streaming` flags — if no
    /// stream activity arrives within `STREAM_WATCHDOG_TIMEOUT`, the flag is
    /// force-reset to stop the 30fps animation loop.
    pub last_stream_event_at: Option<Instant>,
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
    /// Cumulative session cost (USD) snapshotted at the start of the current
    /// user turn. The end-of-turn footer subtracts this from the live
    /// cumulative total so it shows the *per-turn* cost ("Cooked for 2m /
    /// $0.04") rather than the whole session's running spend. Captured at the
    /// same points `turn_started_at` is set to a fresh `Some(now)`, so it
    /// survives across agentic-loop sub-streams within one turn.
    pub turn_start_cost: f64,
    /// Files the assistant edited during the current user turn (Edit/Write
    /// `file_path`s), accumulated as tools complete and reset on each fresh
    /// user submit. Drives `/turn-diff`, which scopes a `git diff` to just
    /// these paths so you can review one agentic step in isolation from the
    /// whole working tree.
    pub turn_edited_files: std::collections::BTreeSet<String>,
    /// Background auto-review dispatch state. Keeps a per-session signature of
    /// the last changed-file diff reviewed so repeated EndTurn cleanup passes
    /// do not queue the same review again.
    pub auto_review: crate::auto_review::AutoReviewState,
    /// Number of API round-trips in the current user turn (incremented each
    /// time `continue_agentic_loop` fires). Resets on each user submission.
    /// Used to enforce a max-turns safety limit (default 200, matching CC
    /// 2.1.144's `maxTurns`). Without this, a model stuck in a retry loop
    /// runs indefinitely, burning unlimited API credits.
    pub agentic_turn_count: u32,
    /// Periodic "persist what you learned" nudge (ported from Hermes Agent's
    /// `_memory_nudge_interval`). Advanced once per genuine user submit; when it
    /// fires it queues a memory-persist `<system-reminder>` for the next request.
    pub memory_nudge: crate::system_reminder::MemoryNudge,
    /// Consecutive self-continuations (auto-driving the next step without a
    /// user "continue") since the last real user submit. Capped by
    /// `max_self_continuations` to prevent a runaway loop when the model keeps
    /// stalling. Reset to 0 on every genuine user submit.
    pub self_continuation_count: u32,
    /// Consecutive empty-but-billed turns we've discarded and re-streamed
    /// since the last turn that produced real content. A degraded provider
    /// stream can bill output tokens yet emit no text/tools/reasoning (renders
    /// as a blank `assistant (Brewed …)` bubble and leaves an `empty_message`
    /// invariant violation on save). `handle_stream_done` removes that empty
    /// message and re-streams; this counter caps the retries so a persistently
    /// broken provider can't loop forever. Reset to 0 whenever a turn produces
    /// content and on every genuine user submit.
    pub empty_billed_resend_count: u32,
    /// Wall-clock instant of the most recent text/reasoning delta. The
    /// spinner derives its honest silence signal from this: a `quiet Ns`
    /// chip past `QUIET_CHIP_SECS` (8s) and a row-dim past `QUIET_DIM_SECS`
    /// (30s). No fabricated "almost done" reassurance — just measured
    /// time-since-last-byte.
    pub streaming_last_token_at: Option<Instant>,
    /// Rolling (elapsed, output_tokens) samples for the live tokens/sec
    /// readout. Fed by engine stream handlers; rendered by the frontend.
    pub token_rate_samples: std::collections::VecDeque<(std::time::Duration, u64)>,
    /// Per-turn total token counts for the sidebar sparkline. Fed by the
    /// stream-done handler; rendered by the frontend.
    pub token_history: std::collections::VecDeque<u64>,
    /// Most recently active agent/teammate task id — written by team/task
    /// handlers, read by the frontend to focus the fan UI.
    pub last_active_agent_task: Option<String>,
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
    pub provider: Arc<dyn Provider>,
    pub providers: Vec<Arc<dyn Provider>>,
    pub model: ModelId,
    /// Recently selected models (most recent first, max 5). Shown at the
    /// top of the model picker for quick switching. Persisted to
    /// `~/.config/jfc/recent_models.json`.
    pub recent_models: Vec<String>,
    pub cwd: String,
    pub pending_approval: Option<PendingApproval>,
    /// FIFO of tool calls waiting for approval behind the current one. When the
    /// model emits multiple approvable tools in one turn (six `bash` calls in a
    /// single response is common), only the first one fits in `pending_approval`
    /// — the rest queue here. After the user decides on the current tool, the
    /// next is dequeued into `pending_approval`. Without this, subsequent tools
    /// were silently dropped, leaving the conversation with a tool_use that
    /// had no matching tool_result and a stalled agentic loop.
    pub approval_queue: std::collections::VecDeque<ToolCall>,
    /// Active `AskUserQuestion` modal, if any. While `Some`, the agentic loop
    /// is parked (the question is the turn's terminal act) and key input is
    /// routed to the question handler. Resolved by submit (answer → tool_result)
    /// or Esc (declined). Unlike `approval_queue`, questions don't queue —
    /// `AskUserQuestion` is a turn-ending tool, so at most one is ever pending.
    pub pending_question: Option<crate::app::PendingQuestion>,
    /// Active MCP elicitation requests waiting for user input.
    /// Multiple elicitations can queue up (one per in-flight MCP tool call).
    /// The TUI renders the first one as a modal; subsequent ones wait.
    pub pending_elicitations:
        std::collections::VecDeque<jfc_core::mcp_elicitation::ElicitationSnapshot>,
    /// Tool calls that have been yielded to the host but are not executing yet:
    /// waiting for approval, classifier judgment, or stream_done batch
    /// dispatch. This is the TUI/remote equivalent of upstream's
    /// `deferred_tool_use` bookkeeping.
    pub deferred_tool_uses: VecDeque<DeferredToolUse>,
    /// IDs currently executing locally or server-side. Mirrors upstream's
    /// `set_in_progress_tool_use_ids` bridge events so remote/headless clients
    /// can distinguish "waiting for a result" from "idle".
    pub in_progress_tool_use_ids: HashSet<String>,
    /// Short labels for completed tool batches, exposed to remote clients as
    /// `tool_use_summary` events and retained for diagnostics.
    pub tool_use_summaries: VecDeque<ToolUseSummary>,
    /// FIFO of user prompts the user submitted while the model was streaming.
    /// v126 calls these `queued_command` attachments. They render in the
    /// transcript immediately as user messages (so the user sees their input
    /// landed) but don't go to the API until the current turn finishes.
    /// Drained by `drain_queued_prompts()` after `is_streaming` flips false
    /// AND the approval pipeline is empty. Each entry remembers whether the
    /// user typed a slash command (v126's `isMeta: true`) — those run
    /// locally on drain instead of going to the API.
    pub queued_prompts: MessageQueue,
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
    /// Wall-clock instant of the last successful session save. The
    /// status-bar render shows "✓ saved" briefly after this fires,
    /// fading after `SAVED_BADGE_TTL_MS` so the indicator doesn't
    /// linger on every render.
    pub last_session_save_at: Option<std::time::Instant>,
    /// A debounced save was requested while a recent save was still inside
    /// `session_save::MIN_SAVE_INTERVAL`. The frontend's housekeeping tick
    /// flushes it via `session_save::flush_pending_save` so the newest
    /// mid-turn state still lands on disk shortly after a burst ends.
    pub session_save_pending: bool,
    pub interrupt_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Per-turn cancellation token. Cloned into every spawned task that
    /// holds critical state (stream_response, tool dispatch, compact,
    /// session save) so an ESC×2 / interrupt can race the in-flight work
    /// against `.cancelled()` instead of waiting for the AtomicBool poll
    /// to come around. Re-minted on every fresh user turn so the previous
    /// token's cancelled state doesn't poison the next stream. wg-async
    /// pattern: tasks holding state must be explicitly cancellable, not
    /// just dropped via a flag the task may never poll.
    pub cancel_token: tokio_util::sync::CancellationToken,
    /// Abort handle for the *inner* stream-driver task spawned per user turn
    /// (the one actually running `stream_response`). Watchdog escalation: when
    /// `check_stream_watchdog` detects a hard-idle stream it cancels the token
    /// (cooperative) *and* aborts this handle (forceful). Without the forceful
    /// abort a stream task stuck in a synchronous syscall (DNS resolution,
    /// audit-log write) would survive the cancel and the next user submission
    /// would race a second concurrent stream task writing the same conversation
    /// buffer. This MUST point at the inner task, not the outer supervisor:
    /// aborting the supervisor only drops its `JoinHandle` to the inner task,
    /// which *detaches* (keeps running) rather than cancelling it. After the
    /// forceful abort the watchdog auto-retries the turn in place (bounded by
    /// `MAX_NETWORK_RECOVERY_ATTEMPTS`) rather than killing it outright — a
    /// byte-silent stall is a transient, not a logical error.
    pub active_stream_handle: Option<tokio::task::AbortHandle>,
    pub always_approved: Vec<String>,
    pub session_approved: Vec<String>,
    pub pending_tool_calls: Vec<ToolCall>,
    /// Count of auto-mode classifier verdicts still in flight. Each tool in
    /// auto-mode spawns an async classifier call (2-5s); until every verdict
    /// lands, `stream_done` must hold the turn open instead of finalizing —
    /// otherwise a late verdict finds the streaming slot already cleared and
    /// the tool is silently dropped (never dispatched, loop stalls). Reset to
    /// 0 at the start of every user turn so a verdict that never arrives
    /// (e.g. cancelled mid-classification) can't wedge the next turn.
    pub pending_classifications: usize,
    /// Tool IDs already dispatched mid-stream (safe tools that started
    /// executing while the model was still generating). stream_done
    /// skips these to avoid double-dispatch.
    pub pre_dispatched_tool_ids: std::collections::HashSet<String>,
    /// Count of eagerly-dispatched tool batches still in flight. Each eager
    /// dispatch increments this; each AllComplete event decrements it. The
    /// turn is only truly complete when this reaches 0 AND pending_tool_calls
    /// is empty.
    pub in_flight_eager_dispatches: usize,
    /// Count of dispatched local tool batches whose batch-level completion
    /// signal has not been observed yet. `pending_tool_calls` is drained when
    /// a batch starts, so it cannot distinguish "nothing is running" from
    /// "tools are running and will report later". This counter is the
    /// authoritative guard against finalizing or rescuing the turn while a
    /// regular, approval, classifier, advisor, or eager dispatch batch is
    /// still expected to emit `ToolEvent::AllComplete`.
    pub in_flight_tool_batches: usize,
    /// Metadata for the provider request currently streaming or most recently
    /// finished. Set by `StreamEvent::RequestMetadata` before the first byte
    /// arrives; cleared when the turn truly ends. Used to detect narration-only
    /// EndTurn responses on prompts that were expected to call tools.
    pub current_stream_request: Option<StreamRequestMetadata>,
    pub max_context_tokens: usize,
    /// Set by `/compact` slash command. Picked up by the main loop next time
    /// it would otherwise check `compact::should_compact` — forces compaction
    /// regardless of token level. Cleared after the compact runs (success or
    /// not) so a single `/compact` invocation triggers exactly one attempt.
    pub force_compact_pending: bool,
    /// Set when a stream's `Done` event carries `StopReason::PauseTurn`
    /// AND the same response also produced local tools that need to
    /// run (mixed mode). The dispatch ladder in event_loop.rs sees
    /// `has_pending_tools` first and routes to local-tool execution,
    /// shadowing the PauseTurn branch. Without this flag, the
    /// post-tool `ToolEvent::AllComplete` handler defaults to
    /// `continue_agentic_loop` which routes through
    /// `build_provider_messages_with_tool_results` → injects the
    /// "Continue from where you left off." synthetic-user filler that
    /// Anthropic's `pause_turn` protocol explicitly forbids
    /// (cli.js v142:622686). When set, AllComplete instead calls
    /// `continue_after_pause_turn` so the resume goes out with the
    /// trailing `server_tool_use` as the resumption cue, intact.
    ///
    /// Cleared the moment the resume dispatches OR the turn ends
    /// without resuming (no pending tools, no pending approvals,
    /// EndTurn) — single-shot per pause_turn occurrence so a later
    /// non-pause_turn turn doesn't accidentally inherit the routing.
    pub pending_pause_turn_resume: bool,
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
    /// Whether a speculative (precomputed) compact has already been
    /// triggered for this session. Prevents repeated spawns. Resets
    /// on compaction completion or /clear.
    pub speculative_compact_fired: bool,
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
    pub tool_ctx: ToolContext,
    pub dedup_cache: Arc<Mutex<ReadDedupCache>>,
    /// Cache of `Provider::fetch_models()` results, keyed by `Provider::name()`. Populated
    /// asynchronously at startup; consulted by the picker before falling back to the
    /// provider's static `available_models()`.
    pub provider_models: HashMap<ProviderId, Vec<ModelInfo>>,
    /// OAuth seat tier from `/api/oauth/profile` (e.g. `"opus"`, `"opusplan"`,
    /// `"claude-opus-4-6[1m]"`). Drives `apply_seat_tier_filter()` in the picker.
    pub seat_tier: Option<String>,
    /// OAuth subscription type (`"max"`, `"pro"`, `"enterprise"`) — shown in the
    /// status bar so the user knows which plan they're billing against.
    pub subscription_type: Option<String>,
    /// Account email from the OAuth profile, surfaced in the status bar.
    pub account_email: Option<String>,
    /// Active session id (set when the user picks one or starts a new one).
    pub current_session_id: Option<crate::ids::SessionId>,
    /// v126 auto-mode classifier config — `enabled: true` routes every tool
    /// call through the LLM classifier instead of prompting the user.
    /// Loaded from `~/.config/jfc/settings.json` at startup.
    pub auto_mode: AutoModeConfig,
    /// v126 permission mode — controls how tool execution is gated.
    pub permission_mode: PermissionMode,
    /// v126 task/todo store. Persists to `~/.config/jfc/tasks/<session>.json`
    /// so todos survive session resume and compaction. Reused across the
    /// agent's turns; the slash commands `/task-*` poke it directly.
    pub task_store: std::sync::Arc<jfc_session::TaskStore>,
    /// Records when each task transitioned to `Completed` so the footer can
    /// keep showing them for 30 seconds with dimmed/strikethrough styling.
    pub task_completion_times: HashMap<TaskId, Instant>,
    /// Transient per-session map of task_id → current activity description.
    /// Updated by the tool execution loop to show what an in_progress task is
    /// doing (e.g. "Running bash: cargo test", "Reading src/main.rs").
    pub task_activities: HashMap<TaskId, String>,
    /// Plan verification gate: when true, the plan has already been verified
    /// for the current batch of pending tasks. Reset to false whenever new
    /// tasks are created via TaskCreate.
    pub plan_verified_this_batch: bool,
    /// Cache of task-batch decompositions keyed by a normalized goal signature.
    /// The factory consults it during plan verification to surface a similar
    /// prior plan as advisory context (plan reuse).
    pub plan_cache: jfc_core::PlanCache,
    pub last_usage_input: u32,
    pub last_usage_output: u32,
    /// Auto-expiring toast queue. Pruned every `UiEvent::Tick`. Pushed via
    /// `EngineEvent::Control(ControlEvent::Notice)` from anywhere in the app (compaction milestones,
    /// session save success, classifier blocks). Mirrors v126's terminal
    /// `notification()` for non-blocking status surfacing.
    pub toasts: Vec<crate::toast::Toast>,
    /// Active LSP diagnostics, keyed by file path. Rendered as a one-line
    /// `Found N new diagnostic issue(s) in M file(s) (ctrl+o to expand)`
    /// row above the spinner when non-empty. Updated by
    /// `EngineEvent::Provider(ProviderEvent::DiagnosticsUpdated)`. Mirrors v126 cli.js:338030-338040.
    pub diagnostics: Vec<crate::diagnostics::DiagnosticEntry>,
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
    /// Sampling-temperature pin for this session. `/temp <0..2>` flips it;
    /// `prepare_stream_request` only forwards it when the selected provider /
    /// model request shape can legally carry temperature.
    pub temperature_state: crate::exploration::TemperatureState,
    /// Adaptive exploration controller. It fills in effort or temperature
    /// only when neither `/effort` nor `/temp` has pinned a knob.
    pub exploration_state: crate::exploration::ExplorationState,
    /// Last time we fired the OnHeartbeat hook. Tick handler checks this
    /// every 80ms and fires the hook at most once every 30s when idle.
    pub last_heartbeat_at: Option<std::time::Instant>,
    /// Last MCP refresh counter we observed. Tick handler compares this
    /// against `mcp::registry::refresh_counter()` to detect inbound
    /// `notifications/tools/list_changed` and emit a toast + reminder.
    pub last_mcp_refresh_seen: u64,
    /// Last file-watcher change-counter we observed. Tick handler
    /// compares against `file_watcher::change_counter()` to detect
    /// CLAUDE.md / agents / settings edits and prepend a system-
    /// reminder on the next outbound prompt.
    pub last_file_watcher_seen: u64,
    /// Queue of `<system-reminder>` bodies posted by background events
    /// (file watcher, MCP `tools/list_changed`, …) awaiting consumption
    /// by the next outbound stream request. The drain happens in
    /// `prepare_stream_request`, so the reminder lands in the wire
    /// payload exactly once and `app.messages` is never mutated by an
    /// FS-rate signal. Dedup-on-push collapses N filesystem events
    /// between turns into one reminder.
    pub pending_background_reminders: Vec<String>,
    /// Message indices the user pinned via `/pin <idx>`. Compaction
    /// preserves pinned messages verbatim regardless of token pressure.
    /// Stored as indices into `messages` rather than a flag on
    /// ChatMessage so we don't have to touch every construction site.
    pub pinned_message_indices: std::collections::HashSet<usize>,
    /// `/fast` toggle — mirrors Claude Code v2.1.139's `/fast` command (Alt+O).
    /// When true, the `fast-mode-2026-02-01` beta header is added to every
    /// Anthropic API request, routing to the lower-latency inference path.
    pub fast_mode: bool,
    /// Per-session FIFO of tool mutations the user can `/undo`. Each
    /// entry captures `(file_path, prev_content, op_label)` before the
    /// tool runs. Capped at 100 entries (the oldest gets dropped). New
    /// entries push to the back; /undo pops the back (most recent
    /// first).
    /// `(tool_id, line)` captured from `ToolOutputChunk`. `stream.rs`
    /// drains this on the next outbound request so the model sees what
    /// Highest budget threshold the user has been warned about so far this
    /// session. 0 = no warnings yet, 80 = 80% warning shown, 100 = 100%
    /// warning shown. Prevents toast spam when the same threshold is
    /// crossed multiple times across re-renders.
    pub cost_budget_warned_at: u8,
    /// Insertion-ordered map of subagent background tasks. IndexMap preserves
    /// the spawn order so tab cycling, footer tabs, and "jump to latest" all
    /// operate on a stable, chronological ordering instead of random HashMap
    /// iteration order.
    pub background_tasks: IndexMap<String, BackgroundTask>,
    pub mcp_servers: Vec<crate::types::McpServerInfo>,
    pub lsp_servers: Vec<crate::types::LspServerInfo>,
    pub usage_by_model: HashMap<String, crate::types::ModelUsage>,
    /// Cached snapshot of the active Anthropic OAuth account's utilization
    /// (refreshed on every successful stream + every ~10s tick). Drives the
    /// ribbon's "5h 47% / 7d 12% · opus weekly" display. `None` for non-
    /// OAuth providers.
    pub anthropic_account_snapshot: Option<crate::providers::anthropic_accounts::AccountSnapshot>,
    /// Last instant we re-queried the rotation manager. Throttle to once
    /// every ~10s so the ribbon stays current without burning a lock per
    /// frame at 30fps.
    pub anthropic_snapshot_refreshed_at: Option<std::time::Instant>,
    /// Last instant we ran a proactive Anthropic token-refresh sweep. Throttled
    /// to once every ~60s so accounts stay "warm" (refreshed before expiry)
    /// without hammering the token endpoint.
    pub anthropic_sweep_at: Option<std::time::Instant>,
    /// Last wall-clock time the UI re-read `daemon-state.json` to refresh
    /// counters for detached background workers. Throttled in the Tick
    /// handler so we don't hammer the JSON file every frame.
    pub last_detached_sync_at: Option<std::time::Instant>,
    /// Cached `daemon-state.json` mtime from the last successful parse.
    /// Used to skip the (potentially MB-sized) read+parse when the file
    /// hasn't been touched by any background worker since last poll —
    /// this is the primary CPU-burn fix for sessions with hundreds of
    /// historical background agents accumulated in the state file.
    pub last_detached_state_mtime: Option<std::time::SystemTime>,
    /// Detached background agents that transitioned to Completed/Failed since
    /// the last user submit. This is the model-facing one-shot handoff: UI
    /// `TaskStatus` parts persist for rendering, but their summaries must not
    /// be replayed from transcript history on every provider request.
    pub pending_background_agent_completions: Vec<BackgroundAgentCompletion>,
    /// Compatibility counter for detached background completions since the
    /// last user submit. Kept in sync with `pending_background_agent_completions`
    /// for existing callers, but the queued summaries are the source of truth.
    pub background_tasks_completed_since_last_turn: u32,
    /// Whether a background agent has transitioned to a *terminal* state
    /// (Completed / Failed / Cancelled) **during this process** — set at the
    /// three real transition sites (live `TaskCompleted` / `TaskFailed`
    /// handlers and the daemon-sync poll). Crucially NOT set by
    /// `restore_persistent_background_agents`, which seeds already-terminal
    /// agents from a *prior* session on `--continue`.
    ///
    /// Gates the Case-2 auto-wake in `maybe_resume_after_background`: without
    /// it, launching `jfc --continue` on a session that had completed
    /// background agents fired an unsolicited (billed) summary turn at startup
    /// before the user typed anything — the restored terminal agents tripped
    /// `all_bg_done` while `turn_started_at` was None. Auto-wake should only
    /// fire for agents that actually finished *while the user was here*.
    pub observed_bg_terminal_transition_this_process: bool,
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
    /// `slate::SlateRouter::route` and `crates/jfc/src/slate.rs`.
    pub slate: Option<SlateRouter>,
    /// Advisor session for `/advisor <query>` (see `crate::advisor`).
    /// `None` until the user invokes `/advisor` for the first time —
    /// mints lazily so the cost is paid only by users who actually use
    /// the feature. The session owns its own model id, transcript, and
    /// token budget; budget exhaustion returns Err and the user must
    /// reset (e.g. via `/clear`) to get a fresh budget.
    pub advisor_session: Option<crate::advisor::AdvisorSession>,
    /// Gate for local advisor access. Startup enables it by default through the
    /// active model unless the user opts out with `advisor_enabled = false`,
    /// `--no-advisor`, or `JFC_ADVISOR_DISABLED=1`. When false, manual advisor
    /// queries surface a hint instead of running.
    pub advisor_enabled: bool,
    /// Opt-in: route the high-stakes session-goal verdict through the model
    /// Council (active model + advisor model must agree) instead of a single
    /// model. Off by default; set via config `council_verdict = true` or the
    /// `JFC_COUNCIL_VERDICT` env var. Requires the advisor to be available; with
    /// no second model it transparently degrades to the single-model evaluator.
    pub council_verdict_enabled: bool,
    /// Active local/client-side advisor model. When set, JFC advertises the
    /// normal `Advisor` tool and executes it through the local provider path,
    /// returning the advisor reply as a regular tool result.
    pub local_advisor_model: Option<ModelId>,
    /// Optional provider prefix for the local advisor model. Preserves
    /// `provider/model` config so Advisor can run through OpenAI, OpenWebUI,
    /// LiteLLM, etc. instead of assuming the active chat provider.
    pub local_advisor_provider: Option<jfc_provider::ProviderId>,
    /// Active Anthropic server-side advisor model. This is distinct from the
    /// local parallel `/advisor <query>` session above; when set, outbound
    /// Anthropic requests advertise the `advisor` server tool.
    pub server_advisor_model: Option<ModelId>,
    /// Brief mode — when `true`, the renderer hides plain assistant text
    /// from the main view; only `SendUserMessage` tool output and explicit
    /// proactive messages are surfaced. Toggled via `/brief`. Mirrors
    /// Claude Code v2.1.142+'s `tengu_brief_mode_enabled` setting.
    pub brief_mode: bool,
    /// Active autonomous loop state — set when `/loop` is started, cleared
    /// when the loop stops. Tracks tick counts + loop.md content so the
    /// renderer can show "loop active" and the wakeup handler can supply
    /// the right preamble. See `crate::autonomous_loop`.
    pub autonomous_loop: Option<crate::autonomous_loop::AutonomousLoopState>,
    /// Active speculation session — set when prompt-suggestion speculation
    /// is running, cleared on accept/discard. See `crate::speculation`.
    pub active_speculation_id: Option<String>,
    /// Per-session accumulated speculation stats (time saved, accept/discard counts).
    pub speculation_stats: crate::speculation::SpeculationStats,
    /// Bash sandbox configuration (bwrap network/filesystem isolation).
    /// When `enabled = true` and bwrap is present, bash commands are wrapped.
    pub bash_sandbox: crate::sandbox::BashSandboxConfig,
    /// Local prompt-rewriter / over-refusal mitigation. `None` (the default)
    /// means the feature is off and `submit_prompt` sends prompts unchanged;
    /// see `crate::runtime::prompt_rewrite_gate`.
    pub prompt_rewrite: Option<jfc_config::PromptRewriteConfig>,
    /// v137 `/goal <condition>` — session-scoped stop condition. When
    /// `Some`, the agentic loop will not let the agent settle on
    /// `EndTurn` until the evaluator (see `crate::goal::evaluate`)
    /// returns `ok=true`. The struct carries iteration counter +
    /// set-at timestamp + last unmet reason so the UI can show
    /// progress and the loop can refuse to spin forever.
    pub goal: Option<crate::goal::ActiveGoal>,
    /// True while a goal evaluator call is in flight. Prevents the
    /// agentic loop from racing two evaluators against the same
    /// EndTurn (which would double-charge tokens and could disagree).
    pub goal_evaluator_in_flight: bool,
    /// Files pinned into the system prompt (survive compaction).
    /// Auto-populated from files that are re-read after every compaction.
    pub pinned_files: Vec<std::path::PathBuf>,
    /// Tracks how many times each file is re-read after compaction.
    /// When a file exceeds 3 re-reads post-compact, it's promoted to pinned_files.
    pub post_compact_reads: std::collections::HashMap<std::path::PathBuf, u32>,
    /// Throttle for idle_prefetch: last time a prefetch batch was fired.
    pub last_prefetch_at: std::time::Instant,
    /// Number of prefetch reads currently in-flight (capped at 2).
    pub prefetch_in_flight: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// Cached git repository root. `None` = not yet resolved.
    /// `Some(None)` = resolved, not in a git repo.
    /// `Some(Some(path))` = resolved git root directory.
    pub git_root: Option<Option<std::path::PathBuf>>,
    /// Estimated token count of the system prompt from the last stream
    /// request. Used by the compaction handler to add overhead to the
    /// post-compact `approx_tokens` estimate (system prompt + tool defs
    /// are invisible to the message-only local estimate).
    pub last_system_prompt_len: Option<usize>,
    /// Gauge ceiling immediately after a compaction.
    ///
    /// Anthropic's prompt cache survives a compaction (5-min TTL), so the next
    /// request's `cache_read_tokens` still reflects the *pre-compaction* prefix
    /// — which would snap the context gauge right back up to e.g. 750k even
    /// though the conversation was just compacted down. While this is `Some`,
    /// `StreamUsage` clamps the gauge to this post-compact estimate so a stale
    /// cache read can't re-inflate it. Cleared once a real `cache_write`
    /// confirms the new (smaller) prefix has been re-cached.
    pub post_compact_token_ceiling: Option<usize>,

    // ── CLI-injected configuration ────────────────────────────────────
    // These fields are populated from the CLI flags parsed in `cli::run`
    // and threaded into `App` before the first event-loop tick. The
    // callers that *consume* these values (stream builder, permission
    // gate, session save, …) are wired in follow-on work — marking
    // `` until then keeps the build clean.
    /// `--max-turns`: ceiling on agentic-loop iterations per user turn.
    pub max_turns: Option<u32>,

    /// `--max-budget-usd`: hard session spend cap in USD.
    pub max_budget_usd: Option<f64>,

    /// `--allowed-tools`: parsed allowlist of tool names.
    pub allowed_tools: Vec<String>,

    /// `--disallowed-tools`: parsed denylist of tool names.
    pub disallowed_tools: Vec<String>,

    /// Tools disallowed by CLAUDE.md frontmatter (`disallowed-tools` key).
    /// Refreshed each time the hierarchy is loaded (every turn).
    pub claudemd_disallowed_tools: Vec<String>,

    /// Additional system-prompt text injected via `--system-prompt` or
    /// `--system-prompt-file`.
    pub cli_system_prompt: Option<String>,

    /// `--dangerously-skip-permissions`: bypass every permission gate.
    pub dangerously_skip_permissions: bool,

    /// `--add-dir`: extra directories added to the search context.
    pub extra_dirs: Vec<std::path::PathBuf>,

    /// `--max-thinking-tokens`: per-turn thinking budget cap.
    pub cli_max_thinking_tokens: Option<u32>,

    /// `--thinking-display`: thinking visibility mode (`show`/`hide`/`summarize`).
    pub cli_thinking_display: Option<String>,

    /// `--no-session-persistence`: when true, skip all disk persistence.
    pub no_session_persistence: bool,

    /// `--task-budget`: token budget per task for the beta task-budgets API.
    pub cli_task_budget: Option<u64>,

    /// `--betas`: custom Anthropic beta tokens appended to native requests.
    pub custom_betas: Vec<String>,

    /// `--fine-grained-tool-streaming`: attach `eager_input_streaming` to
    /// Anthropic native tool schemas.
    pub fine_grained_tool_streaming: bool,

    /// `--strict-tool-schemas`: attach `strict: true` to Anthropic native
    /// tool schemas.
    pub strict_tool_schemas: bool,

    /// `--mcp-config`: path to an MCP configuration file.
    pub mcp_config_path: Option<std::path::PathBuf>,

    /// `--cowork`: IDE pairing mode flag.
    pub cowork: bool,

    /// ID of an active cron job created by `/babysit-prs <schedule>`.
    /// `Some(id)` means a recurring PR-status check is registered with
    /// the local daemon; `/babysit-prs stop` removes it. `None` when no
    /// PR-watch loop is active. Stored in `App` so the stop command can
    /// look the id up without round-tripping through user-visible state.
    pub babysit_prs_cron_id: Option<String>,
}

impl EngineState {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<ModelId>) -> Self {
        let providers = vec![Arc::clone(&provider)];
        let (teammate_tx, teammate_rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::swarm::runner::TeammateEvent>();
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|p| p.to_str().map(str::to_owned))
            .unwrap_or_default();
        let mut state = Self {
            started_at: Instant::now(),
            effects: Vec::new(),
            output_style: crate::output_style::OutputStyle::default(),
            messages: Vec::new(),
            streaming_text: String::new(),
            streaming_reasoning: String::new(),
            streaming_response_bytes: 0,
            streaming_response_baseline: 0,
            turn_output_tokens: 0,
            refusal_fallback_attempted: false,
            refusal_resend_count: 0,
            streaming_thinking_tokens: 0,
            last_thinking_estimate: 0,
            pending_context_hint_tokens_saved: None,
            network_recovery_status: None,
            network_recovery_attempts: 0,
            stream_lifecycle: None,
            claude_status: None,
            claude_status_error: None,
            streaming_assistant_idx: None,
            active_stream_id: None,
            next_stream_id: 0,
            last_response_id: None,
            streaming_started_at: None,
            streaming_last_token_at: None,
            token_rate_samples: std::collections::VecDeque::new(),
            token_history: std::collections::VecDeque::with_capacity(super::TOKEN_HISTORY_CAP),
            last_active_agent_task: None,
            thinking_started_at: None,
            thinking_ended_at: None,
            turn_started_at: None,
            turn_start_cost: 0.0,
            turn_edited_files: std::collections::BTreeSet::new(),
            auto_review: crate::auto_review::AutoReviewState::default(),
            agentic_turn_count: 0,
            memory_nudge: crate::system_reminder::MemoryNudge::default(),
            self_continuation_count: 0,
            empty_billed_resend_count: 0,
            is_streaming: false,
            last_stream_event_at: None,
            provider,
            providers,
            model: model.into(),
            recent_models: load_recent_models(),
            cwd,
            pending_approval: None,
            approval_queue: std::collections::VecDeque::new(),
            pending_question: None,
            pending_elicitations: std::collections::VecDeque::new(),
            deferred_tool_uses: VecDeque::with_capacity(DEFERRED_TOOL_USES_CAP),
            in_progress_tool_use_ids: HashSet::new(),
            tool_use_summaries: VecDeque::with_capacity(TOOL_USE_SUMMARIES_CAP),
            queued_prompts: MessageQueue::new(),
            worktree_count: 0,
            worktree_count_last_refresh: None,
            git_branch: None,
            git_branch_last_refresh: None,
            last_session_save_at: None,
            session_save_pending: false,
            interrupt_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            active_stream_handle: None,
            always_approved: Vec::new(),
            session_approved: Vec::new(),
            tool_ctx: ToolContext::new(),
            dedup_cache: Arc::new(Mutex::new(ReadDedupCache::new())),
            pending_tool_calls: Vec::new(),
            pending_classifications: 0,
            pre_dispatched_tool_ids: std::collections::HashSet::new(),
            in_flight_eager_dispatches: 0,
            in_flight_tool_batches: 0,
            current_stream_request: None,
            force_compact_pending: false,
            pending_pause_turn_resume: false,
            compact_suppressed: false,
            compacting_started_at: None,
            speculative_compact_fired: false,
            compacting_output_chars: 0,
            compacting_attempt_baseline: 0,
            compacting_last_progress: 0,
            max_context_tokens: DEFAULT_CONTEXT_WINDOW_TOKENS,
            provider_models: HashMap::new(),
            seat_tier: None,
            subscription_type: None,
            account_email: None,
            current_session_id: Some(jfc_session::generate_session_id()),
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
            task_store: jfc_session::TaskStore::in_memory(),
            task_completion_times: HashMap::new(),
            task_activities: HashMap::new(),
            plan_verified_this_batch: false,
            plan_cache: jfc_core::PlanCache::new(64),
            last_usage_input: 0,
            last_usage_output: 0,
            toasts: Vec::new(),
            diagnostics: Vec::new(),
            usage_apply_baseline: (0, 0, 0, 0),
            effort_state: crate::effort::EffortState::new(),
            temperature_state: crate::exploration::TemperatureState::new(),
            exploration_state: crate::exploration::ExplorationState::new(),
            last_heartbeat_at: None,
            last_mcp_refresh_seen: 0,
            last_file_watcher_seen: 0,
            pending_background_reminders: Vec::new(),
            pinned_message_indices: std::collections::HashSet::new(),
            fast_mode: false,
            cost_budget_warned_at: 0,
            background_tasks: IndexMap::new(),
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
            usage_by_model: HashMap::new(),
            anthropic_account_snapshot: None,
            anthropic_snapshot_refreshed_at: None,
            anthropic_sweep_at: None,
            last_detached_sync_at: None,
            last_detached_state_mtime: None,
            pending_background_agent_completions: Vec::new(),
            background_tasks_completed_since_last_turn: 0,
            observed_bg_terminal_transition_this_process: false,
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
            council_verdict_enabled: std::env::var("JFC_COUNCIL_VERDICT")
                .ok()
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            local_advisor_model: crate::advisor::active_local_advisor_model(),
            local_advisor_provider: crate::advisor::active_local_advisor_provider(),
            server_advisor_model: crate::advisor::active_server_advisor_model(),
            brief_mode: false,
            autonomous_loop: None,
            active_speculation_id: None,
            speculation_stats: crate::speculation::SpeculationStats::default(),
            bash_sandbox: crate::sandbox::BashSandboxConfig::default(),
            prompt_rewrite: None,
            goal: None,
            goal_evaluator_in_flight: false,
            pinned_files: Vec::new(),
            post_compact_reads: std::collections::HashMap::new(),
            last_prefetch_at: std::time::Instant::now() - std::time::Duration::from_secs(10),
            prefetch_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            git_root: None,
            last_system_prompt_len: None,
            post_compact_token_ceiling: None,
            // CLI-injected configuration: defaults are off / empty; the
            // `cli::run` entry point overwrites these after construction
            // when the user passed the corresponding flags.
            max_turns: None,
            max_budget_usd: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            claudemd_disallowed_tools: Vec::new(),
            cli_system_prompt: None,
            dangerously_skip_permissions: false,
            extra_dirs: Vec::new(),
            cli_max_thinking_tokens: None,
            cli_thinking_display: None,
            no_session_persistence: false,
            cli_task_budget: None,
            custom_betas: Vec::new(),
            fine_grained_tool_streaming: false,
            strict_tool_schemas: false,
            mcp_config_path: None,
            cowork: false,
            babysit_prs_cron_id: None,
        };
        // Open the task store — prefer project-level persistence so tasks
        // survive across ALL sessions in the same repo. Falls back to
        // per-session store only when no git root is discoverable.
        let git_root = crate::context::discover_git_root();
        if let Some(ref root) = git_root {
            state.task_store = jfc_session::TaskStore::open_project(Some(root.as_path()));
            state.git_root = Some(Some(root.clone()));
        } else if let Some(ref sid) = state.current_session_id {
            state.task_store = jfc_session::TaskStore::open(sid.as_str());
        }
        state
    }

    /// Queue a view-facing effect for the frontend to apply after this
    /// dispatch. Consecutive duplicates collapse (a streaming burst would
    /// otherwise queue hundreds of identical `TranscriptAppended`s).
    pub fn push_effect(&mut self, effect: EngineEffect) {
        if self.effects.last() != Some(&effect) {
            self.effects.push(effect);
        }
    }

    /// Push a `<system-reminder>` body onto the background-reminders
    /// queue. Dedupes by exact body — repeated filesystem events
    /// produce at most one reminder per outgoing turn. The queue is
    /// capped at [`BACKGROUND_REMINDERS_CAP`]; when full, the oldest
    /// entry is dropped before pushing. This keeps long idle sessions
    /// from accumulating an unbounded stream of unique log lines that
    /// would otherwise leak memory until the next user prompt drains
    /// the queue.
    pub fn queue_background_reminder(&mut self, body: impl Into<String>) {
        let body = body.into();
        if self
            .pending_background_reminders
            .iter()
            .any(|existing| existing == &body)
        {
            return;
        }
        if self.pending_background_reminders.len() >= BACKGROUND_REMINDERS_CAP {
            self.pending_background_reminders.remove(0);
        }
        self.pending_background_reminders.push(body);
    }

    /// Drain the background-reminders queue, transferring ownership to
    /// the caller. Called by the stream-open path to forward the
    /// reminders into `StreamRequestOverrides`. After this call the
    /// queue is empty until the next FS event arrives.
    pub fn take_background_reminders(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_background_reminders)
    }

    pub fn queue_background_agent_completion(&mut self, completion: BackgroundAgentCompletion) {
        if let Some(existing) = self
            .pending_background_agent_completions
            .iter_mut()
            .find(|existing| existing.task_id == completion.task_id)
        {
            *existing = completion;
        } else {
            if self.pending_background_agent_completions.len() >= BACKGROUND_REMINDERS_CAP {
                self.pending_background_agent_completions.remove(0);
            }
            self.pending_background_agent_completions.push(completion);
        }
        self.background_tasks_completed_since_last_turn =
            self.pending_background_agent_completions
                .len()
                .min(u32::MAX as usize) as u32;
    }

    pub fn take_background_agent_completions(&mut self) -> Vec<BackgroundAgentCompletion> {
        self.background_tasks_completed_since_last_turn = 0;
        std::mem::take(&mut self.pending_background_agent_completions)
    }

    /// Drain the pending post-compaction savings hint. Returns the value once,
    /// then clears it so the `context_hint` is only sent on the single request
    /// immediately following a compaction (matching cli.js's one-shot
    /// context-hint emission).
    pub fn take_context_hint_tokens_saved(&mut self) -> Option<u64> {
        self.pending_context_hint_tokens_saved.take()
    }

    /// Return the merged list of disallowed tools from CLI flags and
    /// CLAUDE.md frontmatter. Deduplicated with case-insensitive matching.
    pub fn effective_disallowed_tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = self.disallowed_tools.clone();
        tools.extend(self.claudemd_disallowed_tools.clone());
        // Deduplicate (case-insensitive)
        let mut seen = std::collections::HashSet::new();
        tools.retain(|t| seen.insert(t.to_lowercase()));
        tools
    }
}

/// Engine methods migrated off the old `App` god object (stage 3b).
impl EngineState {
    pub fn record_stream_activity(&mut self) {
        self.last_stream_event_at = Some(Instant::now());
    }

    pub fn begin_stream_scope(&mut self) -> u64 {
        let next = self.next_stream_id.wrapping_add(1);
        self.next_stream_id = if next == 0 { 1 } else { next };
        self.active_stream_id = Some(self.next_stream_id);
        self.next_stream_id
    }

    pub fn clear_active_stream_scope(&mut self) {
        self.active_stream_id = None;
    }

    pub fn is_stale_stream_event(&self, ev: &EngineEvent) -> bool {
        matches!(
            ev,
            EngineEvent::ScopedStream { stream_id, .. }
                if self.active_stream_id != Some(*stream_id)
        )
    }

    pub fn pipeline_busy_for_submit(&self) -> bool {
        self.compacting_started_at.is_some()
            || self.pending_approval.is_some()
            || !self.approval_queue.is_empty()
            || !self.pending_tool_calls.is_empty()
            || self.pending_classifications > 0
            || self.in_flight_eager_dispatches > 0
            || self.in_flight_tool_batches > 0
            || !self.in_progress_tool_use_ids.is_empty()
    }

    pub fn has_interruptible_work(&self) -> bool {
        self.is_streaming
            || self
                .active_stream_handle
                .as_ref()
                .is_some_and(|handle| !handle.is_finished())
            || self.turn_started_at.is_some()
            || self.pipeline_busy_for_submit()
            || self.goal_evaluator_in_flight
            || self
                .background_tasks
                .values()
                .any(|bt| bt.status.is_alive())
    }

    pub fn record_deferred_tool_use(
        &mut self,
        id: String,
        name: String,
        input_preview: String,
        reason: String,
    ) {
        if let Some(existing) = self
            .deferred_tool_uses
            .iter_mut()
            .find(|deferred| deferred.id == id)
        {
            existing.name = name;
            existing.input_preview = input_preview;
            existing.reason = reason;
            existing.queued_at = Instant::now();
            return;
        }
        if self.deferred_tool_uses.len() >= DEFERRED_TOOL_USES_CAP {
            self.deferred_tool_uses.pop_front();
        }
        self.deferred_tool_uses.push_back(DeferredToolUse {
            id,
            name,
            input_preview,
            reason,
            queued_at: Instant::now(),
        });
    }

    pub fn clear_deferred_tool_use(&mut self, id: &str) {
        self.deferred_tool_uses.retain(|deferred| deferred.id != id);
    }

    pub fn set_in_progress_tool_use_ids(&mut self, action: &str, ids: &[String]) {
        match action {
            "set" => {
                self.in_progress_tool_use_ids.clear();
                self.in_progress_tool_use_ids.extend(ids.iter().cloned());
                for id in ids {
                    self.clear_deferred_tool_use(id);
                }
            }
            "add" => {
                for id in ids {
                    self.in_progress_tool_use_ids.insert(id.clone());
                    self.clear_deferred_tool_use(id);
                }
            }
            "remove" => {
                for id in ids {
                    self.in_progress_tool_use_ids.remove(id);
                    self.clear_deferred_tool_use(id);
                }
            }
            other => {
                tracing::warn!(
                    target: "jfc::tool_state",
                    action = other,
                    ids = ?ids,
                    "unknown set_in_progress_tool_use_ids action"
                );
            }
        }
    }

    pub fn record_tool_use_summary(
        &mut self,
        summary: String,
        preceding_tool_use_ids: Vec<String>,
    ) {
        if summary.trim().is_empty() || preceding_tool_use_ids.is_empty() {
            return;
        }
        if self.tool_use_summaries.len() >= TOOL_USE_SUMMARIES_CAP {
            self.tool_use_summaries.pop_front();
        }
        self.tool_use_summaries.push_back(ToolUseSummary {
            summary,
            preceding_tool_use_ids,
            created_at: Instant::now(),
        });
    }

    pub fn check_stream_watchdog(&mut self, tx: &crate::runtime::EventSender) {
        if !self.is_streaming {
            return;
        }
        // Phase-aware idle deadline: while an extended-thinking block is open
        // the model can be legitimately byte-quiet for a long stretch, so we use
        // the lenient thinking tier; once it is responding (or is a non-thinking
        // model that never opens a thinking block) a silent wire is treated as a
        // dead socket and the tighter base tier applies.
        let thinking_live = self.thinking_started_at.is_some() && self.thinking_ended_at.is_none();
        let Some(timeout_secs) = stream_watchdog_timeout_secs(thinking_live) else {
            return;
        };
        let timed_out = self
            .last_stream_event_at
            .map(|t| t.elapsed().as_secs() >= timeout_secs)
            .unwrap_or(false);
        if !timed_out {
            return;
        }

        let streaming_assistant_idx = self.streaming_assistant_idx;
        let elapsed_secs = self
            .last_stream_event_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);

        // A watchdog timeout means the provider went *byte-silent* for
        // `timeout_secs` — not that the model is slow (every chunk/thinking/
        // usage event resets `last_stream_event_at`, so a stream that emits
        // anything never trips this). Byte-silence is a transient: a dead TCP
        // socket through NAT/LB, a proxy buffering indefinitely, a half-open
        // connection. Auto-retrying it in place (re-driving the same turn) is
        // strictly better than killing the turn and making the user press
        // Ctrl+R, and it composes with the supersession guard in
        // `handle_stream_error`: once `restart_stream_in_place` mints a fresh
        // token + sets `is_streaming = true`, the *old* task's late error
        // ("Stream timed out" / "stream task cancelled") lands on a live fresh
        // stream and is dropped as stale rather than surfacing.
        let can_retry = watchdog_retry_enabled()
            && streaming_assistant_idx.is_some()
            && self.network_recovery_attempts < crate::app::MAX_NETWORK_RECOVERY_ATTEMPTS;

        // Common teardown of the dead task. Cancel cooperatively (the drain
        // loop polls `cancel_token`) AND abort forcefully (a task wedged in a
        // blocking syscall never reaches a `.cancelled()` check). The abort
        // handle must be taken before `restart_stream_in_place` overwrites it
        // with the new inner task's handle. We do NOT mint a fresh token here:
        // the retry path lets `restart_stream_in_place` mint it (after reading
        // the old token's clones are already cancelled), and the give-up path
        // mints its own below.
        self.cancel_token.cancel();
        if let Some(handle) = self.active_stream_handle.take() {
            handle.abort();
        }

        if can_retry {
            let idx = streaming_assistant_idx.expect("can_retry checked is_some");
            let turn_started_at = self.turn_started_at;
            tracing::warn!(
                target: "jfc::app",
                elapsed_secs,
                attempt = self.network_recovery_attempts + 1,
                max = crate::app::MAX_NETWORK_RECOVERY_ATTEMPTS,
                "stream watchdog: idle stream — auto-retrying in place"
            );
            // Clear stale tool bookkeeping that may have accrued before the
            // stall (mirrors the network-error auto-retry path in
            // `handle_stream_error`). `restart_stream_in_place` re-establishes
            // the streaming fields, mints a fresh cancel token, and re-sets
            // `is_streaming = true` / `last_stream_event_at = now`.
            self.pending_tool_calls.clear();
            self.pre_dispatched_tool_ids.clear();
            self.deferred_tool_uses.clear();
            self.in_progress_tool_use_ids.clear();
            self.in_flight_eager_dispatches = 0;
            self.in_flight_tool_batches = 0;
            // Drive the recovery banner + attempt counter through the same
            // machinery as a 529/transient so the spinner shows "reconnecting"
            // and the bound is shared with network retries.
            crate::runtime::record_network_recovery(
                self,
                NetworkRecoveryProvider::Provider,
                "Stream timed out (watchdog) — reconnecting",
            );
            self.exploration_state
                .bump_for_signal(crate::exploration::ExplorationSignal::StreamRetry);
            crate::runtime::restart_stream_in_place(self, tx, idx, turn_started_at);
            return;
        }

        // Give-up path: retry disabled or attempts exhausted. Tear the turn
        // down and surface a hard error so the user can Ctrl+R, rather than
        // leaving a frozen spinner.
        tracing::warn!(
            target: "jfc::app",
            elapsed_secs,
            attempts = self.network_recovery_attempts,
            "stream watchdog: idle stream — giving up (retry disabled or exhausted)"
        );
        self.cancel_token = tokio_util::sync::CancellationToken::new();
        self.is_streaming = false;
        self.clear_active_stream_scope();
        self.streaming_started_at = None;
        self.last_stream_event_at = None;
        self.streaming_last_token_at = None;
        self.token_rate_samples.clear();
        self.thinking_started_at = None;
        self.thinking_ended_at = None;
        self.streaming_text.clear();
        self.streaming_reasoning.clear();
        self.streaming_response_bytes = 0;
        self.streaming_assistant_idx = None;
        self.current_stream_request = None;
        self.stream_lifecycle = None;
        self.turn_started_at = None;
        self.network_recovery_status = None;
        self.network_recovery_attempts = 0;
        // Clear any pending tool calls that accumulated during the
        // dead stream — they're stale and would dispatch into wrong
        // context if processed later.
        self.pending_tool_calls.clear();
        self.pre_dispatched_tool_ids.clear();
        self.deferred_tool_uses.clear();
        self.in_progress_tool_use_ids.clear();
        self.in_flight_eager_dispatches = 0;
        self.in_flight_tool_batches = 0;
        let mut removed_placeholder = false;
        if let Some(idx) = streaming_assistant_idx
            && idx < self.messages.len()
        {
            let msg = &self.messages[idx];
            let empty_stream_placeholder = msg.role == Role::Assistant
                && msg
                    .parts
                    .iter()
                    .all(|part| matches!(part, MessagePart::Text(text) if text.trim().is_empty()));
            if empty_stream_placeholder {
                self.messages.remove(idx);
                removed_placeholder = true;
            }
        }
        // Only append a hard-error message when there's a turn to attach it to.
        // If the placeholder was the whole turn (no content streamed) we still
        // surface a toast so the stopped spinner is explained.
        let error_text = "Stream timed out — the model stopped sending data and the watchdog \
                          gave up. Press Ctrl+R to retry.";
        if !removed_placeholder {
            self.messages
                .push(ChatMessage::assistant(format!("**Error:** {error_text}")));
        }
        crate::toast::push_with_cap(
            &mut self.toasts,
            crate::toast::Toast::new(crate::toast::ToastKind::Error, error_text),
        );
    }

    /// Resolve the git repository root by walking up from `cwd`.
    /// Caches the result in `self.git_root`. Call `invalidate_git_root()`
    /// on Resize to force re-resolution.
    pub fn resolve_git_root(&mut self) {
        if self.git_root.is_some() {
            return;
        }
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            if d.join(".git").exists() {
                self.git_root = Some(Some(d));
                return;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        self.git_root = Some(None);
    }

    /// Invalidate the cached git root so it will be re-resolved on next access.
    pub fn invalidate_git_root(&mut self) {
        self.git_root = None;
    }

    /// Switch to a different session id and reset all per-session state
    /// (tasks, completion-fade timers, task panel selection). Mirrors v126's
    /// new-session reset: each session has its own task bucket so tasks
    /// don't bleed across `/clear` or `/continue`.
    ///
    /// Pass `None` to mint a fresh session id; pass `Some(id)` to adopt an
    /// existing one (the session-load path through the sidebar / `/continue`).
    pub fn switch_session(&mut self, id: Option<crate::ids::SessionId>) {
        let old_id = self.current_session_id.clone();
        let new_id = id.unwrap_or_else(jfc_session::generate_session_id);
        tracing::info!(
            target: "jfc::app",
            old_session_id = ?old_id,
            new_session_id = %new_id,
            "switch_session"
        );
        self.current_session_id = Some(new_id.clone());
        self.clear_active_stream_scope();
        // Mirror the constructor's store choice: inside a git repo the
        // project-level store (<root>/.jfc/tasks.json) survives across ALL
        // sessions; only fall back to the per-session file without one.
        // Re-opening per-session unconditionally here silently dropped
        // project tasks on every /clear // /continue // session load.
        self.task_store = match self.git_root.as_ref().and_then(|r| r.as_ref()) {
            Some(root) => jfc_session::TaskStore::open_project(Some(root.as_path())),
            None => jfc_session::TaskStore::open(new_id.as_str()),
        };
        self.task_completion_times.clear();
        self.task_activities.clear();
        self.deferred_tool_uses.clear();
        self.in_progress_tool_use_ids.clear();
        self.tool_use_summaries.clear();
        self.compact_suppressed = false;
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
            // Exclude queued placeholders — same rationale as
            // `recompute_token_estimate`.
            let unqueued: Vec<crate::types::ChatMessage> = self
                .messages
                .iter()
                .filter(|m| !m.queued)
                .cloned()
                .collect();
            self.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&unqueued);
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

    pub fn tool_needs_approval(&self, tool: &ToolCall) -> bool {
        // Fast path: when running inside a landlock sandbox, permission
        // prompts add friction without security value — auto-approve
        // unless the user has explicitly opted out via config.
        if crate::sandbox::is_sandbox_active() {
            let auto_allow = crate::config::load_arc()
                .permission_automation
                .as_ref()
                .map(|pa| pa.auto_allow_if_sandboxed)
                .unwrap_or(true);
            if auto_allow {
                tracing::debug!(
                    target: "jfc::app",
                    tool_kind = tool.kind.label(),
                    result = false,
                    reason = "sandbox_active",
                    "tool_needs_approval"
                );
                return false;
            }
        }

        // Permission mode takes priority
        match self.permission_mode.auto_approves(tool) {
            PermissionDecision::Approved => return false,
            // Denied tools don't need a *prompt* — but they must not be
            // dispatched either. The StreamTool handler checks
            // `tool_denied_by_mode` before routing and short-circuits
            // denied tools into a Failed transcript entry.
            PermissionDecision::Denied(_) => return false,
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

    /// Cancel a running background task by ID. Marks it as cancelled
    /// and signals the underlying cancellation token if available.
    pub fn cancel_background_task(&mut self, task_id: &str) {
        use crate::types::TaskLifecycle;
        if let Some(bt) = self.background_tasks.get_mut(task_id) {
            bt.status = TaskLifecycle::Cancelled;
        }
    }

    /// Scan the task store for newly-completed tasks and record their
    /// completion instant so the footer can fade them out after 30 s.
    pub fn sync_task_completions(&mut self) {
        use jfc_session::TaskStatus;
        for task in self.task_store.list(jfc_session::DeletedFilter::Exclude) {
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
                .is_some_and(|t| t.status == TaskStatus::Completed)
        });
    }
}

/// Resolve the watchdog idle deadline for the current stream phase.
///
/// Two tiers, selected by `thinking_live` (an extended-thinking block is open):
///   * **thinking** → lenient (`STREAM_WATCHDOG_THINKING_TIMEOUT_SECS`, env
///     `JFC_STREAM_WATCHDOG_THINKING_TIMEOUT_SECS`). The server can reason in
///     silence for a long time; cancelling there discards the costly thinking.
///   * **responding / non-thinking model** → aggressive
///     (`STREAM_WATCHDOG_TIMEOUT_SECS`, env `JFC_STREAM_WATCHDOG_TIMEOUT_SECS`).
///     A silent wire here is almost always a dead socket — reap it fast.
///
/// `JFC_DISABLE_STREAM_WATCHDOG=1` returns `None` (watchdog off). An explicit
/// `JFC_STREAM_WATCHDOG_TIMEOUT_SECS` override is treated as a floor for the
/// thinking tier too, so a user who tightens the base never accidentally makes
/// the thinking tier *looser* than the base they asked for.
fn stream_watchdog_timeout_secs(thinking_live: bool) -> Option<u64> {
    if std::env::var("JFC_DISABLE_STREAM_WATCHDOG")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        return None;
    }
    let env_secs = |key: &str| -> Option<u64> {
        std::env::var(key)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|&secs| secs != 0)
    };
    let base = env_secs("JFC_STREAM_WATCHDOG_TIMEOUT_SECS").unwrap_or(STREAM_WATCHDOG_TIMEOUT_SECS);
    if !thinking_live {
        return Some(base);
    }
    let thinking = env_secs("JFC_STREAM_WATCHDOG_THINKING_TIMEOUT_SECS")
        .unwrap_or(STREAM_WATCHDOG_THINKING_TIMEOUT_SECS);
    // The thinking tier must never be tighter than the responding base.
    Some(thinking.max(base))
}

/// Whether the watchdog should re-drive a hard-idle stream in place instead of
/// tearing the turn down. On by default (a stall is a transient, not a logical
/// error); set `JFC_DISABLE_STREAM_WATCHDOG_RETRY` to fall back to the old
/// kill-the-turn behavior.
fn watchdog_retry_enabled() -> bool {
    !std::env::var("JFC_DISABLE_STREAM_WATCHDOG_RETRY")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::time::Duration;
    use tokio::sync::mpsc;

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
    impl jfc_provider::seal::Sealed for TestProvider {}

    /// A streaming state whose last event was `idle_secs` ago. To stay
    /// independent of the default idle threshold (`STREAM_WATCHDOG_TIMEOUT_SECS`,
    /// currently 180s), tests that want a *tripped* watchdog set
    /// `JFC_STREAM_WATCHDOG_TIMEOUT_SECS` to a small value and pass an
    /// `idle_secs` above it.
    fn idle_streaming_state(idle_secs: u64) -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.messages.push(ChatMessage::user("prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.streaming_assistant_idx = Some(1);
        state.is_streaming = true;
        let stale = Instant::now() - Duration::from_secs(idle_secs);
        state.last_stream_event_at = Some(stale);
        state.streaming_started_at = Some(stale);
        state.turn_started_at = Some(stale);
        state
    }

    // A live-but-slow stream (last event recent) must NOT trip the watchdog —
    // the idle clock is silence-since-last-event, so a stream emitting anything
    // keeps itself alive.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_leaves_recently_active_stream_alone_normal() {
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
        }
        let mut state = idle_streaming_state(2);
        let (tx, _rx) = mpsc::channel(8);
        state.check_stream_watchdog(&tx);
        assert!(state.is_streaming, "recent activity must keep streaming");
        assert_eq!(state.network_recovery_attempts, 0, "no retry recorded");
    }

    // The default idle threshold must be a coarse backstop: a stream that went
    // quiet for ~2 minutes (longer than the OLD 90s default, well within the
    // new 180s default) must NOT be cancelled. This is the direct regression
    // guard for "writing a big file and it cancels after ~1m30s" — the watchdog
    // window now sits above normal inter-event silence.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_default_window_tolerates_two_minute_quiet_regression() {
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
        }
        let mut state = idle_streaming_state(120);
        let (tx, _rx) = mpsc::channel(8);
        state.check_stream_watchdog(&tx);
        assert!(
            state.is_streaming,
            "120s quiet must be under the 180s default backstop — not cancelled"
        );
        assert_eq!(state.network_recovery_attempts, 0, "no retry recorded");
    }

    // Phase-aware tier — THINKING: a live extended-thinking block that has been
    // byte-quiet for 200s must NOT be cancelled under the lenient thinking tier
    // (default 600s), even though the same silence would trip the aggressive
    // base tier (180s). Cancelling mid-thinking discards the costly reasoning.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_thinking_phase_uses_lenient_tier_robust() {
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
            std::env::remove_var("JFC_STREAM_WATCHDOG_THINKING_TIMEOUT_SECS");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
        }
        let mut state = idle_streaming_state(200);
        // A thinking block is open (started, not yet concluded).
        state.thinking_started_at = Some(Instant::now() - Duration::from_secs(200));
        state.thinking_ended_at = None;
        let (tx, _rx) = mpsc::channel(8);
        state.check_stream_watchdog(&tx);
        assert!(
            state.is_streaming,
            "200s quiet while thinking is under the lenient 600s tier — not cancelled"
        );
        assert_eq!(state.network_recovery_attempts, 0, "no retry recorded");
    }

    // Phase-aware tier — RESPONDING: the same 200s silence, but with no live
    // thinking block (a responding or non-thinking model), DOES trip — the
    // aggressive base tier governs and a silent wire is treated as dead.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_responding_phase_uses_aggressive_tier_robust() {
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
            std::env::remove_var("JFC_STREAM_WATCHDOG_THINKING_TIMEOUT_SECS");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
        }
        let mut state = idle_streaming_state(200);
        // No thinking block open → aggressive 180s base tier applies.
        state.thinking_started_at = None;
        state.thinking_ended_at = None;
        let (tx, _rx) = mpsc::channel(8);
        state.check_stream_watchdog(&tx);
        assert!(
            state.is_streaming,
            "auto-retry re-drives the turn (still streaming)"
        );
        assert_eq!(
            state.network_recovery_attempts, 1,
            "200s silence past the 180s base tier must trip the watchdog"
        );
    }

    // A keepalive/any decoded event resets the idle clock: a state that is
    // 120s stale but receives `record_stream_activity()` (what a Keepalive
    // dispatch does) is no longer idle and must not trip even a tight window.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_keepalive_resets_idle_clock_robust() {
        unsafe {
            std::env::set_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS", "60");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
        }
        let mut state = idle_streaming_state(120);
        // Simulate the Keepalive dispatch path resetting wire liveness.
        state.record_stream_activity();
        let (tx, _rx) = mpsc::channel(8);
        state.check_stream_watchdog(&tx);
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
        }
        assert!(
            state.is_streaming,
            "a keepalive reset the clock; stream must stay alive even at a 60s window"
        );
        assert_eq!(state.network_recovery_attempts, 0, "no retry recorded");
    }

    // The core ask: a hard-idle stream auto-retries in place instead of dying.
    // After the watchdog fires, the turn is still streaming (a fresh stream was
    // re-driven), the recovery counter incremented, and the cancel token is
    // fresh (not cancelled) so the new stream isn't poisoned.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_auto_retries_idle_stream_in_place_robust() {
        unsafe {
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            // Pin a tight window so the 120s-idle fixture trips regardless of
            // the (coarser) production default.
            std::env::set_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS", "60");
        }
        let mut state = idle_streaming_state(120);
        let (tx, _rx) = mpsc::channel(8);

        state.check_stream_watchdog(&tx);
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
        }

        assert!(state.is_streaming, "turn re-driven, still streaming");
        assert_eq!(state.streaming_assistant_idx, Some(1));
        assert_eq!(
            state.network_recovery_attempts, 1,
            "one recovery attempt recorded"
        );
        assert!(
            state.network_recovery_status.is_some(),
            "recovery banner armed"
        );
        assert!(
            !state.cancel_token.is_cancelled(),
            "fresh token for the re-driven stream"
        );
        assert_eq!(state.messages.len(), 2, "no hard-error message appended");
    }

    // Bound: once `network_recovery_attempts` reaches the cap, the watchdog
    // gives up — tears the turn down and surfaces a hard error so the user can
    // Ctrl+R rather than watching a frozen spinner forever.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_gives_up_after_max_attempts_robust() {
        unsafe {
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            std::env::set_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS", "60");
        }
        let mut state = idle_streaming_state(120);
        state.messages[1] = ChatMessage::assistant("partial output".into());
        state.network_recovery_attempts = crate::app::MAX_NETWORK_RECOVERY_ATTEMPTS;
        let (tx, _rx) = mpsc::channel(8);

        state.check_stream_watchdog(&tx);
        unsafe {
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
        }

        assert!(!state.is_streaming, "exhausted: turn torn down");
        assert_eq!(
            state.network_recovery_attempts, 0,
            "counter reset on give-up"
        );
        assert!(
            !state.cancel_token.is_cancelled(),
            "fresh token for the next turn"
        );
        let last = state.messages.last().expect("error message appended");
        let text: String = last
            .parts
            .iter()
            .map(|p| match p {
                MessagePart::Text(t) => t.as_str(),
                _ => "",
            })
            .collect();
        assert!(text.contains("**Error:**"), "hard error surfaced: {text}");
        assert!(!state.toasts.is_empty(), "error toast surfaced");
    }

    // Opt-out: with the retry disabled, a hard-idle stream tears down on the
    // first timeout (the original behavior), surfacing a hard error.
    #[tokio::test]
    #[serial_test::serial]
    async fn watchdog_retry_opt_out_tears_down_robust() {
        unsafe {
            std::env::set_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY", "1");
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG");
            std::env::set_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS", "60");
        }
        let mut state = idle_streaming_state(120);
        state.messages[1] = ChatMessage::assistant("partial output".into());
        let (tx, _rx) = mpsc::channel(8);

        state.check_stream_watchdog(&tx);

        unsafe {
            std::env::remove_var("JFC_DISABLE_STREAM_WATCHDOG_RETRY");
            std::env::remove_var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS");
        }
        assert!(
            !state.is_streaming,
            "opt-out: turn torn down on first timeout"
        );
        assert_eq!(state.network_recovery_attempts, 0, "no retry recorded");
        let last = state.messages.last().expect("error message appended");
        let text: String = last
            .parts
            .iter()
            .map(|p| match p {
                MessagePart::Text(t) => t.as_str(),
                _ => "",
            })
            .collect();
        assert!(text.contains("**Error:**"), "hard error surfaced: {text}");
    }
}

#[cfg(test)]
mod background_task_cap_tests {
    use super::BackgroundTask;

    fn bg() -> BackgroundTask {
        BackgroundTask {
            task_id: "t".into(),
            description: "d".into(),
            status: crate::types::TaskLifecycle::Running,
            started_at: std::time::Instant::now(),
            completed_at: None,
            summary: None,
            error: None,
            last_tool: None,
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            model_used: None,
            agent_messages: Vec::new(),
            max_input_tokens: None,
            budget_killed: false,
            parent_task_id: None,
            workflow_progress: None,
            last_activity_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn total_tokens_sums_all_buckets_normal() {
        let mut bt = bg();
        bt.latest_input_tokens = 100;
        bt.latest_cache_read_tokens = 20;
        bt.latest_cache_write_tokens = 5;
        bt.cumulative_output_tokens = 50;
        assert_eq!(bt.total_tokens(), 175);
    }

    #[test]
    fn total_tokens_saturates_instead_of_overflowing_robust() {
        let mut bt = bg();
        bt.latest_input_tokens = u64::MAX;
        bt.cumulative_output_tokens = 1;
        // Saturating add — never panics, clamps at u64::MAX.
        assert_eq!(bt.total_tokens(), u64::MAX);
    }

    #[test]
    fn push_log_and_chat_stay_capped_robust() {
        let mut bt = bg();
        for i in 0..(BackgroundTask::LOG_CAP * 3) {
            bt.push_log(format!("entry {i}\n"));
            bt.push_chat(crate::types::ChatMessage::assistant(format!("msg {i}")));
        }
        assert_eq!(bt.messages.len(), BackgroundTask::LOG_CAP);
        assert_eq!(bt.chat_messages.len(), BackgroundTask::LOG_CAP);
        // Newest entries are the ones retained.
        let last = BackgroundTask::LOG_CAP * 3 - 1;
        assert_eq!(bt.messages.last().unwrap(), &format!("entry {last}\n"));
    }

    #[test]
    fn append_chunk_coalesces_paragraph_normal() {
        let mut bt = bg();
        bt.append_chunk("hello ".into());
        bt.append_chunk("world\n".into());
        // Coalesced: one log entry, one chat message.
        assert_eq!(bt.messages.len(), 1);
        assert_eq!(bt.messages[0], "hello world\n");
        assert_eq!(bt.chat_messages.len(), 1);
        // Newline-terminated previous entry starts a fresh one.
        bt.append_chunk("next".into());
        assert_eq!(bt.messages.len(), 2);
        assert_eq!(bt.chat_messages.len(), 2);
    }
}
