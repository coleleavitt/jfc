use crossterm::event::Event;

use crate::provider::{ModelInfo, ProviderId, StopReason};
use crate::tools::ExecutionResult;
use crate::types::{ChatMessage, ToolCall};

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
        tool_id: crate::ids::ToolId,
        result: ExecutionResult,
    },
    /// Incremental output from a running tool (e.g. bash stdout line-by-line).
    /// The UI appends this to the tool's live output preview.
    ToolOutputChunk {
        tool_id: crate::ids::ToolId,
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
        task_id: crate::ids::TaskId,
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
    /// Background snapshot of the active Anthropic OAuth account. Cached on
    /// `App.anthropic_account_snapshot` and consumed by the ribbon to show
    /// utilization (5h / 7d) plus the active claim type.
    AnthropicSnapshotUpdated {
        snapshot: Option<crate::providers::anthropic_accounts::AccountSnapshot>,
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
        task_id: crate::ids::TaskId,
        description: String,
        model_used: Option<String>,
        max_input_tokens: Option<u64>,
    },
    TaskProgress {
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
    TaskCompleted {
        task_id: crate::ids::TaskId,
        summary: String,
        elapsed_ms: u64,
    },
    TaskFailed {
        task_id: crate::ids::TaskId,
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
        /// Abort handle returned by `swarm::runner::start_teammate`. The
        /// event handler must move this into
        /// `app.team_context.teammates[agent_id].abort_tx` so the channel
        /// stays open for the teammate's lifetime. Dropping it closes the
        /// channel and the runner's abort_rx.changed() resolves Err on the
        /// next poll — which the runner treats as Cancelled, lighting up
        /// every teammate as "Done" before doing any work.
        abort_tx: Option<tokio::sync::watch::Sender<bool>>,
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
